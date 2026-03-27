use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{AsyncApp, Context, Entity, WeakEntity};

use crate::library::RECENTLY_PLAYED_PLAYLIST_ID;
use crate::provider::{CollectionKind, ProviderId, TrackList, TrackSummary};
use crate::transfer::{DownloadPurpose, ReadyPlayback, TransferEvent};

use super::super::transfer_state::TransferStateModel;
use super::super::ui::NotificationLevel;
use super::super::{OryxApp, collection_entity_key, provider_collection_ref_for_local_album};
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
        let Some(track_list) = self.current_visible_track_list(cx) else {
            return;
        };
        let collection_id_override =
            refreshable_collection_for_track_list(&track_list, track_list.tracks.get(index))
                .map(|(_, collection_id)| collection_id);
        let Some(selected_track) = track_list.tracks.get(index).cloned() else {
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
        self.transfer.queue_download(
            provider,
            library,
            selected_track,
            Some(index),
            collection_id_override,
        );
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
        let is_cached = self.track_is_cached(&selected_track, cx);
        let collection_id_override = refreshable_collection_for_track_list(
            &playback_context,
            playback_context.tracks.get(index),
        )
        .map(|(_, collection_id)| collection_id);
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
                !is_cached,
            )
        });
        self.status_message = Some(if is_cached {
            "Starting playback.".to_string()
        } else {
            "Resolving, caching, and starting playback".to_string()
        });
        cx.notify();
        self.transfer.queue_play_request(
            play_nonce,
            provider,
            library,
            selected_track,
            Some(index),
            collection_id_override,
            position,
        );
    }

    pub(in crate::app) fn handle_transfer_event(
        &mut self,
        event: TransferEvent,
        cx: &mut Context<Self>,
    ) {
        let should_persist_external_downloads = matches!(
            &event,
            TransferEvent::ExternalDownloadQueued { .. }
                | TransferEvent::ExternalDownloadStarted { .. }
                | TransferEvent::ExternalDownloadCompleted { .. }
                | TransferEvent::ExternalDownloadCancelled { .. }
                | TransferEvent::ExternalDownloadFailed { .. }
        );
        match event {
            TransferEvent::DownloadStarted { .. } => {}
            TransferEvent::DownloadCompleted {
                title,
                provider,
                collection_id,
                purpose,
                ..
            } => {
                if let Some(collection_id) = collection_id.as_deref() {
                    self.library_catalog.update(cx, |catalog, _cx| {
                        catalog.refresh_album_collection(provider, collection_id);
                    });
                } else {
                    self.refresh_local_library_views(cx);
                }
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
                    DownloadPurpose::ExternalUrl => {
                        format!("Download failed for '{}': {error}", title)
                    }
                };
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Error, cx);
            }
            TransferEvent::ExternalDownloadQueued { title, .. } => {
                self.status_message = Some(format!("Queued '{title}' for download."));
                cx.notify();
            }
            TransferEvent::ExternalDownloadStarted { title, .. } => {
                self.status_message = Some(format!("Downloading '{title}' to Downloads."));
                cx.notify();
            }
            TransferEvent::ExternalDownloadCompleted {
                title, destination, ..
            } => {
                let message = format!("Downloaded '{}' to {}.", title, destination.display());
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Success, cx);
            }
            TransferEvent::ExternalDownloadCancelled { .. } => {}
            TransferEvent::ExternalDownloadFailed { title, error, .. } => {
                let message = format!("Open Media download failed for '{}': {error}", title);
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
                        let refreshable_collection = refreshable_collection_for_track_list(
                            &request.playback_context,
                            request.playback_context.tracks.get(request.index),
                        );
                        let mut should_refresh_playlists = false;
                        if should_record_recently_played(&request.playback_context) {
                            let recently_played_track = request
                                .playback_context
                                .tracks
                                .get(request.index)
                                .cloned()
                                .map(|track| {
                                    track_for_recently_played(
                                        track,
                                        playback.current.artwork_path.as_ref(),
                                    )
                                });
                            let has_recently_played_track = recently_played_track.is_some();
                            if let Some(track) = recently_played_track
                                && let Err(error) =
                                    self.library.record_recently_played_track(&track)
                            {
                                eprintln!(
                                    "failed to record recently played track '{}:{}': {error:#}",
                                    track.reference.provider.as_str(),
                                    track.reference.id
                                );
                            } else if has_recently_played_track {
                                should_refresh_playlists = true;
                            }
                        }
                        match playback_refresh_target(
                            &playback,
                            refreshable_collection,
                            should_refresh_playlists,
                        ) {
                            PlaybackRefreshTarget::None => {}
                            PlaybackRefreshTarget::PlaylistsOnly => {
                                self.refresh_local_playlists(cx);
                            }
                            PlaybackRefreshTarget::Album {
                                provider,
                                collection_id,
                            } => {
                                self.refresh_local_album_collection(provider, &collection_id, cx);
                            }
                            PlaybackRefreshTarget::FullLibrary => {
                                self.refresh_local_library_views(cx);
                            }
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
        if should_persist_external_downloads {
            self.persist_session_snapshot(cx);
        }
        cx.notify();
    }
}

fn refreshable_collection_for_track_list(
    track_list: &TrackList,
    track: Option<&TrackSummary>,
) -> Option<(ProviderId, String)> {
    if let Some(track) = track {
        if let Some(collection_id) = track.collection_id.clone() {
            return Some((track.reference.provider, collection_id));
        }

        if let Some(collection) = matching_collection_for_track(track_list, track) {
            return Some(collection);
        }
    }

    if track_list.collection.reference.kind != CollectionKind::Album {
        return None;
    }

    if track_list.collection.reference.provider == ProviderId::Local {
        if is_local_artist_track_list(track_list) {
            return None;
        }
        return provider_collection_ref_for_local_album(track_list)
            .map(|collection| (collection.provider, collection.id));
    }

    Some((
        track_list.collection.reference.provider,
        track_list.collection.reference.id.clone(),
    ))
}

fn matching_collection_for_track(
    track_list: &TrackList,
    track: &TrackSummary,
) -> Option<(ProviderId, String)> {
    let track_album = track
        .collection_title
        .as_deref()
        .or(track.album.as_deref())?;
    let track_artist = track
        .collection_subtitle
        .as_deref()
        .or(track.artist.as_deref());

    track_list.tracks.iter().find_map(|candidate| {
        let candidate_collection_id = candidate.collection_id.as_ref()?;
        (candidate.reference.provider == track.reference.provider
            && candidate
                .collection_title
                .as_deref()
                .or(candidate.album.as_deref())
                == Some(track_album)
            && candidate
                .collection_subtitle
                .as_deref()
                .or(candidate.artist.as_deref())
                == track_artist)
            .then(|| {
                (
                    candidate.reference.provider,
                    candidate_collection_id.clone(),
                )
            })
    })
}

fn is_local_artist_track_list(track_list: &TrackList) -> bool {
    track_list.collection.reference.provider == ProviderId::Local
        && track_list.collection.reference.kind == CollectionKind::Album
        && track_list
            .collection
            .reference
            .id
            .starts_with("local-artist:")
}

fn should_record_recently_played(playback_context: &TrackList) -> bool {
    let reference = &playback_context.collection.reference;
    !(reference.provider == crate::provider::ProviderId::Local
        && reference.id == RECENTLY_PLAYED_PLAYLIST_ID)
}

#[derive(Debug, PartialEq, Eq)]
enum PlaybackRefreshTarget {
    None,
    PlaylistsOnly,
    Album {
        provider: ProviderId,
        collection_id: String,
    },
    FullLibrary,
}

fn playback_refresh_target(
    playback: &ReadyPlayback,
    refreshable_collection: Option<(ProviderId, String)>,
    playlists_changed: bool,
) -> PlaybackRefreshTarget {
    if playback.cache_changed {
        if let Some((provider, collection_id)) = refreshable_collection {
            PlaybackRefreshTarget::Album {
                provider,
                collection_id,
            }
        } else {
            PlaybackRefreshTarget::FullLibrary
        }
    } else if playlists_changed {
        PlaybackRefreshTarget::PlaylistsOnly
    } else {
        PlaybackRefreshTarget::None
    }
}

fn track_for_recently_played(
    mut track: crate::provider::TrackSummary,
    artwork_path: Option<&std::path::PathBuf>,
) -> crate::provider::TrackSummary {
    if let Some(artwork_path) = artwork_path {
        track.artwork_url = Some(artwork_path.to_string_lossy().into_owned());
    }
    track
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CollectionKind, CollectionRef, CollectionSummary, ProviderId, TrackRef};
    use crate::{audio::PlaybackSource, model::Track};

    #[test]
    fn skips_recording_when_playing_from_recently_played_playlist() {
        let track_list = track_list(
            ProviderId::Local,
            RECENTLY_PLAYED_PLAYLIST_ID,
            CollectionKind::Playlist,
        );

        assert!(!should_record_recently_played(&track_list));
    }

    #[test]
    fn records_when_playing_from_other_local_playlists() {
        let track_list = track_list(ProviderId::Local, "liked-tracks", CollectionKind::Playlist);

        assert!(should_record_recently_played(&track_list));
    }

    #[test]
    fn records_when_playing_from_albums() {
        let track_list = track_list(fixture_provider(), "album-1", CollectionKind::Album);

        assert!(should_record_recently_played(&track_list));
    }

    #[test]
    fn recently_played_prefers_local_artwork_path_when_available() {
        let track = track_summary("track-1", Some("https://cdn.example/art.jpg"));
        let updated =
            track_for_recently_played(track, Some(&std::path::PathBuf::from("/tmp/oryx-art.jpg")));

        assert_eq!(updated.artwork_url.as_deref(), Some("/tmp/oryx-art.jpg"));
    }

    #[test]
    fn playback_refresh_target_only_refreshes_playlists_for_recently_played_updates() {
        assert_eq!(
            playback_refresh_target(
                &ready_playback(false),
                Some((fixture_provider(), "album-1".to_string())),
                true,
            ),
            PlaybackRefreshTarget::PlaylistsOnly
        );
    }

    #[test]
    fn playback_refresh_target_refreshes_album_when_cache_changed() {
        assert_eq!(
            playback_refresh_target(
                &ready_playback(true),
                Some((fixture_provider(), "album-1".to_string())),
                true,
            ),
            PlaybackRefreshTarget::Album {
                provider: fixture_provider(),
                collection_id: "album-1".to_string(),
            }
        );
    }

    #[test]
    fn refreshable_collection_uses_track_collection_id_when_present() {
        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    ProviderId::Local,
                    "local-artist:Artist",
                    CollectionKind::Album,
                    None,
                ),
                title: "Artist".to_string(),
                subtitle: None,
                artwork_url: None,
                track_count: Some(1),
            },
            tracks: vec![track_summary("track-1", None)],
        };

        assert_eq!(
            refreshable_collection_for_track_list(&track_list, track_list.tracks.first()),
            Some((fixture_provider(), "album-1".to_string()))
        );
    }

    #[test]
    fn refreshable_collection_falls_back_to_remote_album_reference() {
        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    fixture_provider(),
                    "album-42",
                    CollectionKind::Album,
                    None,
                ),
                title: "Album".to_string(),
                subtitle: None,
                artwork_url: None,
                track_count: Some(1),
            },
            tracks: vec![track_summary_without_collection("track-1")],
        };

        assert_eq!(
            refreshable_collection_for_track_list(&track_list, track_list.tracks.first()),
            Some((fixture_provider(), "album-42".to_string()))
        );
    }

    #[test]
    fn refreshable_collection_matches_selected_track_album_in_artist_view() {
        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    ProviderId::Local,
                    "local-artist:Artist",
                    CollectionKind::Album,
                    None,
                ),
                title: "Artist".to_string(),
                subtitle: None,
                artwork_url: None,
                track_count: Some(3),
            },
            tracks: vec![
                track_summary("track-1", None),
                TrackSummary {
                    reference: TrackRef::new(
                        fixture_provider(),
                        "track-2",
                        None,
                        Some("track-2".to_string()),
                    ),
                    title: "track-2".to_string(),
                    artist: Some("Artist".to_string()),
                    album: Some("Album B".to_string()),
                    collection_id: None,
                    collection_title: Some("Album B".to_string()),
                    collection_subtitle: Some("Artist".to_string()),
                    duration_seconds: None,
                    bitrate_bps: None,
                    audio_format: None,
                    artwork_url: None,
                },
                TrackSummary {
                    reference: TrackRef::new(
                        fixture_provider(),
                        "track-3",
                        None,
                        Some("track-3".to_string()),
                    ),
                    title: "track-3".to_string(),
                    artist: Some("Artist".to_string()),
                    album: Some("Album B".to_string()),
                    collection_id: Some("album-b".to_string()),
                    collection_title: Some("Album B".to_string()),
                    collection_subtitle: Some("Artist".to_string()),
                    duration_seconds: None,
                    bitrate_bps: None,
                    audio_format: None,
                    artwork_url: None,
                },
            ],
        };

        assert_eq!(
            refreshable_collection_for_track_list(&track_list, track_list.tracks.get(1)),
            Some((fixture_provider(), "album-b".to_string()))
        );
    }

    #[test]
    fn refreshable_collection_does_not_guess_from_first_track_for_local_artist_view() {
        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    ProviderId::Local,
                    "local-artist:Artist",
                    CollectionKind::Album,
                    None,
                ),
                title: "Artist".to_string(),
                subtitle: None,
                artwork_url: None,
                track_count: Some(2),
            },
            tracks: vec![
                track_summary("track-1", None),
                TrackSummary {
                    reference: TrackRef::new(
                        fixture_provider(),
                        "track-2",
                        None,
                        Some("track-2".to_string()),
                    ),
                    title: "track-2".to_string(),
                    artist: Some("Artist".to_string()),
                    album: Some("Other Album".to_string()),
                    collection_id: None,
                    collection_title: Some("Other Album".to_string()),
                    collection_subtitle: Some("Artist".to_string()),
                    duration_seconds: None,
                    bitrate_bps: None,
                    audio_format: None,
                    artwork_url: None,
                },
            ],
        };

        assert_eq!(
            refreshable_collection_for_track_list(&track_list, track_list.tracks.get(1)),
            None
        );
    }

    fn fixture_provider() -> ProviderId {
        ProviderId::parse("fixture_remote").expect("fixture provider id should parse")
    }

    fn track_list(provider: ProviderId, collection_id: &str, kind: CollectionKind) -> TrackList {
        TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(provider, collection_id, kind, None),
                title: "Collection".to_string(),
                subtitle: None,
                artwork_url: None,
                track_count: Some(1),
            },
            tracks: vec![],
        }
    }

    fn track_summary(id: &str, artwork_url: Option<&str>) -> crate::provider::TrackSummary {
        crate::provider::TrackSummary {
            reference: TrackRef::new(fixture_provider(), id, None, Some(id.to_string())),
            title: id.to_string(),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            collection_id: Some("album-1".to_string()),
            collection_title: Some("Album".to_string()),
            collection_subtitle: Some("Artist".to_string()),
            duration_seconds: None,
            bitrate_bps: None,
            audio_format: None,
            artwork_url: artwork_url.map(str::to_string),
        }
    }

    fn track_summary_without_collection(id: &str) -> crate::provider::TrackSummary {
        crate::provider::TrackSummary {
            collection_id: None,
            collection_title: None,
            ..track_summary(id, None)
        }
    }

    fn ready_playback(cache_changed: bool) -> ReadyPlayback {
        ReadyPlayback {
            current: Track::from_track_summary_with_source(
                track_summary("track-1", None),
                "/tmp/track.mp3".to_string(),
                None,
            ),
            source: PlaybackSource::LocalFile("/tmp/track.mp3".into()),
            fully_cached: true,
            cache_changed,
        }
    }
}
