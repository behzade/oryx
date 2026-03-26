use std::collections::{HashMap, HashSet, hash_map::DefaultHasher};
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use lofty::config::{ParseOptions, ParsingMode};
use lofty::prelude::AudioFile;
use lofty::probe::Probe;
use rodio::{Decoder, Source};
use rusqlite::{Connection, OptionalExtension, params};
use url::Url;

use crate::audio::PlaybackSource;
use crate::library::{CachedLibraryTrack, CachedTrack, Library, PreparedPlaybackTrack};
use crate::pathing::sanitize_path_component;
use crate::progressive::ProgressiveDownload;
use crate::provider::{
    AudioFormat, CollectionKind, CollectionSummary, DownloadRequest, MusicProvider, ProviderId,
    SongData, TrackList, TrackRef, TrackSummary, network_agent,
};

const INITIAL_PLAYBACK_BUFFER: u64 = 256 * 1024;
const AUDIO_DURATION_TOLERANCE_SECONDS: u32 = 3;
const AUDIO_DURATION_TOLERANCE_PERCENT: u32 = 5;
const DOWNLOAD_RETRY_LIMIT: usize = 4;
const DOWNLOAD_RETRY_BASE_DELAY_MS: u64 = 750;

#[derive(Clone, Copy, Debug)]
enum DownloadRetryPolicy {
    Bounded(usize),
    Infinite,
}

pub(super) fn ensure_track_cached_with_progress(
    library: &Library,
    provider: &dyn MusicProvider,
    selected_track: &TrackSummary,
    track_position: Option<usize>,
    song: &SongData,
    progress: Option<&ProgressiveDownload>,
) -> Result<CachedTrack> {
    let mut connection = library.open_connection()?;
    let cached_track = resolved_track_for_cache(selected_track, &song.track);

    if let Some(existing) = lookup_cached_track(&mut connection, selected_track)? {
        backfill_cached_track_position(&mut connection, &cached_track, track_position)?;
        let audio_exists =
            cached_audio_file_is_valid(&existing.audio_path, cached_track.duration_seconds);
        let artwork_exists = existing
            .artwork_path
            .as_ref()
            .map(|path| path.is_file())
            .unwrap_or(true);

        if audio_exists && artwork_exists {
            return Ok(existing);
        }
    }

    let album_dir = album_directory(library, &cached_track);
    fs::create_dir_all(&album_dir).with_context(|| {
        format!(
            "Failed to create album directory at {}",
            album_dir.display()
        )
    })?;

    let audio_request = DownloadRequest::from_stream(&song.stream);
    let audio_path = resolve_audio_path(&connection, &album_dir, &cached_track, &audio_request)?;
    if !audio_path.is_file() {
        download_audio_to_path(
            &audio_request,
            &audio_path,
            progress,
            cached_track.duration_seconds,
            DownloadRetryPolicy::Bounded(DOWNLOAD_RETRY_LIMIT),
        )?;
    }

    let artwork_path = ensure_track_artwork_cached(provider, &cached_track, &album_dir)?;

    upsert_cached_track(
        &mut connection,
        &cached_track,
        track_position,
        &song.stream.url,
        &audio_path,
        artwork_path.as_deref(),
    )?;

    Ok(CachedTrack {
        audio_path,
        artwork_path,
    })
}

pub(super) fn prepare_track_for_playback(
    library: &Library,
    provider: &dyn MusicProvider,
    selected_track: &TrackSummary,
    track_position: Option<usize>,
    song: &SongData,
    position: Option<Duration>,
) -> Result<PreparedPlaybackTrack> {
    let mut connection = library.open_connection()?;
    let cached_track = resolved_track_for_cache(selected_track, &song.track);

    if let Some(existing) = lookup_cached_track(&mut connection, selected_track)? {
        backfill_cached_track_position(&mut connection, &cached_track, track_position)?;
        let audio_exists =
            cached_audio_file_is_valid(&existing.audio_path, cached_track.duration_seconds);
        let artwork_exists = existing
            .artwork_path
            .as_ref()
            .map(|path| path.is_file())
            .unwrap_or(true);

        if audio_exists && artwork_exists {
            let (bitrate_bps, audio_format) =
                read_audio_quality(&existing.audio_path, cached_track.duration_seconds)?;
            return Ok(PreparedPlaybackTrack {
                source: PlaybackSource::LocalFile(existing.audio_path.clone()),
                display_path: existing.audio_path,
                artwork_path: existing.artwork_path,
                bitrate_bps,
                audio_format,
                fully_cached: true,
                cache_monitor: None,
            });
        }
    }

    let album_dir = album_directory(library, &cached_track);
    fs::create_dir_all(&album_dir).with_context(|| {
        format!(
            "Failed to create album directory at {}",
            album_dir.display()
        )
    })?;

    let audio_request = DownloadRequest::from_stream(&song.stream);
    let audio_path = resolve_audio_path(&connection, &album_dir, &cached_track, &audio_request)?;
    let artwork_path = ensure_track_artwork_cached(provider, &cached_track, &album_dir)?;

    if cached_audio_file_is_valid(&audio_path, cached_track.duration_seconds) {
        let (bitrate_bps, audio_format) =
            read_audio_quality(&audio_path, cached_track.duration_seconds)?;
        upsert_cached_track(
            &mut connection,
            &cached_track,
            track_position,
            &song.stream.url,
            &audio_path,
            artwork_path.as_deref(),
        )?;
        return Ok(PreparedPlaybackTrack {
            source: PlaybackSource::LocalFile(audio_path.clone()),
            display_path: audio_path,
            artwork_path,
            bitrate_bps,
            audio_format,
            fully_cached: true,
            cache_monitor: None,
        });
    }

    if position.unwrap_or_default().is_zero() {
        let temp_path = temporary_download_path(&audio_path);
        let download = ProgressiveDownload::new();
        let monitor = download.clone();
        let library = library.clone();
        let cached_track = cached_track.clone();
        let track_position = track_position;
        let stream_url = song.stream.url.clone();
        let artwork_for_db = artwork_path.clone();
        let request = audio_request.clone();
        let download_for_thread = download.clone();
        let thread_temp_path = temp_path.clone();
        let thread_audio_path = audio_path.clone();

        std::thread::Builder::new()
            .name("audio-cache-progressive".to_string())
            .spawn(move || {
                let result = complete_progressive_download(
                    &library,
                    &cached_track,
                    track_position,
                    &stream_url,
                    artwork_for_db,
                    request,
                    thread_temp_path,
                    thread_audio_path,
                    download_for_thread.clone(),
                );
                if let Err(error) = result {
                    let error_message = format!("{error:#}");
                    eprintln!(
                        "progressive audio download failed for '{}': {error_message}",
                        cached_track.title
                    );
                    download_for_thread.fail(error_message);
                }
            })
            .context("Failed to spawn progressive audio download worker")?;

        download
            .wait_for_buffer(INITIAL_PLAYBACK_BUFFER)
            .context("Progressive playback buffer did not become ready")?;

        return Ok(PreparedPlaybackTrack {
            source: PlaybackSource::GrowingFile {
                path: temp_path,
                final_path: audio_path.clone(),
                download,
            },
            display_path: audio_path,
            artwork_path,
            bitrate_bps: None,
            audio_format: None,
            fully_cached: false,
            cache_monitor: Some(monitor),
        });
    }

    if !audio_path.is_file() {
        download_audio_to_path(
            &audio_request,
            &audio_path,
            None,
            cached_track.duration_seconds,
            DownloadRetryPolicy::Bounded(DOWNLOAD_RETRY_LIMIT),
        )?;
    }

    upsert_cached_track(
        &mut connection,
        &cached_track,
        track_position,
        &song.stream.url,
        &audio_path,
        artwork_path.as_deref(),
    )?;

    let (bitrate_bps, audio_format) =
        read_audio_quality(&audio_path, cached_track.duration_seconds)?;

    Ok(PreparedPlaybackTrack {
        source: PlaybackSource::LocalFile(audio_path.clone()),
        display_path: audio_path,
        artwork_path,
        bitrate_bps,
        audio_format,
        fully_cached: true,
        cache_monitor: None,
    })
}

pub(super) fn prepare_cached_track_for_playback(
    library: &Library,
    selected_track: &TrackSummary,
    track_position: Option<usize>,
) -> Result<Option<PreparedPlaybackTrack>> {
    let mut connection = library.open_connection()?;
    let Some(existing) = lookup_cached_track(&mut connection, selected_track)? else {
        return Ok(None);
    };

    backfill_cached_track_position(&mut connection, selected_track, track_position)?;

    let audio_exists =
        cached_audio_file_is_valid(&existing.audio_path, selected_track.duration_seconds);
    let artwork_exists = existing
        .artwork_path
        .as_ref()
        .map(|path| path.is_file())
        .unwrap_or(true);
    if !audio_exists || !artwork_exists {
        return Ok(None);
    }

    let (bitrate_bps, audio_format) =
        read_audio_quality(&existing.audio_path, selected_track.duration_seconds)?;

    Ok(Some(PreparedPlaybackTrack {
        source: PlaybackSource::LocalFile(existing.audio_path.clone()),
        display_path: existing.audio_path,
        artwork_path: existing.artwork_path,
        bitrate_bps,
        audio_format,
        fully_cached: true,
        cache_monitor: None,
    }))
}

pub(super) fn cached_track(library: &Library, track: &TrackSummary) -> Result<Option<CachedTrack>> {
    let mut connection = library.open_connection()?;
    lookup_cached_track(&mut connection, track)
}

pub(super) fn all_cached_track_ids(library: &Library) -> Result<HashSet<String>> {
    let mut connection = library.open_connection()?;
    prune_missing_cached_entries(&mut connection)?;
    let mut statement = connection.prepare(
        r#"
        SELECT provider, track_id
        FROM cached_tracks
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        let provider: String = row.get(0)?;
        let track_id: String = row.get(1)?;
        Ok(format!("{provider}:{track_id}"))
    })?;

    let mut cached = HashSet::new();
    for row in rows {
        cached.insert(row?);
    }

    Ok(cached)
}

pub(super) fn cached_library_tracks(library: &Library) -> Result<Vec<CachedLibraryTrack>> {
    let mut connection = library.open_connection()?;
    prune_missing_cached_entries(&mut connection)?;
    backfill_missing_cached_durations(&mut connection)?;
    backfill_missing_cached_quality(&mut connection)?;
    let mut statement = connection.prepare(
        r#"
        SELECT
            provider,
            track_id,
            canonical_url,
            artist,
            album,
            title,
            collection_id,
            collection_title,
            collection_subtitle,
            track_position,
            duration_seconds,
            bitrate_bps,
            audio_format,
            artwork_path
        FROM cached_tracks
        ORDER BY artist COLLATE NOCASE, album COLLATE NOCASE, title COLLATE NOCASE
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        let provider: String = row.get(0)?;
        let track_id: String = row.get(1)?;
        let canonical_url: Option<String> = row.get(2)?;
        let artist: Option<String> = row.get(3)?;
        let album: Option<String> = row.get(4)?;
        let title: String = row.get(5)?;
        let collection_id: Option<String> = row.get(6)?;
        let collection_title: Option<String> = row.get(7)?;
        let collection_subtitle: Option<String> = row.get(8)?;
        let _track_position: Option<usize> = row.get(9)?;
        let duration_seconds: Option<u32> = row.get(10)?;
        let bitrate_bps: Option<u32> = row.get(11)?;
        let audio_format: Option<String> = row.get(12)?;
        let artwork_path: Option<String> = row.get(13)?;

        let provider_id = ProviderId::parse(&provider).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown provider '{provider}'").into(),
            )
        })?;

        Ok(CachedLibraryTrack {
            track: TrackSummary {
                reference: TrackRef::new(provider_id, track_id, canonical_url, Some(title.clone())),
                title,
                artist,
                album,
                collection_id,
                collection_title,
                collection_subtitle,
                duration_seconds,
                bitrate_bps,
                audio_format: audio_format.and_then(|value| parse_audio_format(&value)),
                artwork_url: artwork_path.clone(),
            },
            collection_provider: provider_id,
        })
    })?;

    let mut tracks = Vec::new();
    for row in rows {
        tracks.push(row?);
    }

    Ok(tracks)
}

pub(super) fn delete_collection_from_library(
    library: &Library,
    provider: ProviderId,
    collection_id: &str,
) -> Result<usize> {
    let connection = library.open_connection()?;
    let mut statement = connection.prepare(
        r#"
        SELECT audio_path, artwork_path
        FROM cached_tracks
        WHERE provider = ?1 AND collection_id = ?2
        "#,
    )?;
    let rows = statement.query_map(params![provider.as_str(), collection_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;

    let mut audio_paths = HashSet::new();
    let mut artwork_paths = HashSet::new();
    for row in rows {
        let (audio_path, artwork_path) = row?;
        audio_paths.insert(PathBuf::from(audio_path));
        if let Some(artwork_path) = artwork_path {
            artwork_paths.insert(PathBuf::from(artwork_path));
        }
    }
    drop(statement);

    let deleted_rows = connection.execute(
        r#"
        DELETE FROM cached_tracks
        WHERE provider = ?1 AND collection_id = ?2
        "#,
        params![provider.as_str(), collection_id],
    )?;

    for audio_path in &audio_paths {
        let _ = fs::remove_file(audio_path);
        prune_empty_parent_dirs(audio_path.parent(), &library.library_root);
    }

    for artwork_path in &artwork_paths {
        let still_referenced = connection
            .query_row(
                "SELECT 1 FROM cached_tracks WHERE artwork_path = ?1 LIMIT 1",
                params![artwork_path.to_string_lossy()],
                |_row| Ok(()),
            )
            .optional()?
            .is_some();
        if !still_referenced {
            let _ = fs::remove_file(artwork_path);
            prune_empty_parent_dirs(artwork_path.parent(), &library.library_root);
        }
    }

    Ok(deleted_rows)
}

pub(super) fn delete_track_from_library(
    library: &Library,
    provider: ProviderId,
    track_id: &str,
) -> Result<bool> {
    let connection = library.open_connection()?;
    let cached_paths = connection
        .query_row(
            r#"
            SELECT audio_path, artwork_path
            FROM cached_tracks
            WHERE provider = ?1 AND track_id = ?2
            "#,
            params![provider.as_str(), track_id],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    row.get::<_, Option<String>>(1)?.map(PathBuf::from),
                ))
            },
        )
        .optional()?;

    let Some((audio_path, artwork_path)) = cached_paths else {
        return Ok(false);
    };

    connection.execute(
        r#"
        DELETE FROM cached_tracks
        WHERE provider = ?1 AND track_id = ?2
        "#,
        params![provider.as_str(), track_id],
    )?;

    let _ = fs::remove_file(&audio_path);
    prune_empty_parent_dirs(audio_path.parent(), &library.library_root);

    if let Some(artwork_path) = artwork_path {
        let still_referenced = connection
            .query_row(
                "SELECT 1 FROM cached_tracks WHERE artwork_path = ?1 LIMIT 1",
                params![artwork_path.to_string_lossy()],
                |_row| Ok(()),
            )
            .optional()?
            .is_some();
        if !still_referenced {
            let _ = fs::remove_file(&artwork_path);
            prune_empty_parent_dirs(artwork_path.parent(), &library.library_root);
        }
    }

    Ok(true)
}

pub(super) fn ensure_collection_artwork_cached(
    library: &Library,
    provider: &dyn MusicProvider,
    collection: &CollectionSummary,
) -> Result<Option<PathBuf>> {
    let Some(artwork_url) = collection.artwork_url.as_deref() else {
        return Ok(None);
    };
    if !artwork_url.starts_with("http://") && !artwork_url.starts_with("https://") {
        return Ok(Some(PathBuf::from(artwork_url)));
    }
    let Some(request) = provider.get_artwork_request(artwork_url) else {
        return Ok(None);
    };

    let directory = library
        .library_root
        .join("artwork")
        .join(collection.reference.provider.as_str());
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "Failed to create artwork cache directory at {}",
            directory.display()
        )
    })?;

    let path = resolve_collection_artwork_path(&directory, collection, &request);
    if !path.is_file() {
        download_to_path(
            &request,
            &path,
            None,
            DownloadRetryPolicy::Bounded(DOWNLOAD_RETRY_LIMIT),
        )?;
    }

    Ok(Some(path))
}

fn lookup_cached_track(
    connection: &mut Connection,
    track: &TrackSummary,
) -> Result<Option<CachedTrack>> {
    let cached = connection
        .query_row(
            r#"
            SELECT audio_path, artwork_path
            FROM cached_tracks
            WHERE provider = ?1 AND track_id = ?2
            "#,
            params![track.reference.provider.as_str(), track.reference.id],
            |row| {
                let audio_path: String = row.get(0)?;
                let artwork_path: Option<String> = row.get(1)?;
                Ok(CachedTrack {
                    audio_path: PathBuf::from(audio_path),
                    artwork_path: artwork_path.map(PathBuf::from),
                })
            },
        )
        .optional()?;

    let Some(mut cached) = cached else {
        return Ok(None);
    };

    if !cached.audio_path.is_file() {
        delete_cached_track_row(
            connection,
            track.reference.provider.as_str(),
            &track.reference.id,
        )?;
        return Ok(None);
    }

    if cached
        .artwork_path
        .as_ref()
        .map(|path| !path.is_file())
        .unwrap_or(false)
    {
        clear_cached_track_artwork_path(
            connection,
            track.reference.provider.as_str(),
            &track.reference.id,
        )?;
        cached.artwork_path = None;
    }

    Ok(Some(cached))
}

fn prune_missing_cached_entries(connection: &mut Connection) -> Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT provider, track_id, audio_path, artwork_path
        FROM cached_tracks
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        let provider: String = row.get(0)?;
        let track_id: String = row.get(1)?;
        let audio_path: String = row.get(2)?;
        let artwork_path: Option<String> = row.get(3)?;
        Ok((provider, track_id, audio_path, artwork_path))
    })?;

    let mut missing_audio = Vec::new();
    let mut missing_artwork = Vec::new();

    for row in rows {
        let (provider, track_id, audio_path, artwork_path) = row?;
        if !Path::new(&audio_path).is_file() {
            missing_audio.push((provider, track_id));
            continue;
        }

        if artwork_path
            .as_deref()
            .map(Path::new)
            .map(|path| !path.is_file())
            .unwrap_or(false)
        {
            missing_artwork.push((provider, track_id));
        }
    }
    drop(statement);

    for (provider, track_id) in missing_audio {
        delete_cached_track_row(connection, &provider, &track_id)?;
    }

    for (provider, track_id) in missing_artwork {
        clear_cached_track_artwork_path(connection, &provider, &track_id)?;
    }

    Ok(())
}

fn delete_cached_track_row(
    connection: &mut Connection,
    provider: &str,
    track_id: &str,
) -> Result<()> {
    connection.execute(
        r#"
        DELETE FROM cached_tracks
        WHERE provider = ?1 AND track_id = ?2
        "#,
        params![provider, track_id],
    )?;

    Ok(())
}

fn prune_empty_parent_dirs(mut current: Option<&Path>, root: &Path) {
    while let Some(dir) = current {
        if dir == root {
            break;
        }
        match fs::remove_dir(dir) {
            Ok(()) => current = dir.parent(),
            Err(_) => break,
        }
    }
}

fn backfill_cached_track_position(
    connection: &mut Connection,
    track: &TrackSummary,
    track_position: Option<usize>,
) -> Result<()> {
    let Some(track_position) = track_position else {
        return Ok(());
    };

    connection.execute(
        r#"
        UPDATE cached_tracks
        SET track_position = ?3, updated_at = unixepoch()
        WHERE provider = ?1 AND track_id = ?2
        "#,
        params![
            track.reference.provider.as_str(),
            track.reference.id,
            track_position
        ],
    )?;

    Ok(())
}

fn resolved_track_for_cache(
    selected_track: &TrackSummary,
    resolved_track: &TrackSummary,
) -> TrackSummary {
    TrackSummary {
        reference: selected_track.reference.clone(),
        title: selected_track.title.clone(),
        artist: resolved_track
            .artist
            .clone()
            .or_else(|| selected_track.artist.clone()),
        album: resolved_track
            .album
            .clone()
            .or_else(|| selected_track.album.clone()),
        collection_id: resolved_track
            .collection_id
            .clone()
            .or_else(|| selected_track.collection_id.clone()),
        collection_title: resolved_track
            .collection_title
            .clone()
            .or_else(|| selected_track.collection_title.clone()),
        collection_subtitle: resolved_track
            .collection_subtitle
            .clone()
            .or_else(|| selected_track.collection_subtitle.clone()),
        duration_seconds: resolved_track
            .duration_seconds
            .or(selected_track.duration_seconds),
        bitrate_bps: resolved_track.bitrate_bps.or(selected_track.bitrate_bps),
        audio_format: resolved_track
            .audio_format
            .clone()
            .or_else(|| selected_track.audio_format.clone()),
        artwork_url: resolved_track
            .artwork_url
            .clone()
            .or_else(|| selected_track.artwork_url.clone()),
    }
}

pub(super) fn sync_cached_metadata_for_track_list(
    connection: &Connection,
    track_list: &TrackList,
) -> Result<()> {
    if !matches!(track_list.collection.reference.kind, CollectionKind::Album) {
        return Ok(());
    }

    for (track_position, track) in track_list.tracks.iter().enumerate() {
        connection.execute(
            r#"
            UPDATE cached_tracks
            SET canonical_url = ?3,
                artist = ?4,
                album = ?5,
                title = ?6,
                collection_id = ?7,
                collection_title = ?8,
                collection_subtitle = ?9,
                track_position = ?10,
                duration_seconds = COALESCE(?11, duration_seconds),
                updated_at = unixepoch()
            WHERE provider = ?1 AND track_id = ?2
            "#,
            params![
                track.reference.provider.as_str(),
                track.reference.id,
                track.reference.canonical_url,
                track.artist,
                track.album,
                track.title,
                track_list.collection.reference.id,
                track_list.collection.title,
                track_list.collection.subtitle,
                track_position,
                track.duration_seconds,
            ],
        )?;
    }

    Ok(())
}

fn backfill_missing_cached_durations(connection: &mut Connection) -> Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT provider, track_id, audio_path
        FROM cached_tracks
        WHERE duration_seconds IS NULL
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        let provider: String = row.get(0)?;
        let track_id: String = row.get(1)?;
        let audio_path: String = row.get(2)?;
        Ok((provider, track_id, audio_path))
    })?;

    let mut duration_updates = Vec::new();
    for row in rows {
        let (provider, track_id, audio_path) = row?;
        let Some(duration_seconds) = read_audio_duration_seconds(Path::new(&audio_path))? else {
            continue;
        };
        duration_updates.push((provider, track_id, duration_seconds));
    }
    drop(statement);

    for (provider, track_id, duration_seconds) in duration_updates {
        connection.execute(
            r#"
            UPDATE cached_tracks
            SET duration_seconds = ?3, updated_at = unixepoch()
            WHERE provider = ?1 AND track_id = ?2
            "#,
            params![provider, track_id, duration_seconds],
        )?;
    }

    Ok(())
}

fn backfill_missing_cached_quality(connection: &mut Connection) -> Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT provider, track_id, audio_path, duration_seconds
        FROM cached_tracks
        WHERE bitrate_bps IS NULL OR audio_format IS NULL
        "#,
    )?;

    let rows = statement.query_map([], |row| {
        let provider: String = row.get(0)?;
        let track_id: String = row.get(1)?;
        let audio_path: String = row.get(2)?;
        let duration_seconds: Option<u32> = row.get(3)?;
        Ok((provider, track_id, audio_path, duration_seconds))
    })?;

    let mut updates = Vec::new();
    for row in rows {
        let (provider, track_id, audio_path, duration_seconds) = row?;
        let path = Path::new(&audio_path);
        let _ = duration_seconds;
        updates.push((
            provider,
            track_id,
            infer_audio_bitrate_bps(path)?,
            infer_audio_format(path),
        ));
    }
    drop(statement);

    for (provider, track_id, bitrate_bps, audio_format) in updates {
        connection.execute(
            r#"
            UPDATE cached_tracks
            SET bitrate_bps = COALESCE(?3, bitrate_bps),
                audio_format = COALESCE(?4, audio_format),
                updated_at = unixepoch()
            WHERE provider = ?1 AND track_id = ?2
            "#,
            params![
                provider,
                track_id,
                bitrate_bps,
                audio_format.as_ref().map(serialize_audio_format),
            ],
        )?;
    }

    Ok(())
}

fn read_audio_duration_seconds(path: &Path) -> Result<Option<u32>> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open cached audio file {}", path.display()))?;
    let decoder = Decoder::try_from(file)
        .with_context(|| format!("Failed to decode cached audio file {}", path.display()))?;

    Ok(decoder
        .total_duration()
        .map(|duration| duration.as_secs().min(u32::MAX as u64) as u32))
}

fn read_audio_quality(
    path: &Path,
    _duration_seconds: Option<u32>,
) -> Result<(Option<u32>, Option<AudioFormat>)> {
    Ok((infer_audio_bitrate_bps(path)?, infer_audio_format(path)))
}

fn infer_audio_bitrate_bps(path: &Path) -> Result<Option<u32>> {
    let probe = Probe::open(path)
        .with_context(|| format!("Failed to inspect cached audio file {}", path.display()))?;
    let parse_options = ParseOptions::new()
        .parsing_mode(ParsingMode::Relaxed)
        .read_cover_art(false);
    let tagged_file = match probe.options(parse_options).read() {
        Ok(tagged_file) => tagged_file,
        Err(_) => return Ok(None),
    };
    let properties = tagged_file.properties();
    Ok(bitrate_bps_from_properties(
        properties.audio_bitrate(),
        properties.overall_bitrate(),
    ))
}

fn infer_audio_format(path: &Path) -> Option<AudioFormat> {
    let extension = path.extension()?.to_str()?;
    parse_audio_format(extension)
}

fn parse_audio_format(value: &str) -> Option<AudioFormat> {
    match value.to_ascii_lowercase().as_str() {
        "mp3" => Some(AudioFormat::Mp3),
        "flac" => Some(AudioFormat::Flac),
        "opus" => Some(AudioFormat::Opus),
        "aac" => Some(AudioFormat::Aac),
        "m4a" => Some(AudioFormat::M4a),
        "" => None,
        other => Some(AudioFormat::Unknown(other.to_ascii_uppercase())),
    }
}

fn serialize_audio_format(value: &AudioFormat) -> String {
    match value {
        AudioFormat::Mp3 => "mp3".to_string(),
        AudioFormat::Flac => "flac".to_string(),
        AudioFormat::Opus => "opus".to_string(),
        AudioFormat::Aac => "aac".to_string(),
        AudioFormat::M4a => "m4a".to_string(),
        AudioFormat::Unknown(label) => label.to_ascii_lowercase(),
    }
}

fn clear_cached_track_artwork_path(
    connection: &mut Connection,
    provider: &str,
    track_id: &str,
) -> Result<()> {
    connection.execute(
        r#"
        UPDATE cached_tracks
        SET artwork_path = NULL, updated_at = unixepoch()
        WHERE provider = ?1 AND track_id = ?2
        "#,
        params![provider, track_id],
    )?;

    Ok(())
}

fn album_directory(library: &Library, track: &TrackSummary) -> PathBuf {
    library
        .library_root
        .join(track.reference.provider.as_str())
        .join(sanitize_path_component(
            track
                .collection_subtitle
                .as_deref()
                .or(track.artist.as_deref())
                .unwrap_or("Unknown Artist"),
        ))
        .join(sanitize_path_component(
            track
                .collection_title
                .as_deref()
                .or(track.album.as_deref())
                .unwrap_or("Unknown Album"),
        ))
}

fn resolve_audio_path(
    connection: &Connection,
    album_dir: &Path,
    track: &TrackSummary,
    request: &DownloadRequest,
) -> Result<PathBuf> {
    let extension = extension_for_download(request).unwrap_or("mp3");
    let base_name = sanitize_path_component(&track.title);
    let preferred = album_dir.join(format!("{base_name}.{extension}"));

    if !is_path_claimed_by_other_track(connection, &preferred, track)? {
        return Ok(preferred);
    }

    let suffix = short_stable_suffix(&(track.reference.provider.as_str(), &track.reference.id));
    Ok(album_dir.join(format!("{base_name} [{suffix}].{extension}")))
}

fn resolve_artwork_path(album_dir: &Path, request: &DownloadRequest) -> PathBuf {
    let extension = extension_for_download(request).unwrap_or("jpg");
    album_dir.join(format!("cover.{extension}"))
}

fn ensure_track_artwork_cached(
    provider: &dyn MusicProvider,
    selected_track: &TrackSummary,
    album_dir: &Path,
) -> Result<Option<PathBuf>> {
    match selected_track
        .artwork_url
        .as_deref()
        .and_then(|url| provider.get_artwork_request(url))
    {
        Some(artwork_request) => {
            let path = resolve_artwork_path(album_dir, &artwork_request);
            if !path.is_file() {
                download_to_path(
                    &artwork_request,
                    &path,
                    None,
                    DownloadRetryPolicy::Bounded(DOWNLOAD_RETRY_LIMIT),
                )?;
            }
            Ok(Some(path))
        }
        None => Ok(None),
    }
}

fn resolve_collection_artwork_path(
    artwork_dir: &Path,
    collection: &CollectionSummary,
    request: &DownloadRequest,
) -> PathBuf {
    let extension = extension_for_download(request).unwrap_or("jpg");
    let base_name = sanitize_path_component(&collection.reference.id);
    let suffix = short_stable_suffix(&(
        collection.reference.provider.as_str(),
        &collection.reference.id,
        &request.url,
    ));
    artwork_dir.join(format!("{base_name}-{suffix}.{extension}"))
}

fn temporary_download_path(destination: &Path) -> PathBuf {
    destination.with_extension(format!(
        "{}.part",
        destination
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("download")
    ))
}

fn complete_progressive_download(
    library: &Library,
    track: &TrackSummary,
    track_position: Option<usize>,
    stream_url: &str,
    artwork_path: Option<PathBuf>,
    request: DownloadRequest,
    temp_path: PathBuf,
    destination: PathBuf,
    download: ProgressiveDownload,
) -> Result<()> {
    with_download_path_lock(&destination, || {
        if cached_audio_file_is_valid(&destination, track.duration_seconds) {
            let mut connection = library.open_connection()?;
            upsert_cached_track(
                &mut connection,
                track,
                track_position,
                stream_url,
                &destination,
                artwork_path.as_deref(),
            )?;
            download.finish(downloaded_file_len(&destination)?);
            return Ok(());
        }

        let download_result = match download_to_partial_path(
            &request,
            &temp_path,
            Some(&download),
            DownloadRetryPolicy::Infinite,
        ) {
            Ok(download_result) => download_result,
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                return Err(error);
            }
        };
        if let Err(error) =
            validate_downloaded_audio_file(&temp_path, &download_result, track.duration_seconds)
        {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
        fs::rename(&temp_path, &destination).with_context(|| {
            format!(
                "Failed to move downloaded file from {} to {}",
                temp_path.display(),
                destination.display()
            )
        })?;

        let mut connection = library.open_connection()?;
        upsert_cached_track(
            &mut connection,
            track,
            track_position,
            stream_url,
            &destination,
            artwork_path.as_deref(),
        )?;

        download.finish(download_result.total_bytes);

        Ok(())
    })
}

fn with_download_path_lock<T>(destination: &Path, action: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = download_path_lock(destination);
    let _guard = lock.lock().expect("download path lock poisoned");
    action()
}

fn download_path_lock(destination: &Path) -> Arc<Mutex<()>> {
    static DOWNLOAD_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

    let locks = DOWNLOAD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().expect("download lock registry poisoned");
    locks
        .entry(destination.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn downloaded_file_len(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)
        .with_context(|| format!("Failed to read downloaded file metadata {}", path.display()))?
        .len())
}

fn upsert_cached_track(
    connection: &mut Connection,
    track: &TrackSummary,
    track_position: Option<usize>,
    stream_url: &str,
    audio_path: &Path,
    artwork_path: Option<&Path>,
) -> Result<()> {
    connection.execute(
        r#"
        INSERT INTO cached_tracks (
            provider,
            track_id,
            canonical_url,
            artist,
            album,
            title,
            collection_id,
            collection_title,
            collection_subtitle,
            track_position,
            duration_seconds,
            bitrate_bps,
            audio_format,
            stream_url,
            audio_path,
            artwork_path,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, unixepoch())
        ON CONFLICT(provider, track_id) DO UPDATE SET
            canonical_url = excluded.canonical_url,
            artist = excluded.artist,
            album = excluded.album,
            title = excluded.title,
            collection_id = COALESCE(excluded.collection_id, collection_id),
            collection_title = COALESCE(excluded.collection_title, collection_title),
            collection_subtitle = COALESCE(excluded.collection_subtitle, collection_subtitle),
            track_position = COALESCE(excluded.track_position, track_position),
            duration_seconds = COALESCE(excluded.duration_seconds, duration_seconds),
            bitrate_bps = COALESCE(excluded.bitrate_bps, bitrate_bps),
            audio_format = COALESCE(excluded.audio_format, audio_format),
            stream_url = excluded.stream_url,
            audio_path = excluded.audio_path,
            artwork_path = excluded.artwork_path,
            updated_at = unixepoch()
        "#,
        params![
            track.reference.provider.as_str(),
            track.reference.id,
            track.reference.canonical_url,
            track.artist,
            track.album,
            track.title,
            track.collection_id,
            track.collection_title,
            track.collection_subtitle,
            track_position,
            track.duration_seconds,
            track.bitrate_bps,
            track.audio_format.as_ref().map(serialize_audio_format),
            stream_url,
            audio_path.to_string_lossy(),
            artwork_path.map(|path| path.to_string_lossy().into_owned()),
        ],
    )?;

    super::entities::sync_cached_track(connection, track, track_position, artwork_path)?;

    Ok(())
}

fn short_stable_suffix(value: &impl Hash) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

fn is_path_claimed_by_other_track(
    connection: &Connection,
    path: &Path,
    track: &TrackSummary,
) -> Result<bool> {
    let owner = connection
        .query_row(
            "SELECT provider, track_id FROM cached_tracks WHERE audio_path = ?1",
            params![path.to_string_lossy()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    match owner {
        Some((provider, track_id)) => {
            Ok(provider != track.reference.provider.as_str() || track_id != track.reference.id)
        }
        None => Ok(path.exists()),
    }
}

fn extension_for_download(request: &DownloadRequest) -> Option<&'static str> {
    request
        .mime_type
        .as_deref()
        .and_then(extension_for_mime_type)
        .or_else(|| extension_from_url(&request.url))
}

fn extension_for_mime_type(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/aac" | "audio/x-aac" => Some("aac"),
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/svg+xml" => Some("svg"),
        "image/bmp" => Some("bmp"),
        "image/tiff" | "image/tif" => Some("tiff"),
        _ => None,
    }
}

fn extension_from_url(url: &str) -> Option<&'static str> {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();

    if path.ends_with(".mp3") {
        Some("mp3")
    } else if path.ends_with(".aac") || path.ends_with(".m3u8") {
        Some("aac")
    } else if path.ends_with(".png") {
        Some("png")
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        Some("jpg")
    } else if path.ends_with(".webp") {
        Some("webp")
    } else if path.ends_with(".gif") {
        Some("gif")
    } else if path.ends_with(".svg") {
        Some("svg")
    } else if path.ends_with(".bmp") {
        Some("bmp")
    } else if path.ends_with(".tif") || path.ends_with(".tiff") {
        Some("tiff")
    } else {
        None
    }
}

fn download_to_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
    retry_policy: DownloadRetryPolicy,
) -> Result<()> {
    let parent = destination.parent().with_context(|| {
        format!(
            "Destination {} has no parent directory",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create parent directory {}", parent.display()))?;

    with_download_path_lock(destination, || {
        if destination.is_file() {
            if let Some(progress) = progress {
                progress.finish(downloaded_file_len(destination)?);
            }
            return Ok(());
        }

        let temp_path = temporary_download_path(destination);
        let download_result =
            match download_to_partial_path(request, &temp_path, progress, retry_policy) {
                Ok(download_result) => download_result,
                Err(error) => {
                    let _ = fs::remove_file(&temp_path);
                    return Err(error);
                }
            };

        fs::rename(&temp_path, destination).with_context(|| {
            format!(
                "Failed to move downloaded file from {} to {}",
                temp_path.display(),
                destination.display()
            )
        })?;

        if let Some(progress) = progress {
            progress.finish(download_result.total_bytes);
        }

        Ok(())
    })
}

fn download_audio_to_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
    expected_duration_seconds: Option<u32>,
    retry_policy: DownloadRetryPolicy,
) -> Result<()> {
    let parent = destination.parent().with_context(|| {
        format!(
            "Destination {} has no parent directory",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create parent directory {}", parent.display()))?;

    with_download_path_lock(destination, || {
        if cached_audio_file_is_valid(destination, expected_duration_seconds) {
            if let Some(progress) = progress {
                progress.finish(downloaded_file_len(destination)?);
            }
            return Ok(());
        }

        let temp_path = temporary_download_path(destination);
        let download_result =
            match download_to_partial_path(request, &temp_path, progress, retry_policy) {
                Ok(download_result) => download_result,
                Err(error) => {
                    let _ = fs::remove_file(&temp_path);
                    return Err(error);
                }
            };

        if let Err(error) =
            validate_downloaded_audio_file(&temp_path, &download_result, expected_duration_seconds)
        {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }

        fs::rename(&temp_path, destination).with_context(|| {
            format!(
                "Failed to move downloaded file from {} to {}",
                temp_path.display(),
                destination.display()
            )
        })?;

        if let Some(progress) = progress {
            progress.finish(download_result.total_bytes);
        }

        Ok(())
    })
}

#[derive(Clone, Copy, Debug)]
struct DownloadResult {
    total_bytes: u64,
    expected_total_bytes: Option<u64>,
}

fn download_to_partial_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
    retry_policy: DownloadRetryPolicy,
) -> Result<DownloadResult> {
    let mut attempt = 0usize;

    loop {
        if progress.is_some_and(ProgressiveDownload::is_cancelled) {
            anyhow::bail!("Download cancelled.");
        }

        let attempt_result = if is_hls_playlist_url(&request.url) {
            download_hls_audio_to_partial_path(request, destination, progress)
        } else {
            let existing_len = resumable_download_len(destination, request.supports_byte_ranges)?;
            download_attempt(request, destination, progress, existing_len)
        }
        .with_context(|| {
            format!(
                "Failed to download {} (attempt {}/{})",
                request.url,
                attempt + 1,
                DOWNLOAD_RETRY_LIMIT + 1
            )
        });

        match attempt_result {
            Ok(result) => return Ok(result),
            Err(error)
                if should_retry_partial_download(&error)
                    && retry_policy.allows_retry(attempt)
                    && !progress.is_some_and(ProgressiveDownload::is_cancelled) =>
            {
                if let Some(progress) = progress {
                    progress.set_retrying(true);
                }
                let delay = download_retry_delay(attempt);
                if should_log_download_retry(attempt) {
                    eprintln!(
                        "download attempt {} failed for '{}': {error:#}; retrying in {}ms",
                        attempt + 1,
                        request.url,
                        delay.as_millis()
                    );
                }
                thread::sleep(delay);
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn download_attempt(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
    existing_len: u64,
) -> Result<DownloadResult> {
    if let Some(progress) = progress {
        progress.set_retrying(false);
    }
    let (response, resume_from) = open_download_response(request, existing_len)?;
    let expected_total = expected_total_bytes(&response, resume_from);
    if let Some(progress) = progress {
        progress.set_total_bytes(expected_total);
        if resume_from > 0 {
            progress.report_progress(resume_from);
        }
    }

    let mut reader = response.into_reader();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume_from > 0)
        .truncate(resume_from == 0)
        .open(destination)
        .with_context(|| format!("Failed to open temporary file {}", destination.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut total_bytes = resume_from;

    loop {
        if progress.is_some_and(ProgressiveDownload::is_cancelled) {
            anyhow::bail!("Download cancelled.");
        }

        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])?;
        total_bytes += bytes_read as u64;
        if let Some(progress) = progress {
            progress.report_progress(total_bytes);
        }
    }
    file.flush()?;

    Ok(DownloadResult {
        total_bytes,
        expected_total_bytes: expected_total,
    })
}

#[derive(Clone, Debug)]
struct HlsVariant {
    uri: String,
    bandwidth: Option<u64>,
}

fn download_hls_audio_to_partial_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
) -> Result<DownloadResult> {
    let master_playlist = fetch_text_response(request)?;
    let media_playlist_url = resolve_hls_media_playlist_url(request, &master_playlist)?;
    let media_playlist = if media_playlist_url == request.url {
        master_playlist
    } else {
        fetch_text_response(&DownloadRequest {
            url: media_playlist_url.clone(),
            headers: request.headers.clone(),
            mime_type: None,
            supports_byte_ranges: false,
        })?
    };
    let segment_urls = parse_hls_media_segments(&media_playlist)
        .into_iter()
        .map(|segment| resolve_hls_url(&media_playlist_url, &segment))
        .collect::<Result<Vec<_>>>()?;

    if segment_urls.is_empty() {
        anyhow::bail!("HLS media playlist did not contain any audio segments");
    }

    if let Some(progress) = progress {
        progress.set_total_bytes(None);
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(destination)
        .with_context(|| format!("Failed to open temporary file {}", destination.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut total_bytes = 0u64;

    for segment_url in segment_urls {
        if progress.is_some_and(ProgressiveDownload::is_cancelled) {
            anyhow::bail!("Download cancelled.");
        }

        let segment_request = DownloadRequest {
            url: segment_url,
            headers: request.headers.clone(),
            mime_type: Some("audio/aac".to_string()),
            supports_byte_ranges: false,
        };
        let (response, _) = open_download_response(&segment_request, 0)?;
        let mut reader = response.into_reader();

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            file.write_all(&buffer[..bytes_read])?;
            total_bytes += bytes_read as u64;
            if let Some(progress) = progress {
                progress.report_progress(total_bytes);
            }
        }
    }

    file.flush()?;

    Ok(DownloadResult {
        total_bytes,
        expected_total_bytes: None,
    })
}

fn is_hls_playlist_url(url: &str) -> bool {
    url.split('?')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase()
        .ends_with(".m3u8")
}

fn resolve_hls_media_playlist_url(
    request: &DownloadRequest,
    master_playlist: &str,
) -> Result<String> {
    let variants = parse_hls_variants(master_playlist);
    let Some(variant) = choose_hls_variant(&variants) else {
        return Ok(request.url.clone());
    };

    resolve_hls_url(&request.url, &variant.uri)
}

fn parse_hls_variants(playlist: &str) -> Vec<HlsVariant> {
    if !playlist.contains("#EXT-X-STREAM-INF") {
        return Vec::new();
    }

    let mut variants = Vec::new();
    let mut pending_bandwidth = None;

    for raw_line in playlist.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_bandwidth = parse_hls_bandwidth(attributes);
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        variants.push(HlsVariant {
            uri: line.to_string(),
            bandwidth: pending_bandwidth.take(),
        });
    }

    variants
}

fn choose_hls_variant(variants: &[HlsVariant]) -> Option<&HlsVariant> {
    variants
        .iter()
        .max_by_key(|variant| variant.bandwidth.unwrap_or(0))
}

fn parse_hls_bandwidth(attributes: &str) -> Option<u64> {
    attributes
        .split(',')
        .find_map(|attribute| attribute.trim().strip_prefix("BANDWIDTH="))
        .and_then(|value| value.parse::<u64>().ok())
}

fn parse_hls_media_segments(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn resolve_hls_url(base_url: &str, uri: &str) -> Result<String> {
    if uri.contains("://") {
        return Ok(uri.to_string());
    }

    let base = Url::parse(base_url).with_context(|| format!("Invalid HLS base URL {base_url}"))?;
    let resolved = base
        .join(uri)
        .with_context(|| format!("Failed to resolve HLS URI {uri} against {base_url}"))?;
    Ok(resolved.into())
}

fn build_download_request(request: &DownloadRequest) -> ureq::Request {
    let mut response = network_agent().get(&request.url);
    for header in &request.headers {
        response = response.set(&header.name, &header.value);
    }
    response
}

fn fetch_text_response(request: &DownloadRequest) -> Result<String> {
    build_download_request(request)
        .call()
        .with_context(|| format!("Failed to download {}", request.url))?
        .into_string()
        .with_context(|| format!("Failed to read response body for {}", request.url))
}

fn open_download_response(
    request: &DownloadRequest,
    existing_len: u64,
) -> Result<(ureq::Response, u64)> {
    let mut response = build_download_request(request);

    if request.supports_byte_ranges && existing_len > 0 {
        response = response.set("Range", &format!("bytes={existing_len}-"));
    }

    let response = response
        .call()
        .with_context(|| format!("Failed to download {}", request.url))?;
    let status = response.status();

    if existing_len > 0 {
        if request.supports_byte_ranges {
            if status != 206 {
                anyhow::bail!(
                    "Server did not honor byte-range resume request for {} (status {status})",
                    request.url
                );
            }
            return Ok((response, existing_len));
        }

        return Ok((response, 0));
    }

    Ok((response, 0))
}

fn resumable_download_len(destination: &Path, supports_byte_ranges: bool) -> Result<u64> {
    if !supports_byte_ranges || !destination.is_file() {
        return Ok(0);
    }

    Ok(fs::metadata(destination)
        .with_context(|| format!("Failed to inspect temporary file {}", destination.display()))?
        .len())
}

fn expected_total_bytes(response: &ureq::Response, resume_from: u64) -> Option<u64> {
    parse_content_range_total(response.header("Content-Range")).or_else(|| {
        response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|len| len.saturating_add(resume_from))
    })
}

fn parse_content_range_total(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|value| value.rsplit('/').next())
        .filter(|total| *total != "*")
        .and_then(|total| total.parse::<u64>().ok())
}

fn should_retry_partial_download(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| is_retryable_download_error_message(cause))
}

fn is_retryable_download_error_message(message: &dyn std::fmt::Display) -> bool {
    let text = message.to_string().to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "connection reset",
        "connection aborted",
        "broken pipe",
        "unexpected eof",
        "temporarily unavailable",
        "network is unreachable",
        "connection closed before message completed",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn download_retry_delay(attempt: usize) -> Duration {
    let exponent = attempt.min(4) as u32;
    Duration::from_millis(DOWNLOAD_RETRY_BASE_DELAY_MS.saturating_mul(1u64 << exponent))
}

fn should_log_download_retry(attempt: usize) -> bool {
    attempt < 2 || (attempt + 1).is_multiple_of(10)
}

impl DownloadRetryPolicy {
    fn allows_retry(self, attempt: usize) -> bool {
        match self {
            Self::Bounded(limit) => attempt < limit,
            Self::Infinite => true,
        }
    }
}

fn validate_downloaded_audio_file(
    path: &Path,
    download_result: &DownloadResult,
    expected_duration_seconds: Option<u32>,
) -> Result<()> {
    if let Some(expected_total_bytes) = download_result.expected_total_bytes
        && download_result.total_bytes != expected_total_bytes
    {
        anyhow::bail!(
            "Downloaded audio was truncated: expected {expected_total_bytes} bytes, received {}",
            download_result.total_bytes
        );
    }

    validate_audio_duration(path, expected_duration_seconds)
}

fn validate_audio_duration(path: &Path, expected_duration_seconds: Option<u32>) -> Result<()> {
    let Some(expected_duration_seconds) = expected_duration_seconds.filter(|seconds| *seconds > 0)
    else {
        return Ok(());
    };

    let actual_duration_seconds = read_audio_duration_seconds(path)?.with_context(|| {
        format!(
            "Downloaded audio file {} did not expose a readable duration",
            path.display()
        )
    })?;
    let tolerance = audio_duration_tolerance_seconds(expected_duration_seconds);
    if actual_duration_seconds.saturating_add(tolerance) < expected_duration_seconds {
        anyhow::bail!(
            "Downloaded audio is shorter than expected: expected about {expected_duration_seconds}s, got {actual_duration_seconds}s"
        );
    }

    Ok(())
}

fn audio_duration_tolerance_seconds(expected_duration_seconds: u32) -> u32 {
    let percent_tolerance = ((u64::from(expected_duration_seconds)
        * u64::from(AUDIO_DURATION_TOLERANCE_PERCENT))
    .div_ceil(100)) as u32;
    AUDIO_DURATION_TOLERANCE_SECONDS.max(percent_tolerance)
}

fn cached_audio_file_is_valid(path: &Path, expected_duration_seconds: Option<u32>) -> bool {
    if !path.is_file() {
        return false;
    }

    match validate_audio_duration(path, expected_duration_seconds) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "discarding invalid cached audio {}: {error}",
                path.display()
            );
            let _ = fs::remove_file(path);
            false
        }
    }
}

fn bitrate_bps_from_properties(
    audio_bitrate_kbps: Option<u32>,
    overall_bitrate_kbps: Option<u32>,
) -> Option<u32> {
    audio_bitrate_kbps
        .filter(|kbps| *kbps > 0)
        .or_else(|| overall_bitrate_kbps.filter(|kbps| *kbps > 0))
        .map(|kbps| kbps.saturating_mul(1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_path_components() {
        assert_eq!(
            sanitize_path_component("Creator/Name: Collection?"),
            "Creator_Name_ Collection_"
        );
        assert_eq!(sanitize_path_component("   "), "Untitled");
    }

    #[test]
    fn derives_extensions() {
        assert_eq!(extension_from_url("https://example.com/a.mp3"), Some("mp3"));
        assert_eq!(
            extension_from_url("https://example.com/master/playlist.m3u8"),
            Some("aac")
        );
        assert_eq!(extension_for_mime_type("audio/aac"), Some("aac"));
        assert_eq!(extension_for_mime_type("image/jpeg"), Some("jpg"));
    }

    #[test]
    fn parses_hls_variants_and_segments() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=64000\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=128000\nhigh.m3u8\n";
        let variants = parse_hls_variants(master);

        assert_eq!(variants.len(), 2);
        assert_eq!(
            choose_hls_variant(&variants).map(|variant| variant.uri.as_str()),
            Some("high.m3u8")
        );

        let media = "#EXTM3U\n#EXTINF:10,\nsegment0.aac\n#EXTINF:10,\nsegment1.aac\n";
        assert_eq!(
            parse_hls_media_segments(media),
            vec!["segment0.aac".to_string(), "segment1.aac".to_string()]
        );
    }

    #[test]
    fn bitrate_uses_audio_bitrate_before_overall_bitrate() {
        assert_eq!(
            bitrate_bps_from_properties(Some(320), Some(355)),
            Some(320_000)
        );
        assert_eq!(bitrate_bps_from_properties(None, Some(192)), Some(192_000));
        assert_eq!(
            bitrate_bps_from_properties(Some(0), Some(192)),
            Some(192_000)
        );
        assert_eq!(bitrate_bps_from_properties(None, None), None);
    }

    #[test]
    fn parses_content_range_totals() {
        assert_eq!(
            parse_content_range_total(Some("bytes 262144-524287/1048576")),
            Some(1_048_576)
        );
        assert_eq!(parse_content_range_total(Some("bytes 0-10/*")), None);
        assert_eq!(parse_content_range_total(None), None);
    }

    #[test]
    fn classifies_transient_download_errors_as_retryable() {
        let error = anyhow::anyhow!("timed out reading response");
        let fatal = anyhow::anyhow!("404 Not Found");

        assert!(should_retry_partial_download(&error));
        assert!(!should_retry_partial_download(&fatal));
    }

    #[test]
    fn download_retry_delay_grows_with_backoff() {
        assert_eq!(download_retry_delay(0), Duration::from_millis(750));
        assert_eq!(download_retry_delay(1), Duration::from_millis(1_500));
        assert_eq!(download_retry_delay(4), Duration::from_millis(12_000));
        assert_eq!(download_retry_delay(8), Duration::from_millis(12_000));
    }
}
