use std::time::Duration;

use gpui::Context;

use crate::library::{Library, SessionSnapshot};
use crate::model::{PlaybackStatus, RepeatMode, Track};
use crate::provider::{CollectionSummary, TrackList};

use super::{BrowseMode, OryxApp, collection_browser_key, collection_entity_key};

pub(super) struct RestoredSessionState {
    pub(super) query: String,
    pub(super) query_cursor: usize,
    pub(super) browse_mode: BrowseMode,
    pub(super) search_results: Vec<CollectionSummary>,
    pub(super) selected_collection_id: Option<String>,
    pub(super) track_list: Option<TrackList>,
    pub(super) playback_context: Option<TrackList>,
    pub(super) current_track_index: Option<usize>,
    pub(super) now_playing: Option<Track>,
    pub(super) resume_position: Duration,
    pub(super) playback_status: PlaybackStatus,
    pub(super) repeat_mode: RepeatMode,
    pub(super) shuffle_enabled: bool,
    pub(super) shuffle_seed: u64,
    pub(super) status_message: String,
    pub(super) selected_local_album_id: Option<String>,
    pub(super) selected_local_artist_id: Option<String>,
    pub(super) selected_local_playlist_id: Option<String>,
}

pub(super) fn restored_session_state(
    library: &Library,
    snapshot: Option<SessionSnapshot>,
) -> RestoredSessionState {
    let Some(snapshot) = snapshot else {
        return RestoredSessionState {
            query: String::new(),
            query_cursor: 0,
            browse_mode: BrowseMode::Discover,
            search_results: Vec::new(),
            selected_collection_id: None,
            track_list: None,
            playback_context: None,
            current_track_index: None,
            now_playing: None,
            resume_position: Duration::ZERO,
            playback_status: PlaybackStatus::Idle,
            repeat_mode: RepeatMode::Off,
            shuffle_enabled: false,
            shuffle_seed: 0,
            status_message: "Type a query and press Enter.".to_string(),
            selected_local_album_id: None,
            selected_local_artist_id: None,
            selected_local_playlist_id: None,
        };
    };

    let playback_context = snapshot
        .playback_context
        .clone()
        .or_else(|| snapshot.browser_track_list.clone());
    let selected_collection_id = snapshot
        .browser_track_list
        .as_ref()
        .map(|track_list| collection_entity_key(&track_list.collection.reference))
        .or_else(|| {
            snapshot
                .browser_collection_id
                .as_ref()
                .and_then(|selected| {
                    snapshot.search_results.iter().find_map(|collection| {
                        (collection_browser_key(&collection.reference) == *selected
                            || collection_entity_key(&collection.reference) == *selected)
                            .then(|| collection_entity_key(&collection.reference))
                    })
                })
        })
        .or_else(|| snapshot.browser_collection_id.clone());

    let now_playing = playback_context
        .as_ref()
        .and_then(|track_list| {
            snapshot
                .current_track_index
                .and_then(|index| track_list.tracks.get(index))
        })
        .cloned()
        .map(|track| {
            let cached = library.cached_track(&track).ok().flatten();
            let source = cached
                .as_ref()
                .map(|cached| cached.audio_path.display().to_string())
                .unwrap_or_else(|| track.reference.canonical_url.clone().unwrap_or_default());
            Track::from_track_summary_with_source(
                track,
                source,
                cached.and_then(|cached| cached.artwork_path),
            )
        });

    let saved_playback_status = snapshot.playback_status.clone();
    let playback_status = match (saved_playback_status, now_playing.is_some()) {
        (PlaybackStatus::Playing, true) => PlaybackStatus::Paused,
        (PlaybackStatus::Buffering, true) => PlaybackStatus::Paused,
        (status, _) => status,
    };

    let resume_position = snapshot.playback_position();
    let query = snapshot.query;

    RestoredSessionState {
        query_cursor: query.len(),
        query,
        browse_mode: snapshot.browse_mode,
        search_results: snapshot.search_results,
        selected_collection_id,
        track_list: snapshot.browser_track_list,
        playback_context,
        current_track_index: snapshot.current_track_index,
        now_playing,
        resume_position,
        playback_status,
        repeat_mode: snapshot.repeat_mode,
        shuffle_enabled: snapshot.shuffle_enabled,
        shuffle_seed: snapshot.shuffle_seed,
        status_message: "Restored previous session.".to_string(),
        selected_local_album_id: snapshot.selected_local_album_id,
        selected_local_artist_id: snapshot.selected_local_artist_id,
        selected_local_playlist_id: snapshot.selected_local_playlist_id,
    }
}

pub(super) fn persist_session_snapshot(app: &OryxApp, cx: &gpui::App) {
    let discover = app.discover.read(cx);
    let catalog = app.library_catalog.read(cx);
    let playback_state = app.playback_state.read(cx);
    let selected_local_album_id = catalog.selected_local_collection_id_owned(BrowseMode::Albums);
    let selected_local_artist_id = catalog.selected_local_collection_id_owned(BrowseMode::Artists);
    let selected_local_playlist_id =
        catalog.selected_local_collection_id_owned(BrowseMode::Playlists);
    let snapshot = SessionSnapshot {
        query: app.query_input.content().to_string(),
        browse_mode: app.browse_mode,
        search_results: discover.search_results(),
        browser_collection_id: discover.selected_collection_id(),
        browser_track_list: discover.track_list(),
        playback_context: playback_state.playback_context(),
        current_track_index: playback_state.current_track_index(),
        playback_status: playback_state.playback_status(),
        repeat_mode: playback_state.repeat_mode(),
        shuffle_enabled: playback_state.shuffle_enabled(),
        shuffle_seed: playback_state.shuffle_seed(),
        playback_position_seconds: playback_state
            .position()
            .ok()
            .flatten()
            .unwrap_or(playback_state.resume_position())
            .as_secs(),
        selected_local_album_id,
        selected_local_artist_id,
        selected_local_playlist_id,
    };

    if let Err(error) = app.library.save_session_snapshot(&snapshot) {
        eprintln!("failed to persist session snapshot: {error}");
    }
}

impl OryxApp {
    pub(super) fn persist_current_playback_position(&mut self, cx: &mut Context<Self>) {
        let position = self.playback_state.read(cx).current_playback_position();
        self.update_playback_state(cx, |state| {
            state.set_resume_position(position);
        });
        self.persist_session_snapshot(cx);
    }

    pub(super) fn handle_shutdown_request(&mut self, cx: &mut Context<Self>) {
        self.persist_current_playback_position(cx);
        cx.quit();
    }
}
