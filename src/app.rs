mod browse;
mod controller;
mod discover;
mod library;
mod open_url;
mod playback;
mod session_state;
mod shell;
mod text_input;
mod transfer_state;
mod ui;

use crate::assets::EmbeddedAssetSource;
use anyhow::Result;
use gpui::prelude::*;
use gpui::{
    App, Application, AsyncApp, Bounds, Context, Entity, Pixels, Styled, Subscription,
    TitlebarOptions, WeakEntity, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowOptions, div, px, rgb, size, svg,
};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::audio::PlaybackController;
use crate::keybindings;
use crate::library::Library;
use crate::platform;
use crate::provider::{
    CollectionKind, CollectionRef, ProviderId, ProviderRegistry, SharedProvider, TrackList,
    TrackSummary,
};
use crate::theme;
use crate::transfer::TransferManager;
use crate::url_media::initialize_media_url_resolver;
use serde::{Deserialize, Serialize};
use souvlaki::MediaControlEvent;

use self::discover::DiscoverModule;
use self::library::LibraryModule;
use self::playback::{PlaybackIntent, PlaybackModule, PlaybackRuntimeEvent};
use self::session_state::restored_session_state;
use self::text_input::TextInputState;
use self::transfer_state::ActiveTransfer;
use self::transfer_state::TransferStateModel;
use self::ui::{ContextMenuTarget, NotificationCenter, UiState};

const PREVIOUS_RESTART_THRESHOLD: Duration = Duration::from_secs(5);
const UI_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const PLAYER_CENTER_WIDTH: f32 = 420.0;
const WINDOW_MIN_WIDTH: f32 = 960.0;
const WINDOW_MIN_HEIGHT: f32 = 680.0;
const WINDOW_WIDTH_RATIO: f32 = 0.9;
const WINDOW_HEIGHT_RATIO: f32 = 0.88;
const STARTUP_MEDIA_SESSION_PUBLISH_DELAYS: [Duration; 3] = [
    Duration::from_millis(150),
    Duration::from_millis(500),
    Duration::from_millis(1200),
];

struct StartupWindowDimensions {
    size: gpui::Size<Pixels>,
    min_size: gpui::Size<Pixels>,
}

fn startup_window_dimensions(cx: &App) -> StartupWindowDimensions {
    let fallback_size = size(px(theme::WINDOW_WIDTH), px(theme::WINDOW_HEIGHT));

    let Some(display) = cx.primary_display() else {
        return StartupWindowDimensions {
            size: fallback_size,
            min_size: size(px(WINDOW_MIN_WIDTH), px(WINDOW_MIN_HEIGHT)),
        };
    };

    let display_size = display.bounds().size;
    let width = scaled_window_dimension(
        theme::WINDOW_WIDTH,
        WINDOW_MIN_WIDTH,
        display_size.width,
        WINDOW_WIDTH_RATIO,
    );
    let height = scaled_window_dimension(
        theme::WINDOW_HEIGHT,
        WINDOW_MIN_HEIGHT,
        display_size.height,
        WINDOW_HEIGHT_RATIO,
    );

    StartupWindowDimensions {
        size: size(px(width), px(height)),
        min_size: size(
            px(WINDOW_MIN_WIDTH.min(width)),
            px(WINDOW_MIN_HEIGHT.min(height)),
        ),
    }
}

fn scaled_window_dimension(
    preferred: f32,
    minimum: f32,
    display_dimension: Pixels,
    ratio: f32,
) -> f32 {
    let max_dimension = ((display_dimension.to_f64() as f32) * ratio)
        .floor()
        .max(1.0);
    let min_dimension = minimum.min(max_dimension);
    preferred.min(max_dimension).max(min_dimension)
}

fn main_window_titlebar() -> Option<TitlebarOptions> {
    #[cfg(target_os = "macos")]
    {
        Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(12.0), px(12.0))),
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(unix)]
fn install_signal_shutdown_handlers() -> Option<Arc<Mutex<Receiver<()>>>> {
    use signal_hook::consts::signal::{SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) else {
        return None;
    };

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            let _ = tx.send(());
        }
    });

    Some(Arc::new(Mutex::new(rx)))
}

#[cfg(not(unix))]
fn install_signal_shutdown_handlers() -> Option<Arc<Mutex<Receiver<()>>>> {
    None
}

pub fn run() -> Result<()> {
    let shutdown_rx = install_signal_shutdown_handlers();
    let app = Application::new().with_assets(EmbeddedAssetSource::new());
    app.run(move |cx: &mut App| {
        platform::setup_app(cx);
        keybindings::bind(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if let Some(shutdown_rx) = shutdown_rx.clone() {
            let background = cx.background_executor().clone();
            cx.spawn(move |cx: &mut AsyncApp| {
                let background = background.clone();
                let async_cx = cx.clone();
                async move {
                    let receiver = shutdown_rx.clone();
                    let shutdown = background
                        .spawn(async move { receiver.lock().ok()?.recv().ok() })
                        .await;

                    if shutdown.is_some() {
                        let _ = async_cx.update(|app| {
                            app.quit();
                        });
                    }
                }
            })
            .detach();
        }
        let startup_window = startup_window_dimensions(cx);
        let bounds = Bounds::centered(None, startup_window.size, cx);

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(startup_window.min_size),
                    titlebar: main_window_titlebar(),
                    window_background: WindowBackgroundAppearance::Opaque,
                    window_decorations: Some(WindowDecorations::Client),
                    ..Default::default()
                },
                {
                    move |window, cx| {
                        let media_controls_hwnd = platform::media_controls_hwnd(window);
                        cx.new(move |cx| OryxApp::new(media_controls_hwnd, None, cx))
                    }
                },
            )
            .expect("opening Oryx window failed");

        let _ = window.update(cx, |root, window, cx| {
            platform::configure_window(window);
            window.activate_window();
            window.focus(&root.shell_focus_handle);
            cx.activate(true);
        });
    });

    Ok(())
}

struct OryxApp {
    shell_focus_handle: gpui::FocusHandle,
    query_focus_handle: gpui::FocusHandle,
    open_url_focus_handle: gpui::FocusHandle,
    provider_auth_username_focus_handle: gpui::FocusHandle,
    provider_auth_password_focus_handle: gpui::FocusHandle,
    provider_link_focus_handle: gpui::FocusHandle,
    import_review_input_focus_handles: HashMap<text_input::TextInputId, gpui::FocusHandle>,
    providers: Vec<SharedProvider>,
    library: Library,
    library_catalog: Entity<LibraryModule>,
    discover: Entity<DiscoverModule>,
    playback_state: Entity<PlaybackModule>,
    transfer_state: Entity<TransferStateModel>,
    notifications: Entity<NotificationCenter>,
    ui_state: Entity<UiState>,
    transfer: TransferManager,
    query_input: TextInputState,
    open_url_input: TextInputState,
    provider_auth_username_input: TextInputState,
    provider_auth_password_input: TextInputState,
    provider_link_input: TextInputState,
    import_review_inputs: HashMap<text_input::TextInputId, TextInputState>,
    browse_mode: BrowseMode,
    visible_local_track_list_override: Option<(BrowseMode, TrackList)>,
    status_message: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl OryxApp {
    fn new(
        media_controls_hwnd: Option<*mut c_void>,
        shutdown_rx: Option<Arc<Mutex<Receiver<()>>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        initialize_media_url_resolver();
        let library = Library::new().expect("oryx library should initialize");
        let registry = ProviderRegistry::with_defaults(Some(&library));
        let providers = registry.all().to_vec();
        let library_catalog = cx.new(|_cx| LibraryModule::new(library.clone()));
        let (playback, media_event_rx) = PlaybackController::new(media_controls_hwnd);
        let (transfer, transfer_rx) = TransferManager::new();
        let media_event_rx = Arc::new(Mutex::new(media_event_rx));
        let transfer_rx = Arc::new(Mutex::new(transfer_rx));
        let snapshot = library.load_session_snapshot().ok().flatten();
        let restored = restored_session_state(&library, snapshot);
        library_catalog.update(cx, |catalog, _cx| {
            if let Some(id) = restored.selected_local_album_id.clone() {
                catalog.select_local_collection(BrowseMode::Albums, id);
            }
            if let Some(id) = restored.selected_local_artist_id.clone() {
                catalog.select_local_collection(BrowseMode::Artists, id);
            }
            if let Some(id) = restored.selected_local_playlist_id.clone() {
                catalog.select_local_collection(BrowseMode::Playlists, id);
            }
        });

        for provider in &providers {
            if let Ok(Some(serialized)) = library.load_provider_auth(provider.id()) {
                let _ = provider.restore_credentials(&serialized);
            }
        }
        let enabled_search_providers = default_enabled_search_providers(&providers);
        let discover = cx.new(|_cx| {
            DiscoverModule::new(
                enabled_search_providers.clone(),
                restored.search_results.clone(),
                restored.selected_collection_id.clone(),
                restored.track_list.clone(),
            )
        });
        let playback_state = cx.new(|_cx| {
            PlaybackModule::new(
                playback.clone(),
                restored.playback_context.clone(),
                restored.current_track_index,
                restored.now_playing.clone(),
                restored.resume_position,
                restored.playback_status.clone(),
                restored.repeat_mode.clone(),
                restored.shuffle_enabled,
                restored.shuffle_seed,
            )
        });
        let transfer_state = cx.new(|_cx| TransferStateModel::new());
        let notifications = cx.new(|_cx| NotificationCenter::new());
        let ui_state = cx.new(|_cx| UiState::new());
        let transfer_subscription = cx.subscribe(
            &transfer_state,
            |this: &mut Self, _, event: &crate::transfer::TransferEvent, cx| {
                this.handle_transfer_event(event.clone(), cx);
            },
        );
        let playback_subscription = cx.subscribe(
            &playback_state,
            |this: &mut Self, _, intent: &PlaybackIntent, cx| {
                this.dispatch_playback_intent(*intent, cx);
            },
        );
        let playback_runtime_subscription = cx.subscribe(
            &playback_state,
            |this: &mut Self, _, event: &PlaybackRuntimeEvent, cx| {
                this.handle_playback_runtime_event(event.clone(), cx);
            },
        );

        let mut app = Self {
            shell_focus_handle: cx.focus_handle().tab_stop(true),
            query_focus_handle: cx.focus_handle().tab_stop(true),
            open_url_focus_handle: cx.focus_handle().tab_stop(true),
            provider_auth_username_focus_handle: cx.focus_handle().tab_stop(true),
            provider_auth_password_focus_handle: cx.focus_handle().tab_stop(true),
            provider_link_focus_handle: cx.focus_handle().tab_stop(true),
            import_review_input_focus_handles: HashMap::new(),
            providers,
            library,
            query_input: TextInputState::new(restored.query, restored.query_cursor),
            open_url_input: TextInputState::new(String::new(), 0),
            provider_auth_username_input: TextInputState::new(String::new(), 0),
            provider_auth_password_input: TextInputState::new(String::new(), 0),
            provider_link_input: TextInputState::new(String::new(), 0),
            import_review_inputs: HashMap::new(),
            browse_mode: restored.browse_mode,
            visible_local_track_list_override: None,
            library_catalog,
            discover,
            playback_state: playback_state.clone(),
            transfer_state: transfer_state.clone(),
            notifications,
            ui_state,
            transfer,
            status_message: Some(restored.status_message),
            _subscriptions: vec![
                transfer_subscription,
                playback_subscription,
                playback_runtime_subscription,
            ],
        };

        Self::spawn_media_control_listener(media_event_rx, playback_state, cx);
        Self::spawn_transfer_listener(transfer_rx, transfer_state, cx);
        app.restore_external_downloads(restored.external_downloads, cx);
        Self::spawn_playback_refresh(cx);
        Self::spawn_startup_audio_prewarm(playback, cx);
        if let Some(shutdown_rx) = shutdown_rx {
            Self::spawn_shutdown_listener(shutdown_rx, cx);
        }
        Self::spawn_startup_media_session_publish(cx);
        app.install_quit_persistence(cx);
        cx.notify();
        app
    }

    fn searchable_provider_ids(&self) -> Vec<ProviderId> {
        self.providers
            .iter()
            .map(|provider| provider.id())
            .filter(|provider_id| *provider_id != ProviderId::Local)
            .collect()
    }

    fn provider_for_id(&self, provider_id: ProviderId) -> Option<SharedProvider> {
        self.providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .cloned()
    }

    fn toggle_provider_from_menu(
        &mut self,
        provider_id: ProviderId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.provider_for_id(provider_id) else {
            self.status_message = Some(format!("Provider '{}' is not available.", provider_id));
            self.discover.update(cx, |discover, _cx| {
                discover.close_source_picker();
            });
            cx.notify();
            return;
        };

        let is_enabled = self.discover.read(cx).is_provider_enabled(provider_id);
        if is_enabled {
            self.discover.update(cx, |discover, _cx| {
                discover.disable_provider(provider_id);
            });
            self.reset_discover_scope(
                format!("Disabled {} for search.", provider.display_name()),
                cx,
            );
            return;
        }

        if provider.requires_credentials() && !provider.has_stored_credentials() {
            self.open_provider_auth_prompt(provider_id, window, cx);
            return;
        }

        self.discover.update(cx, |discover, _cx| {
            discover.enable_provider(provider_id);
        });
        self.reset_discover_scope(
            format!("Enabled {} for search.", provider.display_name()),
            cx,
        );
    }

    fn reset_discover_scope(&mut self, message: String, cx: &mut Context<Self>) {
        self.discover.update(cx, |discover, _cx| {
            discover.reset_scope();
        });
        self.status_message = Some(message);
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    fn toggle_source_picker(&mut self, cx: &mut Context<Self>) {
        if self.ui_state.read(cx).provider_auth_prompt().is_some()
            || self.ui_state.read(cx).provider_link_prompt().is_some()
            || self.ui_state.read(cx).open_url_prompt_open()
        {
            return;
        }
        self.update_ui_state(cx, |state| {
            state.close_app_menu();
        });
        self.discover.update(cx, |discover, _cx| {
            discover.toggle_source_picker();
        });
        cx.notify();
    }

    fn toggle_downloads_modal(&mut self, cx: &mut Context<Self>) {
        if self.ui_state.read(cx).provider_auth_prompt().is_some()
            || self.ui_state.read(cx).provider_link_prompt().is_some()
            || self.ui_state.read(cx).open_url_prompt_open()
        {
            return;
        }
        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        self.update_ui_state(cx, |state| {
            state.toggle_downloads_modal();
        });
        cx.notify();
    }

    fn close_downloads_modal(&mut self, cx: &mut Context<Self>) {
        if !self.ui_state.read(cx).downloads_modal_open() {
            return;
        }
        self.update_ui_state(cx, |state| {
            state.close_downloads_modal();
        });
        cx.notify();
    }

    fn set_browse_mode(&mut self, mode: BrowseMode, cx: &mut Context<Self>) {
        self.browse_mode = mode;
        self.clear_visible_local_track_list_override();
        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        let (album_count, artist_count, playlist_count) = {
            let catalog = self.library_catalog.read(cx);
            (
                catalog.album_count(),
                catalog.artist_count(),
                catalog.playlist_count(),
            )
        };
        self.status_message = Some(match self.browse_mode {
            BrowseMode::Discover => {
                format!(
                    "Press Enter to search {}.",
                    self.discover.read(cx).active_source_label(&self.providers)
                )
            }
            BrowseMode::Albums => format!("{album_count} local album(s)."),
            BrowseMode::Artists => format!("{artist_count} local artist(s)."),
            BrowseMode::Playlists => format!("{playlist_count} playlist(s)."),
        });
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    fn active_download(&self, track: &TrackSummary, cx: &App) -> Option<ActiveTransfer> {
        self.transfer_state.read(cx).active_download(track)
    }

    fn current_visible_track_list(&self, cx: &App) -> Option<TrackList> {
        match self.browse_mode {
            BrowseMode::Discover => self.discover.read(cx).track_list(),
            BrowseMode::Albums | BrowseMode::Artists | BrowseMode::Playlists => self
                .visible_local_track_list_override
                .as_ref()
                .filter(|(mode, _track_list)| *mode == self.browse_mode)
                .map(|(_mode, track_list)| track_list.clone())
                .or_else(|| {
                    self.library_catalog
                        .read(cx)
                        .current_local_track_list_owned(self.browse_mode)
                }),
        }
    }

    fn current_visible_track_list_cloned(&self, cx: &App) -> Option<TrackList> {
        self.current_visible_track_list(cx)
    }

    fn clear_visible_local_track_list_override(&mut self) {
        self.visible_local_track_list_override = None;
    }

    fn set_visible_local_track_list_override(&mut self, mode: BrowseMode, track_list: TrackList) {
        self.visible_local_track_list_override = Some((mode, track_list));
    }

    fn install_quit_persistence(&self, cx: &mut Context<Self>) {
        let _ = cx.on_app_quit(|this, cx| {
            this.persist_current_playback_position(cx);
            async {}
        });
    }

    fn quit_app(&mut self, cx: &mut Context<Self>) {
        self.persist_current_playback_position(cx);
        cx.quit();
    }

    fn toggle_app_menu(&mut self, cx: &mut Context<Self>) {
        if self.ui_state.read(cx).provider_auth_prompt().is_some()
            || self.ui_state.read(cx).provider_link_prompt().is_some()
            || self.ui_state.read(cx).open_url_prompt_open()
            || self.ui_state.read(cx).import_review_loading()
            || self.ui_state.read(cx).pending_import_review().is_some()
        {
            return;
        }

        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        self.update_ui_state(cx, |state| {
            state.close_downloads_modal();
            state.close_context_menu();
            state.toggle_app_menu();
        });
        cx.notify();
    }

    fn close_app_menu(&mut self, cx: &mut Context<Self>) {
        let closed = self
            .ui_state
            .update(cx, |state, _cx| state.close_app_menu());
        if closed {
            cx.notify();
        }
    }

    fn spawn_media_control_listener(
        receiver: Arc<Mutex<std::sync::mpsc::Receiver<MediaControlEvent>>>,
        playback_state: Entity<PlaybackModule>,
        cx: &mut Context<Self>,
    ) {
        let background = cx.background_executor().clone();
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let background = background.clone();
            let mut async_cx = cx.clone();
            let playback_state = playback_state.clone();
            async move {
                loop {
                    let receiver = receiver.clone();
                    let event = background
                        .spawn(async move { receiver.lock().ok()?.recv().ok() })
                        .await;

                    let Some(event) = event else {
                        break;
                    };

                    if playback_state
                        .update(&mut async_cx, |playback, cx| {
                            playback.handle_media_control_event(event, cx);
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn spawn_playback_refresh(cx: &mut Context<Self>) {
        let background = cx.background_executor().clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let background = background.clone();
            let mut async_cx = cx.clone();
            async move {
                loop {
                    background
                        .spawn(async move {
                            std::thread::sleep(UI_REFRESH_INTERVAL);
                        })
                        .await;

                    if this
                        .update(&mut async_cx, |this, cx| {
                            let has_active_downloads =
                                this.transfer_state.read(cx).has_active_downloads();
                            this.playback_state.update(cx, |playback, cx| {
                                playback.handle_refresh_tick(has_active_downloads, cx);
                            });
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn spawn_startup_media_session_publish(cx: &mut Context<Self>) {
        let background = cx.background_executor().clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let background = background.clone();
            let mut async_cx = cx.clone();
            async move {
                for delay in STARTUP_MEDIA_SESSION_PUBLISH_DELAYS {
                    background
                        .spawn(async move {
                            std::thread::sleep(delay);
                        })
                        .await;

                    if this
                        .update(&mut async_cx, |this, cx| {
                            this.playback_state
                                .read(cx)
                                .publish_restored_media_session();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn spawn_startup_audio_prewarm(playback: PlaybackController, cx: &mut Context<Self>) {
        let background = cx.background_executor().clone();
        cx.spawn(move |_this: WeakEntity<Self>, _cx: &mut AsyncApp| {
            let background = background.clone();
            async move {
                let result = background.spawn(async move { playback.warm() }).await;
                if let Err(error) = result {
                    eprintln!("failed to prewarm audio output: {error}");
                }
            }
        })
        .detach();
    }

    fn spawn_shutdown_listener(receiver: Arc<Mutex<Receiver<()>>>, cx: &mut Context<Self>) {
        let background = cx.background_executor().clone();
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let background = background.clone();
            let mut async_cx = cx.clone();
            async move {
                let receiver = receiver.clone();
                let shutdown = background
                    .spawn(async move { receiver.lock().ok()?.recv().ok() })
                    .await;

                if shutdown.is_none() {
                    return;
                }

                let _ = this.update(&mut async_cx, |this, cx| {
                    this.handle_shutdown_request(cx);
                });
            }
        })
        .detach();
    }

    fn track_is_cached(&self, track: &TrackSummary, cx: &App) -> bool {
        self.library_catalog.read(cx).track_is_cached(track)
    }

    fn update_playback_state(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut PlaybackModule),
    ) {
        self.playback_state.update(cx, |state, _cx| update(state));
    }

    fn update_ui_state(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut UiState)) {
        self.ui_state.update(cx, |state, _cx| update(state));
    }

    fn open_context_menu(
        &mut self,
        position: gpui::Point<Pixels>,
        target: ContextMenuTarget,
        cx: &mut Context<Self>,
    ) {
        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        self.update_ui_state(cx, |state| {
            state.open_context_menu(position, target);
        });
        cx.notify();
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        let closed = self
            .ui_state
            .update(cx, |state, _cx| state.close_context_menu());
        if closed {
            cx.notify();
        }
    }

    fn persist_session_snapshot(&self, cx: &App) {
        session_state::persist_session_snapshot(self, cx);
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) enum AppIcon {
    Search,
    Play,
    PlayCircle,
    Pause,
    SkipBack,
    SkipForward,
    Shuffle,
    RepeatAll,
    RepeatOne,
    Music,
    Menu,
    Heart,
    HeartFilled,
    Download,
    Plus,
    Trash,
    X,
}

impl AppIcon {
    pub(super) fn asset_path(self) -> &'static str {
        match self {
            Self::Search => "icons/lucide/search.svg",
            Self::Play => "icons/lucide/play.svg",
            Self::PlayCircle => "icons/lucide/play-circle.svg",
            Self::Pause => "icons/lucide/pause.svg",
            Self::SkipBack => "icons/lucide/skip-back.svg",
            Self::SkipForward => "icons/lucide/skip-forward.svg",
            Self::Shuffle => "icons/lucide/shuffle.svg",
            Self::RepeatAll => "icons/lucide/repeat-2.svg",
            Self::RepeatOne => "icons/lucide/repeat-1.svg",
            Self::Music => "icons/lucide/music-4.svg",
            Self::Menu => "icons/lucide/menu.svg",
            Self::Heart => "icons/lucide/heart.svg",
            Self::HeartFilled => "icons/lucide/heart-filled.svg",
            Self::Download => "icons/lucide/download.svg",
            Self::Plus => "icons/lucide/plus.svg",
            Self::Trash => "icons/lucide/trash-2.svg",
            Self::X => "icons/lucide/x.svg",
        }
    }
}

pub(super) fn render_icon_with_color(icon: AppIcon, size: f32, color: u32) -> gpui::Div {
    div().w(px(size)).h(px(size)).overflow_hidden().child(
        svg()
            .path(icon.asset_path())
            .w_full()
            .h_full()
            .text_color(rgb(color)),
    )
}

fn format_clock(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn track_cache_key(track: &TrackSummary) -> String {
    format!(
        "{}:{}",
        track.reference.provider.as_str(),
        track.reference.id
    )
}

fn collection_entity_key(collection: &CollectionRef) -> String {
    format!(
        "{}:{}:{}",
        match collection.kind {
            CollectionKind::Album => "album",
            CollectionKind::Playlist => "playlist",
        },
        collection.provider.as_str(),
        collection.id
    )
}

fn collection_browser_key(collection: &CollectionRef) -> String {
    format!("{}:{}", collection.provider.as_str(), collection.id)
}

fn default_enabled_search_providers(providers: &[SharedProvider]) -> HashSet<ProviderId> {
    providers
        .iter()
        .filter(|provider| provider.id() != ProviderId::Local)
        .filter(|provider| !provider.requires_credentials() || provider.has_stored_credentials())
        .map(|provider| provider.id())
        .collect()
}

pub(super) fn format_duration(duration_seconds: Option<u32>) -> String {
    let Some(total_seconds) = duration_seconds else {
        return "--:--".to_string();
    };

    format_clock(Duration::from_secs(total_seconds as u64))
}

trait CollectionKindLabel {
    fn label(&self) -> &'static str;
}

impl CollectionKindLabel for CollectionKind {
    fn label(&self) -> &'static str {
        match self {
            CollectionKind::Album => "Album",
            CollectionKind::Playlist => "Playlist",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum BrowseMode {
    #[default]
    Discover,
    Albums,
    Artists,
    Playlists,
}

fn local_collection_selection_key(mode: BrowseMode, collection: &CollectionRef) -> String {
    match mode {
        BrowseMode::Albums | BrowseMode::Playlists => collection_entity_key(collection),
        BrowseMode::Artists => collection.id.clone(),
        BrowseMode::Discover => collection_entity_key(collection),
    }
}

fn pick_existing_or_first(
    mode: BrowseMode,
    selected: Option<String>,
    lists: &[TrackList],
) -> Option<String> {
    selected
        .filter(|selected| {
            lists.iter().any(|list| {
                list.collection.reference.id == *selected
                    || local_collection_selection_key(mode, &list.collection.reference) == *selected
            })
        })
        .and_then(|selected| {
            lists.iter().find_map(|list| {
                (list.collection.reference.id == selected
                    || local_collection_selection_key(mode, &list.collection.reference) == selected)
                    .then(|| local_collection_selection_key(mode, &list.collection.reference))
            })
        })
        .or_else(|| {
            lists
                .first()
                .map(|list| local_collection_selection_key(mode, &list.collection.reference))
        })
}

fn selected_local_track_list<'a>(
    mode: BrowseMode,
    lists: &'a [TrackList],
    selected_id: Option<&str>,
) -> Option<&'a TrackList> {
    selected_id
        .and_then(|selected_id| {
            lists.iter().find(|list| {
                list.collection.reference.id == selected_id
                    || local_collection_selection_key(mode, &list.collection.reference)
                        == selected_id
            })
        })
        .or_else(|| lists.first())
}

pub(super) fn provider_collection_ref_for_local_album(
    track_list: &TrackList,
) -> Option<CollectionRef> {
    let provider = track_list
        .tracks
        .first()
        .map(|track| track.reference.provider)
        .unwrap_or(track_list.collection.reference.provider);
    if provider == ProviderId::Local {
        return None;
    }

    let collection_id = track_list
        .tracks
        .first()
        .and_then(|track| track.collection_id.clone())
        .or_else(|| Some(track_list.collection.reference.id.clone()))?;

    Some(CollectionRef::new(
        provider,
        collection_id.clone(),
        CollectionKind::Album,
        provider.collection_url(CollectionKind::Album, &collection_id),
    ))
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
