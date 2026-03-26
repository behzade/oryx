use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::app::BrowseMode;
use crate::library::Library;
use crate::model::{PlaybackStatus, RepeatMode};
use crate::provider::{CollectionKind, CollectionSummary, ProviderId, TrackList, TrackSummary};

pub(crate) const RECENTLY_PLAYED_PLAYLIST_ID: &str = "recently-played";
const RECENTLY_PLAYED_PLAYLIST_TITLE: &str = "Recently Played";
const RECENTLY_PLAYED_PLAYLIST_SUBTITLE: &str = "System playlist";
const RECENTLY_PLAYED_PLAYLIST_KEY: &str = "recently_played_tracks";
const RECENTLY_PLAYED_LIMIT: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub query: String,
    #[serde(default)]
    pub browse_mode: BrowseMode,
    pub search_results: Vec<CollectionSummary>,
    #[serde(default, alias = "selected_collection_id")]
    pub browser_collection_id: Option<String>,
    #[serde(default, alias = "track_list")]
    pub browser_track_list: Option<TrackList>,
    #[serde(default)]
    pub playback_context: Option<TrackList>,
    #[serde(default)]
    pub current_track_index: Option<usize>,
    #[serde(default)]
    pub playback_status: PlaybackStatus,
    #[serde(default)]
    pub repeat_mode: RepeatMode,
    #[serde(default)]
    pub shuffle_enabled: bool,
    #[serde(default)]
    pub shuffle_seed: u64,
    #[serde(default)]
    pub playback_position_seconds: u64,
    #[serde(default)]
    pub selected_local_album_id: Option<String>,
    #[serde(default)]
    pub selected_local_artist_id: Option<String>,
    #[serde(default)]
    pub selected_local_playlist_id: Option<String>,
    #[serde(default)]
    pub external_downloads: Vec<PersistedExternalDownload>,
}

impl SessionSnapshot {
    pub fn playback_position(&self) -> Duration {
        Duration::from_secs(self.playback_position_seconds)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedExternalDownload {
    pub id: String,
    pub title: String,
    pub source_url: String,
    #[serde(default)]
    pub destination: Option<std::path::PathBuf>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub downloaded_bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    pub state: PersistedExternalDownloadState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PersistedExternalDownloadState {
    Pending,
    Completed {
        destination: std::path::PathBuf,
    },
    Failed {
        destination: Option<std::path::PathBuf>,
        error: String,
    },
}

pub(super) fn save_session_snapshot(library: &Library, snapshot: &SessionSnapshot) -> Result<()> {
    let connection = library.open_connection()?;
    let value = serde_json::to_string(snapshot).context("Failed to serialize session snapshot")?;

    connection.execute(
        r#"
        INSERT INTO app_state (key, value, updated_at)
        VALUES ('session_snapshot', ?1, unixepoch())
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = unixepoch()
        "#,
        params![value],
    )?;

    Ok(())
}

pub(super) fn load_session_snapshot(library: &Library) -> Result<Option<SessionSnapshot>> {
    let connection = library.open_connection()?;

    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = 'session_snapshot'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    value
        .map(|value| serde_json::from_str(&value).context("Failed to deserialize session snapshot"))
        .transpose()
}

pub(super) fn save_provider_auth(
    library: &Library,
    provider: ProviderId,
    serialized: &str,
) -> Result<()> {
    let connection = library.open_connection()?;
    let key = provider_auth_key(provider);

    connection.execute(
        r#"
        INSERT INTO app_state (key, value, updated_at)
        VALUES (?1, ?2, unixepoch())
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = unixepoch()
        "#,
        params![key, serialized],
    )?;

    Ok(())
}

pub(super) fn load_provider_auth(
    library: &Library,
    provider: ProviderId,
) -> Result<Option<String>> {
    let connection = library.open_connection()?;
    let key = provider_auth_key(provider);

    connection
        .query_row(
            "SELECT value FROM app_state WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

pub(super) fn load_recently_played_playlist(library: &Library) -> Result<Option<TrackList>> {
    let connection = library.open_connection()?;
    let stored_tracks: Option<String> = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = ?1",
            params![RECENTLY_PLAYED_PLAYLIST_KEY],
            |row| row.get(0),
        )
        .optional()?;

    let Some(stored_tracks) = stored_tracks else {
        return Ok(None);
    };
    let tracks: Vec<TrackSummary> = serde_json::from_str(&stored_tracks)
        .context("Failed to deserialize recently played tracks")?;
    let tracks = hydrate_recently_played_tracks(library, tracks)?;
    if tracks.is_empty() {
        return Ok(None);
    }

    Ok(Some(recently_played_track_list(tracks)))
}

pub(super) fn record_recently_played_track(library: &Library, track: &TrackSummary) -> Result<()> {
    let connection = library.open_connection()?;
    let existing_tracks = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = ?1",
            params![RECENTLY_PLAYED_PLAYLIST_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|stored_tracks| {
            serde_json::from_str::<Vec<TrackSummary>>(&stored_tracks)
                .context("Failed to deserialize recently played tracks")
        })
        .transpose()?
        .unwrap_or_default();

    let track = hydrate_recently_played_track(library, track.clone())?;
    let tracks = prepend_recent_track(existing_tracks, track, RECENTLY_PLAYED_LIMIT);
    let value =
        serde_json::to_string(&tracks).context("Failed to serialize recently played tracks")?;
    connection.execute(
        r#"
        INSERT INTO app_state (key, value, updated_at)
        VALUES (?1, ?2, unixepoch())
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = unixepoch()
        "#,
        params![RECENTLY_PLAYED_PLAYLIST_KEY, value],
    )?;

    Ok(())
}

pub(super) fn save_collection_track_list(
    library: &Library,
    _collection: &crate::provider::CollectionRef,
    track_list: &TrackList,
) -> Result<()> {
    let connection = library.open_connection()?;
    super::cache::sync_cached_metadata_for_track_list(&connection, track_list)?;
    super::entities::sync_collection_track_list(&connection, track_list)?;

    Ok(())
}

fn provider_auth_key(provider: ProviderId) -> String {
    format!("provider_auth:{}", provider.as_str())
}

fn recently_played_track_list(tracks: Vec<TrackSummary>) -> TrackList {
    TrackList {
        collection: CollectionSummary {
            reference: crate::provider::CollectionRef::new(
                ProviderId::Local,
                RECENTLY_PLAYED_PLAYLIST_ID,
                CollectionKind::Playlist,
                None,
            ),
            title: RECENTLY_PLAYED_PLAYLIST_TITLE.to_string(),
            subtitle: Some(RECENTLY_PLAYED_PLAYLIST_SUBTITLE.to_string()),
            artwork_url: None,
            track_count: Some(tracks.len()),
        },
        tracks,
    }
}

fn hydrate_recently_played_tracks(
    library: &Library,
    tracks: Vec<TrackSummary>,
) -> Result<Vec<TrackSummary>> {
    tracks
        .into_iter()
        .map(|track| hydrate_recently_played_track(library, track))
        .collect()
}

fn hydrate_recently_played_track(
    library: &Library,
    mut track: TrackSummary,
) -> Result<TrackSummary> {
    if let Some(cached) = library.cached_track(&track)?
        && let Some(artwork_path) = cached.artwork_path
    {
        track.artwork_url = Some(artwork_path.to_string_lossy().into_owned());
    }

    Ok(track)
}

fn prepend_recent_track(
    mut tracks: Vec<TrackSummary>,
    track: TrackSummary,
    limit: usize,
) -> Vec<TrackSummary> {
    tracks.retain(|existing| {
        !(existing.reference.provider == track.reference.provider
            && existing.reference.id == track.reference.id)
    });
    tracks.insert(0, track);
    tracks.truncate(limit.max(1));
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ProviderId, TrackRef};

    fn fixture_track(id: &str) -> TrackSummary {
        TrackSummary {
            reference: TrackRef::new(
                ProviderId::parse("fixture_remote").expect("fixture provider"),
                id,
                None,
                Some(format!("Track {id}")),
            ),
            title: format!("Track {id}"),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            collection_id: Some("album-1".to_string()),
            collection_title: Some("Album".to_string()),
            collection_subtitle: Some("Artist".to_string()),
            duration_seconds: Some(180),
            bitrate_bps: None,
            audio_format: None,
            artwork_url: None,
        }
    }

    #[test]
    fn prepend_recent_track_moves_existing_track_to_front_without_duplicates() {
        let tracks = prepend_recent_track(
            vec![fixture_track("a"), fixture_track("b")],
            fixture_track("a"),
            10,
        );

        let ids = tracks
            .iter()
            .map(|track| track.reference.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn prepend_recent_track_trims_to_limit() {
        let tracks = prepend_recent_track(
            vec![fixture_track("a"), fixture_track("b"), fixture_track("c")],
            fixture_track("z"),
            3,
        );

        let ids = tracks
            .iter()
            .map(|track| track.reference.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["z", "a", "b"]);
    }

    #[test]
    fn recently_played_track_list_uses_local_playlist_identity() {
        let track_list = recently_played_track_list(vec![fixture_track("a")]);

        assert_eq!(track_list.collection.reference.provider, ProviderId::Local);
        assert_eq!(
            track_list.collection.reference.id,
            RECENTLY_PLAYED_PLAYLIST_ID
        );
        assert_eq!(
            track_list.collection.reference.kind,
            CollectionKind::Playlist
        );
        assert_eq!(track_list.collection.title, RECENTLY_PLAYED_PLAYLIST_TITLE);
        assert_eq!(track_list.collection.track_count, Some(1));
    }
}
