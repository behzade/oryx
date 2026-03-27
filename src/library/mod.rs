mod cache;
mod entities;
mod import;
mod provider_state;
mod session;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::audio::PlaybackSource;
use crate::metadata::{MetadataPolicy, ResolvedTrackMetadata};
use crate::platform;
use crate::progressive::ProgressiveDownload;
use crate::provider::{
    AudioFormat, CollectionRef, CollectionSummary, MusicProvider, ProviderId, SongData, TrackList,
    TrackSummary,
};

pub use import::{
    ArtworkBackfillSummary, ImportAlbumReview, ImportMetadataField, ImportMetadataSource,
    ImportReview, ImportSummary, ImportTrackReview,
};
pub(crate) const LIKED_PLAYLIST_ID: &str = entities::LIKED_PLAYLIST_ID;
pub use provider_state::ProviderRuntimeState;
pub(crate) const RECENTLY_PLAYED_PLAYLIST_ID: &str = session::RECENTLY_PLAYED_PLAYLIST_ID;
pub use session::{PersistedExternalDownload, PersistedExternalDownloadState, SessionSnapshot};

#[derive(Clone, Debug)]
pub struct Library {
    library_root: PathBuf,
    db_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct CachedTrack {
    pub audio_path: PathBuf,
    pub artwork_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct CachedLibraryTrack {
    pub track: TrackSummary,
    pub collection_provider: crate::provider::ProviderId,
}

#[derive(Clone, Debug)]
pub struct PreparedPlaybackTrack {
    pub source: PlaybackSource,
    pub display_path: PathBuf,
    pub artwork_path: Option<PathBuf>,
    pub bitrate_bps: Option<u32>,
    pub audio_format: Option<AudioFormat>,
    pub fully_cached: bool,
    pub cache_changed: bool,
    pub cache_monitor: Option<ProgressiveDownload>,
}

impl Library {
    pub fn new() -> Result<Self> {
        let root = oryx_root_dir()?;
        Self::new_in(root)
    }

    pub fn new_in(root: PathBuf) -> Result<Self> {
        let library_root = root.join("library");
        let db_path = root.join("oryx.db");

        fs::create_dir_all(&library_root).with_context(|| {
            format!(
                "Failed to create library root at {}",
                library_root.display()
            )
        })?;

        let library = Self {
            library_root,
            db_path,
        };
        library.initialize()?;

        Ok(library)
    }

    pub fn ensure_track_cached_with_progress(
        &self,
        provider: &dyn MusicProvider,
        selected_track: &TrackSummary,
        track_position: Option<usize>,
        song: &SongData,
        progress: Option<&ProgressiveDownload>,
    ) -> Result<CachedTrack> {
        cache::ensure_track_cached_with_progress(
            self,
            provider,
            selected_track,
            track_position,
            song,
            progress,
        )
    }

    pub fn prepare_track_for_playback(
        &self,
        provider: &dyn MusicProvider,
        selected_track: &TrackSummary,
        track_position: Option<usize>,
        song: &SongData,
        position: Option<std::time::Duration>,
    ) -> Result<PreparedPlaybackTrack> {
        cache::prepare_track_for_playback(
            self,
            provider,
            selected_track,
            track_position,
            song,
            position,
        )
    }

    pub fn prepare_cached_track_for_playback(
        &self,
        selected_track: &TrackSummary,
        track_position: Option<usize>,
    ) -> Result<Option<PreparedPlaybackTrack>> {
        cache::prepare_cached_track_for_playback(self, selected_track, track_position)
    }

    pub fn cached_track(&self, track: &TrackSummary) -> Result<Option<CachedTrack>> {
        cache::cached_track(self, track)
    }

    pub fn all_cached_track_ids(&self) -> Result<HashSet<String>> {
        cache::all_cached_track_ids(self)
    }

    pub fn cached_library_tracks(&self) -> Result<Vec<CachedLibraryTrack>> {
        cache::cached_library_tracks(self)
    }

    pub fn cached_library_tracks_for_collection(
        &self,
        provider: ProviderId,
        collection_id: &str,
    ) -> Result<Vec<CachedLibraryTrack>> {
        cache::cached_library_tracks_for_collection(self, provider, collection_id)
    }

    pub fn entity_playlist_track_lists(&self) -> Result<Vec<TrackList>> {
        let connection = self.open_connection()?;
        entities::playlist_track_lists(&connection)
    }

    pub fn liked_track_keys(&self) -> Result<HashSet<String>> {
        let connection = self.open_connection()?;
        entities::liked_track_keys(&connection)
    }

    pub fn toggle_track_liked(&self, track: &TrackSummary) -> Result<bool> {
        let connection = self.open_connection()?;
        entities::toggle_liked_track(&connection, track)
    }

    pub fn entity_album_track_lists(&self) -> Result<Vec<TrackList>> {
        let connection = self.open_connection()?;
        entities::album_track_lists(&connection)
    }

    pub fn stage_local_selection(&self, selections: &[PathBuf]) -> Result<ImportReview> {
        self.stage_local_selection_with_metadata_policy(selections, MetadataPolicy::TagsOnly)
    }

    pub fn stage_local_selection_with_metadata_policy(
        &self,
        selections: &[PathBuf],
        metadata_policy: MetadataPolicy,
    ) -> Result<ImportReview> {
        import::stage_local_selection(self, selections, metadata_policy)
    }

    pub fn commit_import_review(&self, review: &ImportReview) -> Result<ImportSummary> {
        import::commit_import_review(self, review)
    }

    pub fn cleanup_import_review(&self, review: &ImportReview) -> Result<()> {
        import::cleanup_import_review(review)
    }

    pub fn resolve_import_track_online(
        &self,
        analysis_path: &std::path::Path,
    ) -> Result<(ResolvedTrackMetadata, Option<String>)> {
        import::resolve_import_track_online(analysis_path)
    }

    pub fn backfill_local_artwork(&self) -> Result<ArtworkBackfillSummary> {
        import::backfill_local_artwork(self)
    }

    pub fn ensure_collection_artwork_cached(
        &self,
        provider: &dyn MusicProvider,
        collection: &CollectionSummary,
    ) -> Result<Option<PathBuf>> {
        cache::ensure_collection_artwork_cached(self, provider, collection)
    }

    pub fn save_collection_track_list(
        &self,
        collection: &CollectionRef,
        track_list: &TrackList,
    ) -> Result<()> {
        session::save_collection_track_list(self, collection, track_list)
    }

    pub fn load_collection_track_list(
        &self,
        collection: &CollectionRef,
    ) -> Result<Option<TrackList>> {
        let connection = self.open_connection()?;
        entities::load_collection_track_list(&connection, collection)
    }

    pub fn save_session_snapshot(&self, snapshot: &SessionSnapshot) -> Result<()> {
        session::save_session_snapshot(self, snapshot)
    }

    pub fn load_session_snapshot(&self) -> Result<Option<SessionSnapshot>> {
        session::load_session_snapshot(self)
    }

    pub fn save_provider_auth(&self, provider: ProviderId, serialized: &str) -> Result<()> {
        session::save_provider_auth(self, provider, serialized)
    }

    pub fn load_provider_auth(&self, provider: ProviderId) -> Result<Option<String>> {
        session::load_provider_auth(self, provider)
    }

    pub fn load_recently_played_playlist(&self) -> Result<Option<TrackList>> {
        session::load_recently_played_playlist(self)
    }

    pub fn record_recently_played_track(&self, track: &TrackSummary) -> Result<()> {
        session::record_recently_played_track(self, track)
    }

    pub fn load_provider_runtime_state(
        &self,
        provider: ProviderId,
    ) -> Result<Option<ProviderRuntimeState>> {
        provider_state::load_provider_runtime_state(self, provider)
    }

    pub fn save_validated_provider_manifest(
        &self,
        provider: ProviderId,
        manifest_hash: &str,
        manifest_toml: &str,
        display_name: &str,
    ) -> Result<()> {
        provider_state::save_validated_provider_manifest(
            self,
            provider,
            manifest_hash,
            manifest_toml,
            display_name,
        )
    }

    pub fn record_provider_validation_failure(
        &self,
        provider: ProviderId,
        candidate_hash: &str,
        error: &str,
    ) -> Result<()> {
        provider_state::record_provider_validation_failure(self, provider, candidate_hash, error)
    }

    pub fn record_provider_validation_pending_auth(
        &self,
        provider: ProviderId,
        candidate_hash: &str,
    ) -> Result<()> {
        provider_state::record_provider_validation_pending_auth(self, provider, candidate_hash)
    }

    pub fn delete_collection_from_library(
        &self,
        provider: ProviderId,
        collection_id: &str,
    ) -> Result<usize> {
        cache::delete_collection_from_library(self, provider, collection_id)
    }

    pub fn delete_track_from_library(&self, provider: ProviderId, track_id: &str) -> Result<bool> {
        cache::delete_track_from_library(self, provider, track_id)
    }

    fn initialize(&self) -> Result<()> {
        let connection = self.open_connection()?;

        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS cached_tracks (
                provider TEXT NOT NULL,
                track_id TEXT NOT NULL,
                canonical_url TEXT,
                artist TEXT,
                album TEXT,
                title TEXT NOT NULL,
                collection_id TEXT,
                collection_title TEXT,
                collection_subtitle TEXT,
                track_position INTEGER,
                duration_seconds INTEGER,
                bitrate_bps INTEGER,
                audio_format TEXT,
                stream_url TEXT NOT NULL,
                audio_path TEXT NOT NULL,
                artwork_path TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY(provider, track_id)
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_cached_tracks_audio_path
                ON cached_tracks(audio_path);
            CREATE TABLE IF NOT EXISTS app_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            "#,
        )?;

        entities::initialize_schema(&connection)?;
        provider_state::initialize_schema(&connection)?;

        ensure_cached_tracks_duration_column(&connection)?;
        ensure_cached_tracks_track_position_column(&connection)?;
        ensure_cached_tracks_collection_columns(&connection)?;
        ensure_cached_tracks_quality_columns(&connection)?;

        Ok(())
    }

    fn open_connection(&self) -> Result<Connection> {
        Connection::open(&self.db_path).with_context(|| {
            format!(
                "Failed to open library database at {}",
                self.db_path.display()
            )
        })
    }
}

fn ensure_cached_tracks_duration_column(connection: &Connection) -> Result<()> {
    if has_cached_tracks_column(connection, "duration_seconds")? {
        return Ok(());
    }

    connection.execute(
        "ALTER TABLE cached_tracks ADD COLUMN duration_seconds INTEGER",
        [],
    )?;

    Ok(())
}

fn ensure_cached_tracks_track_position_column(connection: &Connection) -> Result<()> {
    if has_cached_tracks_column(connection, "track_position")? {
        return Ok(());
    }

    connection.execute(
        "ALTER TABLE cached_tracks ADD COLUMN track_position INTEGER",
        [],
    )?;

    Ok(())
}

fn ensure_cached_tracks_collection_columns(connection: &Connection) -> Result<()> {
    for column in [
        ("collection_id", "TEXT"),
        ("collection_title", "TEXT"),
        ("collection_subtitle", "TEXT"),
    ] {
        if has_cached_tracks_column(connection, column.0)? {
            continue;
        }

        connection.execute(
            &format!(
                "ALTER TABLE cached_tracks ADD COLUMN {} {}",
                column.0, column.1
            ),
            [],
        )?;
    }

    Ok(())
}

fn ensure_cached_tracks_quality_columns(connection: &Connection) -> Result<()> {
    for column in [("bitrate_bps", "INTEGER"), ("audio_format", "TEXT")] {
        if has_cached_tracks_column(connection, column.0)? {
            continue;
        }

        connection.execute(
            &format!(
                "ALTER TABLE cached_tracks ADD COLUMN {} {}",
                column.0, column.1
            ),
            [],
        )?;
    }

    Ok(())
}

fn has_cached_tracks_column(connection: &Connection, target: &str) -> Result<bool> {
    let mut statement = connection.prepare("PRAGMA table_info(cached_tracks)")?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;

    for column in columns {
        if column? == target {
            return Ok(true);
        }
    }

    Ok(false)
}

fn oryx_root_dir() -> Result<PathBuf> {
    platform::app_root_dir()
}
