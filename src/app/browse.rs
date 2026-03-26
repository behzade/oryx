mod discover;
mod library;
mod playback_context;
mod rows;

use std::fs;
use std::path::Path;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    App, Context, FontWeight, MouseButton, MouseDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px, relative, rgb,
};

use crate::library::{
    ImportAlbumReview, ImportMetadataField, ImportMetadataSource, ImportReview, ImportTrackReview,
};
use crate::progressive::ProgressiveSnapshot;
use crate::provider::{CollectionSummary, ProviderId, TrackList, TrackSummary};
use crate::theme;
use crate::url_media::fallback_download_name;

use self::rows::{
    action_button, audio_quality_from_track_summary, collection_quality_metadata,
    download_progress_ratio, metadata_label, render_collection_artwork,
    summarize_track_list_quality, vertical_divider,
};
use super::library::{AudioQuality, CollectionQualitySummary, normalized_audio_quality_grade};
use super::text_input::{TextInputElement, TextInputId};
use super::transfer_state::DownloadItemState;
use super::ui::{self, ContextMenuState, ContextMenuTarget};
use super::{
    AppIcon, BrowseMode, CollectionKindLabel, OryxApp, collection_entity_key, format_duration,
    local_collection_selection_key, provider_collection_ref_for_local_album,
    render_icon_with_color,
};

const TOPBAR_SIDE_SLOT_WIDTH: f32 = 96.0;
const DISCOVERY_COLUMN_MIN_WIDTH: f32 = 340.0;
const DISCOVERY_COLUMN_MAX_WIDTH: f32 = 420.0;
const DISCOVERY_COLUMN_RATIO: f32 = 0.33;
const PLAYBACK_CONTEXT_MIN_WIDTH: f32 = 300.0;
const PLAYBACK_CONTEXT_MAX_WIDTH: f32 = 360.0;
const PLAYBACK_CONTEXT_RATIO: f32 = 0.28;
const SOURCE_MENU_WIDTH: f32 = 196.0;
const SOURCE_MENU_TOP_OFFSET: f32 = 58.0;
const PLAYBACK_CONTEXT_BREAKPOINT: f32 = 1180.0;
const COMPACT_METADATA_BREAKPOINT: f32 = 1320.0;
const STREAM_PLAYABLE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct StreamButtonStyle {
    background: u32,
    border: u32,
    text: u32,
}

#[derive(Clone, Copy)]
struct DownloadActionStyle {
    icon: u32,
    border: u32,
    background: u32,
}

fn stream_button_style(
    snapshot: ProgressiveSnapshot,
    started_at: Instant,
    duration_seconds: Option<u64>,
) -> Option<StreamButtonStyle> {
    if snapshot.downloaded_bytes < STREAM_PLAYABLE_BYTES {
        return None;
    }

    let Some(total_bytes) = snapshot.total_bytes else {
        return Some(StreamButtonStyle {
            background: theme::DOWNLOAD_PROGRESS_LIGHT,
            border: theme::DOWNLOAD_PROGRESS,
            text: theme::TEXT_PRIMARY,
        });
    };
    let Some(duration_seconds) = duration_seconds.filter(|seconds| *seconds > 0) else {
        return Some(StreamButtonStyle {
            background: theme::DOWNLOAD_PROGRESS_LIGHT,
            border: theme::DOWNLOAD_PROGRESS,
            text: theme::TEXT_PRIMARY,
        });
    };

    let elapsed_seconds = started_at.elapsed().as_secs_f64().max(1.0);
    let speed_bytes_per_second = snapshot.downloaded_bytes as f64 / elapsed_seconds;
    if speed_bytes_per_second <= 0.0 {
        return Some(StreamButtonStyle {
            background: 0xFF4A2E22,
            border: 0xFFD48E7A,
            text: 0xFFF0D5CB,
        });
    }

    let remaining_bytes = total_bytes.saturating_sub(snapshot.downloaded_bytes) as f64;
    let estimated_finish_seconds = remaining_bytes / speed_bytes_per_second;
    let viability_ratio = estimated_finish_seconds / duration_seconds as f64;

    Some(if viability_ratio <= 0.9 {
        StreamButtonStyle {
            background: 0xFF243A2A,
            border: 0xFF8CCF9E,
            text: 0xFFE2F4E6,
        }
    } else if viability_ratio <= 1.2 {
        StreamButtonStyle {
            background: 0xFF45351F,
            border: 0xFFE0BE73,
            text: 0xFFF5E7BE,
        }
    } else {
        StreamButtonStyle {
            background: 0xFF4A2E22,
            border: 0xFFD48E7A,
            text: 0xFFF0D5CB,
        }
    })
}

fn download_icon_button(icon: AppIcon, style: DownloadActionStyle, disabled: bool) -> gpui::Div {
    div()
        .w(px(30.))
        .h(px(30.))
        .rounded(px(theme::RADIUS_FULL))
        .border_1()
        .border_color(rgb(style.border))
        .bg(rgb(style.background))
        .when(!disabled, |this| this.cursor_pointer())
        .when(disabled, |this| this.opacity(0.35))
        .flex()
        .items_center()
        .justify_center()
        .child(render_icon_with_color(icon, 15., style.icon))
}

fn neutral_download_action_style() -> DownloadActionStyle {
    DownloadActionStyle {
        icon: theme::TEXT_MUTED,
        border: theme::BORDER_SUBTLE,
        background: theme::SURFACE_FLOATING,
    }
}

fn destructive_download_action_style() -> DownloadActionStyle {
    DownloadActionStyle {
        icon: 0xFFF0D5CB,
        border: 0xFFD48E7A,
        background: 0xFF4A2E22,
    }
}

fn stream_download_action_style(style: StreamButtonStyle) -> DownloadActionStyle {
    DownloadActionStyle {
        icon: style.text,
        border: style.border,
        background: style.background,
    }
}

fn format_byte_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut value = bytes as f64;
    let mut unit_index = 0usize;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else if value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit_index])
    } else {
        format!("{value:.1} {}", UNITS[unit_index])
    }
}

fn download_filename(state: &DownloadItemState) -> Option<String> {
    match state {
        DownloadItemState::Queued { source_url, .. } => Some(fallback_download_name(source_url)),
        DownloadItemState::Active {
            destination,
            source_url,
            ..
        } => destination
            .as_deref()
            .and_then(download_path_filename)
            .or_else(|| source_url.as_deref().map(fallback_download_name)),
        DownloadItemState::Completed { destination, .. } => download_path_filename(destination),
        DownloadItemState::Failed {
            destination,
            source_url,
            ..
        } => destination
            .as_deref()
            .and_then(download_path_filename)
            .or_else(|| Some(fallback_download_name(source_url))),
    }
}

fn download_format_label(state: &DownloadItemState) -> Option<String> {
    match state {
        DownloadItemState::Queued { source_url, .. } => {
            download_path_extension(Path::new(source_url))
        }
        DownloadItemState::Active { destination, .. } => {
            destination.as_deref().and_then(download_path_extension)
        }
        DownloadItemState::Completed { destination, .. } => download_path_extension(destination),
        DownloadItemState::Failed { destination, .. } => {
            destination.as_deref().and_then(download_path_extension)
        }
    }
}

fn download_size_label(
    state: &DownloadItemState,
    snapshot: Option<ProgressiveSnapshot>,
) -> Option<String> {
    match state {
        DownloadItemState::Queued { .. } | DownloadItemState::Active { .. } => {
            let snapshot = snapshot?;
            match snapshot.total_bytes {
                Some(total_bytes) if snapshot.downloaded_bytes > 0 => Some(format!(
                    "{} / {}",
                    format_byte_size(snapshot.downloaded_bytes),
                    format_byte_size(total_bytes)
                )),
                Some(total_bytes) => Some(format_byte_size(total_bytes)),
                None if snapshot.downloaded_bytes > 0 => {
                    Some(format_byte_size(snapshot.downloaded_bytes))
                }
                None => None,
            }
        }
        DownloadItemState::Completed { destination, .. } => {
            download_file_len(destination).map(format_byte_size)
        }
        DownloadItemState::Failed { destination, .. } => destination
            .as_deref()
            .and_then(download_file_len)
            .map(|bytes| format!("Partial {}", format_byte_size(bytes))),
    }
}

fn download_duration_label(state: &DownloadItemState) -> Option<String> {
    match state {
        DownloadItemState::Active {
            duration_seconds, ..
        } => duration_seconds
            .and_then(|seconds| u32::try_from(seconds).ok())
            .map(Some)
            .map(format_duration),
        _ => None,
    }
}

fn download_metadata_line(
    state: &DownloadItemState,
    snapshot: Option<ProgressiveSnapshot>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(format) = download_format_label(state) {
        parts.push(format);
    }
    if let Some(size) = download_size_label(state, snapshot) {
        parts.push(size);
    }
    if let Some(duration) = download_duration_label(state) {
        parts.push(duration);
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("  •  "))
    }
}

fn download_path_filename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

fn download_path_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.trim().is_empty())
        .map(|ext| ext.to_ascii_uppercase())
}

fn download_file_len(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}

pub(super) fn section_divider() -> gpui::Div {
    rows::section_divider()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BrowseLayout {
    Wide,
    Compact,
}

impl OryxApp {
    pub(super) fn browse_layout(&self, window: &Window) -> BrowseLayout {
        let viewport_width = window.viewport_size().width.to_f64() as f32;
        if viewport_width < PLAYBACK_CONTEXT_BREAKPOINT {
            BrowseLayout::Compact
        } else {
            BrowseLayout::Wide
        }
    }

    pub(super) fn discovery_column_width(&self, window: &Window) -> f32 {
        let viewport_width = window.viewport_size().width.to_f64() as f32;
        fluid_panel_width(
            viewport_width * DISCOVERY_COLUMN_RATIO,
            DISCOVERY_COLUMN_MIN_WIDTH,
            DISCOVERY_COLUMN_MAX_WIDTH,
        )
    }

    pub(super) fn playback_context_width(&self, window: &Window) -> f32 {
        match self.browse_layout(window) {
            BrowseLayout::Wide => {
                let viewport_width = window.viewport_size().width.to_f64() as f32;
                fluid_panel_width(
                    viewport_width * PLAYBACK_CONTEXT_RATIO,
                    PLAYBACK_CONTEXT_MIN_WIDTH,
                    PLAYBACK_CONTEXT_MAX_WIDTH,
                )
            }
            BrowseLayout::Compact => 0.0,
        }
    }

    pub(super) fn source_menu_left_offset(&self, window: &Window) -> f32 {
        (self.discovery_column_width(window) - SOURCE_MENU_WIDTH - 20.0).max(theme::SPACE_3)
    }

    pub(super) fn should_show_compact_metadata(&self, window: &Window) -> bool {
        (window.viewport_size().width.to_f64() as f32) < COMPACT_METADATA_BREAKPOINT
    }

    pub(super) fn should_show_playback_context_in_layout(&self, window: &Window, cx: &App) -> bool {
        self.browse_layout(window) == BrowseLayout::Wide
            && self.should_show_playback_context_panel(cx)
    }

    fn collection_metadata(
        &self,
        collection: &CollectionSummary,
        cx: &App,
    ) -> Option<rows::RowMetadata> {
        let quality = self
            .library_catalog
            .read(cx)
            .collection_quality(&collection.reference)
            .and_then(|summary| collection_quality_metadata(&summary));
        metadata_label(collection.reference.provider, quality, true)
    }

    fn track_list_metadata(&self, track_list: &TrackList, cx: &App) -> Option<rows::RowMetadata> {
        let provider = track_list
            .tracks
            .first()
            .map(|track| track.reference.provider)
            .unwrap_or(track_list.collection.reference.provider);
        let quality = self
            .library_catalog
            .read(cx)
            .collection_quality(&track_list.collection.reference)
            .or_else(|| summarize_track_list_quality(track_list))
            .and_then(|summary| collection_quality_metadata(&summary));
        metadata_label(provider, quality, provider != ProviderId::Local)
    }

    fn track_metadata_for_collection(
        &self,
        track: &TrackSummary,
        collection_provider: ProviderId,
        collection_quality: Option<&CollectionQualitySummary>,
        cx: &App,
    ) -> Option<rows::RowMetadata> {
        let track_quality = self
            .library_catalog
            .read(cx)
            .track_quality(track)
            .or_else(|| audio_quality_from_track_summary(track));
        let track_grade = track_quality
            .as_ref()
            .and_then(normalized_audio_quality_grade);
        let show_quality = match (track_grade, collection_quality) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(track_grade), Some(CollectionQualitySummary::Uniform(collection_quality))) => {
                track_grade != *collection_quality
            }
            (Some(_), Some(CollectionQualitySummary::Mixed)) => true,
        };

        let quality = if show_quality {
            track_quality
                .as_ref()
                .and_then(normalized_audio_quality_grade)
                .map(rows::quality_metadata_for_grade)
        } else {
            None
        };

        metadata_label(
            track.reference.provider,
            quality,
            track.reference.provider != collection_provider,
        )
    }

    pub(super) fn render_provider_auth_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (
            focus_username,
            focus_password,
            provider_auth_error,
            provider_auth_submitting,
            provider_auth_prompt,
        ) = {
            let ui_state = self.ui_state.read(cx);
            (
                self.provider_auth_username_focus_handle.is_focused(window),
                self.provider_auth_password_focus_handle.is_focused(window),
                ui_state.provider_auth_error(),
                ui_state.provider_auth_submitting(),
                ui_state.provider_auth_prompt(),
            )
        };
        let provider_name = self
            .provider_for_id(provider_auth_prompt.unwrap_or(ProviderId::Local))
            .map(|provider| provider.display_name().to_string())
            .or_else(|| {
                provider_auth_prompt.map(|provider_id| provider_id.display_name().to_string())
            })
            .unwrap_or_else(|| "Provider".to_string());

        let username_field = div()
            .cursor_text()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(if focus_username {
                theme::ACCENT_PRIMARY
            } else {
                theme::BORDER_SUBTLE
            }))
            .bg(rgb(theme::SURFACE_BASE))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_2))
            .track_focus(&self.provider_auth_username_focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.focus_text_input(&TextInputId::ProviderAuthUsername, window);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .line_height(px(20.))
                    .text_size(px(theme::META_SIZE))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.on_text_input_mouse_down(
                                TextInputId::ProviderAuthUsername,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(
                                TextInputId::ProviderAuthUsername,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(
                                TextInputId::ProviderAuthUsername,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_move(
                        cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                            this.on_text_input_mouse_move(
                                TextInputId::ProviderAuthUsername,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .child(TextInputElement {
                        app: cx.entity(),
                        input_id: TextInputId::ProviderAuthUsername,
                        placeholder: "Username",
                        masked: false,
                    }),
            );

        let password_field = div()
            .cursor_text()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(if focus_password {
                theme::ACCENT_PRIMARY
            } else {
                theme::BORDER_SUBTLE
            }))
            .bg(rgb(theme::SURFACE_BASE))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_2))
            .track_focus(&self.provider_auth_password_focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.focus_text_input(&TextInputId::ProviderAuthPassword, window);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .line_height(px(20.))
                    .text_size(px(theme::META_SIZE))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.on_text_input_mouse_down(
                                TextInputId::ProviderAuthPassword,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(
                                TextInputId::ProviderAuthPassword,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(
                                TextInputId::ProviderAuthPassword,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_move(
                        cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                            this.on_text_input_mouse_move(
                                TextInputId::ProviderAuthPassword,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .child(TextInputElement {
                        app: cx.entity(),
                        input_id: TextInputId::ProviderAuthPassword,
                        placeholder: "Password",
                        masked: true,
                    }),
            );

        ui::render_modal_overlay(ui::render_modal_card(
            ui::ModalWidth::Narrow,
            ui::render_modal_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_3))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .child(format!("Sign in to {provider_name}")),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child(
                                        "Enter your username and password to fetch authenticated playback URLs."
                                            .to_string(),
                                    ),
                            ),
                    )
                    .child(div().flex().flex_col().gap(px(theme::SPACE_2)).child(username_field).child(password_field))
                    .when_some(provider_auth_error.as_ref(), |panel, error| {
                        panel.child(
                            div()
                                .text_size(px(theme::SMALL_SIZE))
                                .text_color(rgb(theme::ACCENT_PRIMARY))
                                .child(error.clone()),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(theme::SPACE_2))
                            .child(
                                div()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_2))
                                    .rounded(px(10.))
                                    .cursor_pointer()
                                    .bg(rgb(theme::SURFACE_BASE))
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                            this.close_provider_auth_prompt(cx);
                                        }),
                                    )
                                    .child("Cancel".to_string()),
                            )
                            .child(
                                div()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_2))
                                    .rounded(px(10.))
                                    .cursor_pointer()
                                    .bg(rgb(theme::ACCENT_PRIMARY))
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::BG_CANVAS))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                            this.submit_provider_auth(cx);
                                        }),
                                    )
                                    .child(if provider_auth_submitting {
                                        "Signing in...".to_string()
                                    } else {
                                        "Sign in".to_string()
                                    }),
                            ),
                    ),
            ),
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                this.close_provider_auth_prompt(cx);
            }),
        )
    }

    pub(super) fn render_provider_link_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (focus_prompt, provider_link_error, provider_link_submitting, prompt_mode) = {
            let ui_state = self.ui_state.read(cx);
            (
                self.provider_link_focus_handle.is_focused(window),
                ui_state.provider_link_error(),
                ui_state.provider_link_submitting(),
                ui_state
                    .provider_link_prompt()
                    .unwrap_or(ui::ProviderLinkPromptMode::Import),
            )
        };
        let (title, description, placeholder, submit_label) = match prompt_mode {
            ui::ProviderLinkPromptMode::Import => (
                "Import Provider Link",
                "Paste a compact provider link or raw TOML. Oryx will validate it before promotion.",
                "oryx-provider://v1/...",
                if provider_link_submitting {
                    "Importing..."
                } else {
                    "Import"
                },
            ),
            ui::ProviderLinkPromptMode::Export => (
                "Export Provider Link",
                "Enter a provider id. Oryx will export the active provider config and copy a compact link to the clipboard.",
                "provider-id",
                "Copy Link",
            ),
        };

        let prompt_field = div()
            .cursor_text()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(if focus_prompt {
                theme::ACCENT_PRIMARY
            } else {
                theme::BORDER_SUBTLE
            }))
            .bg(rgb(theme::SURFACE_BASE))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_2))
            .track_focus(&self.provider_link_focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.focus_text_input(&TextInputId::ProviderLink, window);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .line_height(px(20.))
                    .text_size(px(theme::META_SIZE))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.on_text_input_mouse_down(
                                TextInputId::ProviderLink,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(
                                TextInputId::ProviderLink,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(
                                TextInputId::ProviderLink,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_move(
                        cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                            this.on_text_input_mouse_move(
                                TextInputId::ProviderLink,
                                event,
                                window,
                                cx,
                            );
                        }),
                    )
                    .child(TextInputElement {
                        app: cx.entity(),
                        input_id: TextInputId::ProviderLink,
                        placeholder,
                        masked: false,
                    }),
            );

        ui::render_modal_overlay(ui::render_modal_card(
            ui::ModalWidth::Narrow,
            ui::render_modal_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_3))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .child(title.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child(description.to_string()),
                            ),
                    )
                    .child(prompt_field)
                    .when_some(provider_link_error.as_ref(), |panel, error| {
                        panel.child(
                            div()
                                .text_size(px(theme::SMALL_SIZE))
                                .text_color(rgb(theme::ACCENT_PRIMARY))
                                .child(error.clone()),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(theme::SPACE_2))
                            .child(
                                div()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_2))
                                    .rounded(px(10.))
                                    .cursor_pointer()
                                    .bg(rgb(theme::SURFACE_BASE))
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.close_provider_link_prompt(cx);
                                            },
                                        ),
                                    )
                                    .child("Cancel".to_string()),
                            )
                            .child(
                                div()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_2))
                                    .rounded(px(10.))
                                    .cursor_pointer()
                                    .bg(rgb(theme::ACCENT_PRIMARY))
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::BG_CANVAS))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.submit_provider_link_prompt(cx);
                                            },
                                        ),
                                    )
                                    .child(submit_label.to_string()),
                            ),
                    ),
            ),
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                this.close_provider_link_prompt(cx);
            }),
        )
    }

    pub(super) fn render_downloads_modal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let downloads = self.transfer_state.read(cx).download_items();
        let active_download_count = downloads.iter().filter(|item| item.is_active()).count();

        let body = if downloads.is_empty() {
            div()
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT_PRIMARY))
                        .child("Downloads".to_string()),
                )
                .child(
                    div()
                        .text_size(px(theme::META_SIZE))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(
                            "No downloads yet. Track downloads and external URL downloads will show up here."
                                .to_string(),
                        ),
                )
        } else {
            let mut column = div().flex().flex_col().gap(px(theme::SPACE_3)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .child("Downloads".to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::META_SIZE))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(if active_download_count == 0 {
                                format!("{} saved item(s)", downloads.len())
                            } else {
                                format!(
                                    "{} active transfer(s), {} total item(s)",
                                    active_download_count,
                                    downloads.len()
                                )
                            }),
                    ),
            );

            for download in downloads {
                let download_id = download.id.clone();
                let cancel_download_id = download_id.clone();
                let pause_download_id = download_id.clone();
                let snapshot = match &download.state {
                    DownloadItemState::Queued { progress, .. }
                    | DownloadItemState::Active { progress, .. } => Some(progress.snapshot()),
                    _ => None,
                };
                let download_title = download_filename(&download.state)
                    .filter(|filename| !filename.trim().is_empty())
                    .unwrap_or_else(|| download.title.clone());
                let progress_ratio = snapshot.and_then(download_progress_ratio).unwrap_or(0.0);
                let is_paused = snapshot.is_some_and(|snapshot| snapshot.paused);
                let is_external_pending = matches!(
                    download.state,
                    DownloadItemState::Queued { .. } | DownloadItemState::Active { .. }
                ) && matches!(
                    download.purpose,
                    crate::transfer::DownloadPurpose::ExternalUrl
                );
                let purpose = match download.purpose {
                    crate::transfer::DownloadPurpose::Explicit => "Download",
                    crate::transfer::DownloadPurpose::PlaybackPrefetch => "Playback cache",
                    crate::transfer::DownloadPurpose::ExternalUrl => "External URL",
                };
                let secondary_line = match &download.state {
                    DownloadItemState::Queued { source_url, .. } => source_url.clone(),
                    DownloadItemState::Active {
                        source_url,
                        destination,
                        ..
                    } => destination
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .or_else(|| source_url.clone())
                        .unwrap_or_else(|| purpose.to_string()),
                    DownloadItemState::Completed { destination, .. } => {
                        destination.display().to_string()
                    }
                    DownloadItemState::Failed {
                        destination, error, ..
                    } => destination
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| error.clone()),
                };
                let metadata_line = download_metadata_line(&download.state, snapshot);
                let stream_action = match (&download.state, snapshot, download.purpose) {
                    (
                        DownloadItemState::Active {
                            destination: Some(destination),
                            started_at,
                            duration_seconds,
                            ..
                        },
                        Some(snapshot),
                        crate::transfer::DownloadPurpose::ExternalUrl,
                    ) => stream_button_style(snapshot, started_at.clone(), *duration_seconds)
                        .map(|style| (destination.clone(), style)),
                    _ => None,
                };
                column = column.child(
                    div()
                        .w_full()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(theme::BORDER_SUBTLE))
                        .bg(rgb(theme::SURFACE_BASE))
                        .child(
                            div()
                                .px(px(theme::SPACE_3))
                                .py(px(theme::SPACE_3))
                                .flex()
                                .flex_col()
                                .gap(px(theme::SPACE_3))
                                .child(
                                    div().flex().items_center().gap(px(theme::SPACE_3)).child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .text_size(px(theme::BODY_SIZE))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .truncate()
                                                    .child(download_title),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(theme::SMALL_SIZE))
                                                    .text_color(rgb(theme::TEXT_DIM))
                                                    .truncate()
                                                    .child(secondary_line),
                                            )
                                            .when_some(metadata_line, |column, metadata_line| {
                                                column.child(
                                                    div()
                                                        .text_size(px(theme::SMALL_SIZE))
                                                        .text_color(rgb(theme::TEXT_MUTED))
                                                        .truncate()
                                                        .child(metadata_line),
                                                )
                                            }),
                                    ),
                                )
                                .when(snapshot.is_some(), |card| {
                                    card.child(
                                        div()
                                            .w_full()
                                            .h(px(3.))
                                            .overflow_hidden()
                                            .bg(rgb(theme::DOWNLOAD_PROGRESS_LIGHT))
                                            .child(
                                                div()
                                                    .h_full()
                                                    .w(relative(progress_ratio.clamp(0.0, 1.0)))
                                                    .bg(rgb(theme::DOWNLOAD_PROGRESS)),
                                            ),
                                    )
                                })
                                .when(
                                    is_external_pending
                                        || stream_action.is_some()
                                        || matches!(
                                            download.state,
                                            DownloadItemState::Completed { .. }
                                                | DownloadItemState::Failed { .. }
                                        ),
                                    |card| {
                                        let actions =
                                            div().flex().justify_center().gap(px(theme::SPACE_2));
                                        let actions = if is_external_pending {
                                            let pause_icon = if is_paused {
                                                AppIcon::Play
                                            } else {
                                                AppIcon::Pause
                                            };
                                            let actions = actions.child(
                                                download_icon_button(
                                                    pause_icon,
                                                    neutral_download_action_style(),
                                                    false,
                                                )
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(
                                                        move |this,
                                                              _event: &MouseDownEvent,
                                                              _window,
                                                              cx| {
                                                            if is_paused {
                                                                this.resume_external_download(
                                                                    pause_download_id.clone(),
                                                                    cx,
                                                                );
                                                            } else {
                                                                this.pause_external_download(
                                                                    pause_download_id.clone(),
                                                                    cx,
                                                                );
                                                            }
                                                        },
                                                    ),
                                                ),
                                            );
                                            let actions = if let Some((destination, style)) =
                                                stream_action.clone()
                                            {
                                                actions.child(
                                                    download_icon_button(
                                                        AppIcon::PlayCircle,
                                                        stream_download_action_style(style),
                                                        false,
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(
                                                            move |this,
                                                                  _event: &MouseDownEvent,
                                                                  _window,
                                                                  cx| {
                                                                this.open_external_download(
                                                                    destination.clone(),
                                                                    cx,
                                                                );
                                                            },
                                                        ),
                                                    ),
                                                )
                                            } else {
                                                actions.child(download_icon_button(
                                                    AppIcon::PlayCircle,
                                                    neutral_download_action_style(),
                                                    true,
                                                ))
                                            };
                                            actions.child(
                                                download_icon_button(
                                                    AppIcon::X,
                                                    destructive_download_action_style(),
                                                    false,
                                                )
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(
                                                        move |this,
                                                              _event: &MouseDownEvent,
                                                              _window,
                                                              cx| {
                                                            this.cancel_external_download(
                                                                cancel_download_id.clone(),
                                                                cx,
                                                            );
                                                        },
                                                    ),
                                                ),
                                            )
                                        } else {
                                            actions
                                        };
                                        let actions = match &download.state {
                                            DownloadItemState::Completed {
                                                destination, ..
                                            } => {
                                                let destination = destination.clone();
                                                actions.child(
                                                    download_icon_button(
                                                        AppIcon::PlayCircle,
                                                        neutral_download_action_style(),
                                                        false,
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(
                                                            move |this,
                                                                  _event: &MouseDownEvent,
                                                                  _window,
                                                                  cx| {
                                                                this.open_external_download(
                                                                    destination.clone(),
                                                                    cx,
                                                                );
                                                            },
                                                        ),
                                                    ),
                                                )
                                            }
                                            DownloadItemState::Failed {
                                                source_url,
                                                destination,
                                                ..
                                            } => {
                                                let source_url = source_url.clone();
                                                let destination = destination.clone();
                                                actions.child(
                                                    div()
                                                        .px(px(theme::SPACE_3))
                                                        .py(px(theme::SPACE_2))
                                                        .rounded(px(10.))
                                                        .cursor_pointer()
                                                        .border_1()
                                                        .border_color(rgb(theme::BORDER_SUBTLE))
                                                        .text_size(px(theme::SMALL_SIZE))
                                                        .text_color(rgb(theme::TEXT_MUTED))
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(
                                                                move |this,
                                                                      _event: &MouseDownEvent,
                                                                      _window,
                                                                      cx| {
                                                                    this.retry_external_download(
                                                                        download_id.clone(),
                                                                        source_url.clone(),
                                                                        destination.clone(),
                                                                        cx,
                                                                    );
                                                                },
                                                            ),
                                                        )
                                                        .child("Retry".to_string()),
                                                )
                                            }
                                            _ => actions,
                                        };
                                        card.child(actions)
                                    },
                                ),
                        ),
                );
            }

            column
        };

        ui::render_modal_overlay(ui::render_modal_card(
            ui::ModalWidth::Medium,
            ui::render_modal_body(
                body.child(
                    div().flex().justify_end().child(
                        div()
                            .px(px(theme::SPACE_3))
                            .py(px(theme::SPACE_2))
                            .rounded(px(10.))
                            .cursor_pointer()
                            .bg(rgb(theme::SURFACE_BASE))
                            .text_size(px(theme::META_SIZE))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                    this.close_downloads_modal(cx);
                                }),
                            )
                            .child("Close".to_string()),
                    ),
                ),
            ),
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                this.close_downloads_modal(cx);
            }),
        )
    }

    pub(super) fn render_lists(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let layout = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .bg(rgb(theme::SURFACE_BASE))
            .child(match self.browse_mode {
                BrowseMode::Discover => self.render_results_panel(window, cx),
                BrowseMode::Albums => self.render_local_albums_panel(window, cx),
                BrowseMode::Artists => self.render_local_artists_panel(window, cx),
                BrowseMode::Playlists => self.render_local_playlists_panel(window, cx),
            })
            .child(vertical_divider())
            .child(self.render_tracks_panel(window, cx));

        if self.should_show_playback_context_in_layout(window, cx) {
            layout
                .child(vertical_divider())
                .child(self.render_playback_context_panel(window, cx))
        } else {
            layout
        }
    }

    pub(super) fn render_import_review_modal(
        &self,
        review: &ImportReview,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let viewport_width = window.viewport_size().width.to_f64() as f32;
        let viewport_height = window.viewport_size().height.to_f64() as f32;
        let modal_max_width = (viewport_width - 48.0).max(320.0);
        let modal_max_height = (viewport_height - 48.0).max(320.0);
        let matched_tracks = review.ready_track_count();
        let unresolved_tracks = review.unresolved_track_count();
        let skipped_tracks = review.skipped_track_count();
        let mut body = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .id("import-review-scroll")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_3))
            .child(
                div()
                    .w_full()
                    .rounded(px(10.))
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_3))
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_3))
                    .child(
                        div()
                            .text_size(px(theme::SMALL_SIZE))
                            .text_color(rgb(theme::ACCENT_PRIMARY))
                            .child("Import Review"),
                    )
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(review.source_root.display().to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::META_SIZE))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(format!(
                                "{} album group(s)  •  {} ready offline or resolved  •  {} need attention  •  {} skipped",
                                review.albums.len(),
                                matched_tracks,
                                unresolved_tracks,
                                skipped_tracks
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::SPACE_2))
                            .child(action_button(
                                "Import Ready Files",
                                true,
                                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                                    this.commit_pending_import_review(window, cx);
                                }),
                            ))
                            .child(action_button(
                                "Cancel",
                                false,
                                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                    this.cancel_pending_import_review(cx);
                                }),
                            )),
                    ),
            );

        for (index, album) in review.albums.iter().enumerate() {
            body = body.child(self.render_import_album_review(album, index, window, cx));
        }

        ui::render_modal_overlay(ui::render_modal_card_sized(
            ui::ModalWidth::Wide,
            modal_max_width,
            modal_max_height,
            ui::render_modal_body(body),
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                this.cancel_pending_import_review(cx);
            }),
        )
    }

    pub(super) fn render_import_review_loading_modal(&self, _cx: &mut Context<Self>) -> gpui::Div {
        ui::render_modal_overlay(ui::render_modal_card(
            ui::ModalWidth::Narrow,
            ui::render_modal_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_3))
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT_PRIMARY))
                            .child("Analyzing Import".to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::META_SIZE))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child("Preparing the import review…".to_string()),
                    ),
            ),
        ))
    }

    fn render_import_album_review(
        &self,
        album: &ImportAlbumReview,
        _index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let detected_title = album
            .detected_album
            .as_ref()
            .map(|detected| detected.title.clone())
            .unwrap_or_else(|| "Mixed or Unresolved Metadata".to_string());
        let detected_artist = album
            .detected_album
            .as_ref()
            .map(|detected| detected.artist.clone())
            .unwrap_or_else(|| album.source_label.clone());
        let mut section = div()
            .w_full()
            .rounded(px(10.))
            .bg(rgb(theme::SURFACE_FLOATING))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_3))
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_3))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_3))
                    .child(render_collection_artwork(album.artwork_url.clone(), 88.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .text_color(rgb(theme::ACCENT_PRIMARY))
                                    .child(format!(
                                        "{} ready  •  {} attention  •  {} skipped",
                                        album.ready_track_count(),
                                        album.unresolved_track_count(),
                                        album.skipped_track_count()
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .child(detected_title),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .overflow_hidden()
                                    .child(detected_artist),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .text_color(rgb(theme::TEXT_DIM))
                                    .overflow_hidden()
                                    .child(album.source_label.clone()),
                            ),
                    ),
            );

        for warning in &album.warnings {
            section = section.child(
                div()
                    .w_full()
                    .rounded(px(8.))
                    .bg(rgb(theme::SURFACE_BASE))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_2))
                    .text_size(px(theme::SMALL_SIZE))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(warning.clone()),
            );
        }

        for track in &album.tracks {
            section = section.child(self.render_import_track_review(track, window, cx));
        }

        section
    }

    fn render_import_track_review(
        &self,
        track: &ImportTrackReview,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (state_label, state_color, state_background) = if track.skipped {
            ("Skipped", theme::TEXT_DIM, theme::ROW_IDLE_BG)
        } else if track.detected_track.is_some()
            && track.metadata_source == Some(ImportMetadataSource::LocalTags)
        {
            ("Ready Offline", 0xFF90D2A0, 0xFF243126)
        } else if track.detected_track.is_some()
            && track.metadata_source == Some(ImportMetadataSource::OnlineServices)
        {
            (
                "Ready Online",
                theme::ACCENT_PRIMARY,
                theme::ACCENT_PRIMARY_LIGHT,
            )
        } else if track.detected_track.is_some() {
            ("Ready Manual", 0xFFB7C565, 0xFF2A3220)
        } else {
            ("Needs Input", 0xFFF2C27B, 0xFF3B3021)
        };

        let file_name = track
            .source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| track.original_title.as_str())
            .to_string();
        let missing_fields = if track.missing_fields.is_empty() {
            None
        } else {
            Some(
                track
                    .missing_fields
                    .iter()
                    .map(|field| field.label())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        };

        let source_path_for_skip = track.source_path.clone();
        let source_path_for_unskip = track.source_path.clone();
        let source_path_for_manual = track.source_path.clone();
        let source_path_for_online = track.source_path.clone();
        let analysis_path_for_online = track.analysis_path.clone();
        let metadata_summary = track
            .detected_track
            .as_ref()
            .map(|detected| {
                let title = detected
                    .track_number
                    .map(|number| format!("{number:02} {}", detected.title))
                    .unwrap_or_else(|| detected.title.clone());
                format!("{title}  •  {}  •  {}", detected.artist, detected.album)
            })
            .unwrap_or_else(|| "No importable metadata yet.".to_string());

        let mut row = div()
            .w_full()
            .rounded(px(8.))
            .bg(rgb(theme::SURFACE_BASE))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_3))
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_3))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(theme::SPACE_3))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_size(px(theme::BODY_SIZE))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .child(file_name),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .text_color(rgb(theme::TEXT_DIM))
                                    .overflow_hidden()
                                    .child(track.source_path.display().to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .overflow_hidden()
                                    .child(metadata_summary),
                            )
                            .when_some(track.metadata_source_label(), |column, label| {
                                column.child(
                                    div()
                                        .text_size(px(theme::SMALL_SIZE))
                                        .text_color(rgb(theme::TEXT_DIM))
                                        .child(format!("Source: {label}")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .rounded(px(theme::RADIUS_FULL))
                            .bg(rgb(state_background))
                            .border_1()
                            .border_color(rgb(state_color))
                            .px(px(theme::SPACE_2))
                            .py(px(6.))
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(state_color))
                                    .child(state_label),
                            ),
                    ),
            );

        if let Some(issue) = track.issue.as_ref() {
            row = row.child(
                div()
                    .w_full()
                    .rounded(px(8.))
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_2))
                    .text_size(px(theme::SMALL_SIZE))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(issue.clone()),
            );
        }

        if let Some(missing_fields) = missing_fields {
            row = row.child(
                div()
                    .text_size(px(theme::SMALL_SIZE))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child(format!("Missing from local tags: {missing_fields}")),
            );
        }

        row = row.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(theme::SPACE_2))
                .when(track.skipped, |actions| {
                    actions.child(action_button(
                        "Unskip",
                        false,
                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                            this.skip_pending_import_track(
                                source_path_for_unskip.clone(),
                                false,
                                cx,
                            );
                        }),
                    ))
                })
                .when(!track.skipped, |actions| {
                    actions.child(action_button(
                        "Skip",
                        false,
                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                            this.skip_pending_import_track(source_path_for_skip.clone(), true, cx);
                        }),
                    ))
                })
                .when(track.needs_attention(), |actions| {
                    actions
                        .child(action_button(
                            "Use Online",
                            true,
                            cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                                this.resolve_pending_import_track_online(
                                    source_path_for_online.clone(),
                                    analysis_path_for_online.clone(),
                                    window,
                                    cx,
                                );
                            }),
                        ))
                        .child(action_button(
                            if track.manual_mode {
                                "Editing Manual Info"
                            } else {
                                "Fill Manually"
                            },
                            !track.manual_mode,
                            cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                                this.begin_pending_import_track_manual_entry(
                                    source_path_for_manual.clone(),
                                    window,
                                    cx,
                                );
                            }),
                        ))
                }),
        );

        if track.manual_mode {
            row = row.child(
                div()
                    .w_full()
                    .rounded(px(8.))
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_3))
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_2))
                    .child(
                        div()
                            .text_size(px(theme::SMALL_SIZE))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child("Manual metadata"),
                    )
                    .child(self.render_import_metadata_input(
                        "Title",
                        "Track title",
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::Title,
                        },
                        window,
                        cx,
                    ))
                    .child(self.render_import_metadata_input(
                        "Artist",
                        "Track artist",
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::Artist,
                        },
                        window,
                        cx,
                    ))
                    .child(self.render_import_metadata_input(
                        "Album",
                        "Album title",
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::Album,
                        },
                        window,
                        cx,
                    ))
                    .child(self.render_import_metadata_input(
                        "Album Artist",
                        "Album artist (optional)",
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::AlbumArtist,
                        },
                        window,
                        cx,
                    ))
                    .child(self.render_import_metadata_input(
                        "Disc Number",
                        "Disc number (optional)",
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::DiscNumber,
                        },
                        window,
                        cx,
                    ))
                    .child(self.render_import_metadata_input(
                        "Track Number",
                        "Track number (optional)",
                        TextInputId::ImportManual {
                            source_path: track.source_path.clone(),
                            field: ImportMetadataField::TrackNumber,
                        },
                        window,
                        cx,
                    )),
            );
        }

        row
    }

    fn render_import_metadata_input(
        &self,
        label: &'static str,
        placeholder: &'static str,
        input_id: TextInputId,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let focused = self.text_input_focus_handle(&input_id).is_focused(window);
        let focus_input_id = input_id.clone();
        let mouse_down_input_id = input_id.clone();
        let mouse_up_input_id = input_id.clone();
        let mouse_up_out_input_id = input_id.clone();
        let mouse_move_input_id = input_id.clone();

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_1))
            .child(
                div()
                    .text_size(px(theme::SMALL_SIZE))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child(label),
            )
            .child(
                div()
                    .cursor_text()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(if focused {
                        theme::ACCENT_PRIMARY
                    } else {
                        theme::BORDER_SUBTLE
                    }))
                    .bg(rgb(theme::SURFACE_BASE))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_2))
                    .track_focus(self.text_input_focus_handle(&input_id))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                            this.focus_text_input(&focus_input_id, window);
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .line_height(px(20.))
                            .text_size(px(theme::META_SIZE))
                            .cursor_text()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    this.on_text_input_mouse_down(
                                        mouse_down_input_id.clone(),
                                        event,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, event: &gpui::MouseUpEvent, window, cx| {
                                    this.on_text_input_mouse_up(
                                        mouse_up_input_id.clone(),
                                        event,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(move |this, event: &gpui::MouseUpEvent, window, cx| {
                                    this.on_text_input_mouse_up(
                                        mouse_up_out_input_id.clone(),
                                        event,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, event: &gpui::MouseMoveEvent, window, cx| {
                                    this.on_text_input_mouse_move(
                                        mouse_move_input_id.clone(),
                                        event,
                                        window,
                                        cx,
                                    );
                                },
                            ))
                            .child(TextInputElement {
                                app: cx.entity().clone(),
                                input_id,
                                placeholder,
                                masked: false,
                            }),
                    ),
            )
    }

    pub(super) fn render_context_menu_overlay(
        &self,
        menu: &ContextMenuState,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let panel = div()
            .absolute()
            .left(menu.position.x)
            .top(menu.position.y)
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .bg(rgb(theme::SURFACE_FLOATING))
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                |_event: &MouseDownEvent, window: &mut Window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                |_event: &MouseDownEvent, window: &mut Window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                },
            )
            .p(px(theme::SPACE_1))
            .child(self.render_context_menu_action(menu, cx));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_context_menu(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_context_menu(cx);
                }),
            )
            .child(panel)
    }

    fn render_context_menu_action(
        &self,
        menu: &ContextMenuState,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let target = menu.target.clone();
        let label = match &target {
            ContextMenuTarget::LocalAlbum { .. } => "Remove Downloaded Album",
            ContextMenuTarget::LocalTrack { .. } => "Remove Downloaded Track",
        };

        div()
            .px(px(theme::SPACE_2))
            .py(px(theme::SPACE_2))
            .rounded(px(8.))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .text_size(px(theme::META_SIZE))
            .text_color(rgb(theme::TEXT_MUTED))
            .hover(|style| {
                style
                    .bg(rgb(theme::ACCENT_PRIMARY_LIGHT))
                    .text_color(rgb(theme::ACCENT_PRIMARY))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    match &target {
                        ContextMenuTarget::LocalAlbum {
                            provider,
                            collection_id,
                            title,
                        } => this.delete_local_album_from_library(
                            *provider,
                            collection_id.clone(),
                            title.clone(),
                            cx,
                        ),
                        ContextMenuTarget::LocalTrack {
                            provider,
                            track_id,
                            title,
                        } => this.delete_local_track_from_library(
                            *provider,
                            track_id.clone(),
                            title.clone(),
                            cx,
                        ),
                    }
                }),
            )
            .child(render_icon_with_color(
                AppIcon::Trash,
                theme::ACTION_ICON_SIZE,
                theme::TEXT_MUTED,
            ))
            .child(label.to_string())
    }
}

fn fluid_panel_width(preferred: f32, minimum: f32, maximum: f32) -> f32 {
    preferred.clamp(minimum, maximum)
}
