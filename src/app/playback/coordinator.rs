use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{AsyncApp, Context, Entity, WeakEntity};

use crate::provider::TrackList;
use crate::transfer::{DownloadPurpose, TransferEvent};

use super::super::transfer_state::TransferStateModel;
use super::super::ui::NotificationLevel;
use super::super::{OryxApp, collection_entity_key};
use super::{PendingPlayRequest, media_session_track};

impl OryxApp {
    pub(in crate::app) fn spawn_transfer_listener(
        receiver: Arc<Mutex<Receiver<TransferEvent>>>,
        transfer_state: Entity<TransferStateModel>,
        cx: &mut Context<Self>,
    ) {
        let background = cx.background_executor().clone();
        cx.spawn(move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let background = background.clone();
            let mut async_cx = cx.clone();
            let transfer_state = transfer_state.clone();
            async move {
                loop {
                    let receiver = receiver.clone();
                    let event = background
                        .spawn(async move { receiver.lock().ok()?.recv().ok() })
                        .await;

                    let Some(event) = event else {
                        break;
                    };

                    if transfer_state
                        .update(&mut async_cx, |state, cx| {
                            state.handle_worker_event(event, cx);
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

    pub(in crate::app) fn play_track_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(playback_context) = self.current_visible_track_list_cloned(cx) else {
            return;
        };

        self.start_playback_for_context(playback_context, index, None, cx);
    }

    pub(in crate::app) fn play_track_at_position(
        &mut self,
        index: usize,
        position: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        let Some(playback_context) = self.playback_state.read(cx).playback_context() else {
            return;
        };

        self.start_playback_for_context(playback_context, index, position, cx);
    }

    pub(in crate::app) fn download_track_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(selected_track) = self
            .current_visible_track_list(cx)
            .and_then(|track_list| track_list.tracks.into_iter().nth(index))
        else {
            return;
        };

        if self.track_is_cached(&selected_track, cx)
            || self.track_is_downloading(&selected_track, cx)
        {
            return;
        }

        let Some(provider) = self.provider_for_id(selected_track.reference.provider) else {
            self.status_message = Some(format!(
                "Provider '{}' is not available.",
                selected_track.reference.provider
            ));
            cx.notify();
            return;
        };
        let library = self.library.clone();
        let track_title = selected_track.title.clone();
        self.status_message = Some(format!("Downloading '{}'.", track_title));
        cx.notify();
        self.transfer
            .queue_download(provider, library, selected_track, Some(index));
    }

    pub(in crate::app) fn cancel_download_track_at(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_track) = self
            .current_visible_track_list(cx)
            .and_then(|track_list| track_list.tracks.into_iter().nth(index))
        else {
            return;
        };

        let cancelled = self.transfer_state.update(cx, |state, _cx| {
            state.cancel_explicit_download(&selected_track)
        });
        if cancelled {
            self.status_message = Some(format!(
                "Cancelled download for '{}'.",
                selected_track.title
            ));
            cx.notify();
        }
    }

    pub(in crate::app) fn start_playback_for_context(
        &mut self,
        playback_context: TrackList,
        index: usize,
        position: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_track) = playback_context.tracks.get(index).cloned() else {
            return;
        };
        let browser_track_list = self.discover.read(cx).track_list();
        let browser_collection_id = self.discover.read(cx).selected_collection_id();
        let browser_contains_track = browser_track_list
            .as_ref()
            .map(|track_list| {
                collection_entity_key(&track_list.collection.reference)
                    == collection_entity_key(&playback_context.collection.reference)
            })
            .unwrap_or(false);

        let track_ref = selected_track.reference.clone();
        let provider = self.provider_for_id(track_ref.provider);
        let library = self.library.clone();
        let play_nonce = self.playback_state.update(cx, |state, _cx| {
            state.begin_play_request(
                PendingPlayRequest {
                    request_id: 0,
                    playback_context,
                    index,
                    position,
                    browser_track_list,
                    browser_collection_id,
                    browser_contains_track,
                },
                position.unwrap_or(Duration::ZERO),
            )
        });
        self.status_message = Some("Resolving, caching, and starting playback".to_string());
        cx.notify();
        self.transfer.queue_play_request(
            play_nonce,
            provider,
            library,
            selected_track,
            Some(index),
            position,
        );
    }

    pub(in crate::app) fn handle_transfer_event(
        &mut self,
        event: TransferEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            TransferEvent::DownloadStarted { .. } => {}
            TransferEvent::DownloadCompleted { title, purpose, .. } => {
                self.refresh_local_library_views(cx);
                if matches!(purpose, DownloadPurpose::Explicit) {
                    self.status_message = Some(format!("Saved '{}' for offline playback.", title));
                    self.show_notification(
                        format!("Saved '{}' for offline playback.", title),
                        NotificationLevel::Success,
                        cx,
                    );
                }
            }
            TransferEvent::DownloadCancelled { .. } => {}
            TransferEvent::DownloadFailed {
                track_id,
                title,
                purpose,
                error,
            } => {
                if self.transfer_state.read(cx).was_cancelled(&track_id) {
                    cx.notify();
                    return;
                }
                if matches!(purpose, DownloadPurpose::PlaybackPrefetch) {
                    eprintln!("playback prefetch failed for '{}': {error}", title);
                    cx.notify();
                    return;
                }
                let message = match purpose {
                    DownloadPurpose::Explicit => {
                        format!("Download failed for '{}': {error}", title)
                    }
                    DownloadPurpose::PlaybackPrefetch => unreachable!(),
                };
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Error, cx);
            }
            TransferEvent::PlaybackReady {
                request_id,
                playback,
            } => {
                let Some(request) = self.playback_state.update(cx, |state, _cx| {
                    state.take_pending_request_if_matches(request_id)
                }) else {
                    return;
                };

                self.update_playback_state(cx, |state| {
                    state.set_play_loading(false);
                });

                let media_track = media_session_track(&playback.current);
                match self.playback_state.read(cx).play_source_at(
                    media_track,
                    playback.source.clone(),
                    request.position,
                ) {
                    Ok(()) => {
                        if playback.fully_cached {
                            self.refresh_local_library_views(cx);
                        }
                        self.status_message =
                            Some(format!("Playing '{}'.", playback.current.title));
                        self.update_playback_state(cx, |state| {
                            state.apply_ready_playback(
                                request.playback_context.clone(),
                                request.index,
                                playback.current,
                            );
                        });
                        if request.browser_contains_track {
                            self.discover.update(cx, |discover, _cx| {
                                discover.sync_browser_playback_context(
                                    request.browser_collection_id.clone(),
                                    request.browser_track_list.clone(),
                                );
                            });
                        }
                        self.persist_session_snapshot(cx);
                    }
                    Err(error) => {
                        self.status_message = Some(format!("Failed to start playback: {error}"));
                        self.show_notification(
                            format!("Failed to start playback: {error}"),
                            NotificationLevel::Error,
                            cx,
                        );
                        self.update_playback_state(cx, |state| {
                            state.mark_playback_failed();
                        });
                    }
                }
            }
            TransferEvent::PlaybackFailed {
                request_id,
                title,
                error,
            } => {
                let Some(_request) = self.playback_state.update(cx, |state, _cx| {
                    state.take_pending_request_if_matches(request_id)
                }) else {
                    return;
                };
                let message = format!("Failed to resolve '{}': {error}", title);
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Error, cx);
                self.update_playback_state(cx, |state| {
                    state.mark_playback_failed();
                });
            }
        }
        cx.notify();
    }
}
