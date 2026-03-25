use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::app::BrowseMode;
use crate::library::Library;
use crate::model::{PlaybackStatus, RepeatMode};
use crate::provider::{CollectionSummary, ProviderId, TrackList};

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
}

impl SessionSnapshot {
    pub fn playback_position(&self) -> Duration {
        Duration::from_secs(self.playback_position_seconds)
    }
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
