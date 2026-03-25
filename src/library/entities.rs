use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::provider::{CollectionKind, CollectionRef, CollectionSummary, TrackList, TrackSummary};

pub(super) const LIKED_PLAYLIST_ID: &str = "liked-tracks";
const LIKED_PLAYLIST_TITLE: &str = "Liked Tracks";
const LIKED_PLAYLIST_SUBTITLE: &str = "System playlist";
const SYSTEM_PLAYLIST_MEMBERSHIP_SOURCE: &str = "system_playlist";

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum EntityKind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl EntityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS library_entities (
            entity_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            provider TEXT,
            title TEXT NOT NULL,
            subtitle TEXT,
            canonical_url TEXT,
            artwork_url TEXT,
            duration_seconds INTEGER,
            bitrate_bps INTEGER,
            audio_format TEXT,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS library_entity_aliases (
            entity_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            entity_kind TEXT NOT NULL,
            remote_id TEXT NOT NULL,
            canonical_url TEXT,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
            PRIMARY KEY(provider, entity_kind, remote_id)
        );
        CREATE INDEX IF NOT EXISTS idx_library_entity_aliases_entity_id
            ON library_entity_aliases(entity_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_library_entity_aliases_canonical_url
            ON library_entity_aliases(provider, entity_kind, canonical_url)
            WHERE canonical_url IS NOT NULL;
        CREATE TABLE IF NOT EXISTS library_collection_tracks (
            collection_entity_id TEXT NOT NULL,
            track_entity_id TEXT NOT NULL,
            membership_source TEXT NOT NULL,
            position INTEGER,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
            PRIMARY KEY(collection_entity_id, track_entity_id, membership_source)
        );
        CREATE INDEX IF NOT EXISTS idx_library_collection_tracks_position
            ON library_collection_tracks(collection_entity_id, membership_source, position);
        "#,
    )?;

    Ok(())
}

pub(super) fn sync_collection_track_list(
    connection: &Connection,
    track_list: &TrackList,
) -> Result<()> {
    let collection_kind = match track_list.collection.reference.kind {
        CollectionKind::Album => EntityKind::Album,
        CollectionKind::Playlist => EntityKind::Playlist,
    };
    let collection_entity_id =
        entity_id_for_collection(collection_kind, &track_list.collection.reference);
    upsert_collection_entity(connection, collection_kind, &track_list.collection)?;

    connection.execute(
        r#"
        DELETE FROM library_collection_tracks
        WHERE collection_entity_id = ?1 AND membership_source = 'provider_snapshot'
        "#,
        params![collection_entity_id],
    )?;

    for (index, track) in track_list.tracks.iter().enumerate() {
        let track_entity_id = entity_id_for_track(track);
        upsert_track_entity(connection, track, track.artwork_url.as_deref())?;
        connection.execute(
            r#"
            INSERT INTO library_collection_tracks (
                collection_entity_id,
                track_entity_id,
                membership_source,
                position,
                updated_at
            ) VALUES (?1, ?2, 'provider_snapshot', ?3, unixepoch())
            ON CONFLICT(collection_entity_id, track_entity_id, membership_source) DO UPDATE SET
                position = excluded.position,
                updated_at = unixepoch()
            "#,
            params![collection_entity_id, track_entity_id, index],
        )?;
    }

    Ok(())
}

pub(super) fn sync_cached_track(
    connection: &Connection,
    track: &TrackSummary,
    track_position: Option<usize>,
    artwork_path: Option<&Path>,
) -> Result<()> {
    let artwork = artwork_path
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| track.artwork_url.clone());
    upsert_track_entity(connection, track, artwork.as_deref())?;

    let Some(collection_summary) = inferred_collection_summary(track, artwork.as_deref()) else {
        return Ok(());
    };

    let collection_entity_id =
        entity_id_for_collection(EntityKind::Album, &collection_summary.reference);
    upsert_collection_entity(connection, EntityKind::Album, &collection_summary)?;
    let track_entity_id = entity_id_for_track(track);

    connection.execute(
        r#"
        INSERT INTO library_collection_tracks (
            collection_entity_id,
            track_entity_id,
            membership_source,
            position,
            updated_at
        ) VALUES (?1, ?2, 'cached_track', ?3, unixepoch())
        ON CONFLICT(collection_entity_id, track_entity_id, membership_source) DO UPDATE SET
            position = COALESCE(excluded.position, position),
            updated_at = unixepoch()
        "#,
        params![collection_entity_id, track_entity_id, track_position],
    )?;

    Ok(())
}

pub(super) fn album_track_lists(connection: &Connection) -> Result<Vec<TrackList>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            album.entity_id,
            album.provider,
            album.title,
            album.subtitle,
            album.artwork_url,
            album_alias.remote_id,
            album_alias.canonical_url,
            membership.track_entity_id,
            membership.membership_source,
            membership.position,
            track.provider,
            track.title,
            track.subtitle,
            track.canonical_url,
            track.artwork_url,
            track.duration_seconds,
            track.bitrate_bps,
            track.audio_format,
            track_alias.remote_id,
            cached_tracks.artist,
            cached_tracks.album,
            cached_tracks.duration_seconds,
            cached_tracks.bitrate_bps,
            cached_tracks.audio_format,
            cached_tracks.artwork_path
        FROM library_entities album
        JOIN library_collection_tracks membership
            ON membership.collection_entity_id = album.entity_id
        JOIN library_entities track
            ON track.entity_id = membership.track_entity_id
        LEFT JOIN library_entity_aliases album_alias
            ON album_alias.entity_id = album.entity_id
           AND album_alias.entity_kind = 'album'
           AND album_alias.provider = album.provider
        LEFT JOIN library_entity_aliases track_alias
            ON track_alias.entity_id = track.entity_id
           AND track_alias.entity_kind = 'track'
           AND track_alias.provider = track.provider
        LEFT JOIN cached_tracks
            ON cached_tracks.provider = track.provider
           AND cached_tracks.track_id = track_alias.remote_id
        WHERE album.kind = 'album'
          AND EXISTS (
              SELECT 1
              FROM library_collection_tracks local_membership
              WHERE local_membership.collection_entity_id = album.entity_id
                AND local_membership.membership_source = 'cached_track'
          )
        ORDER BY
            lower(album.title),
            lower(coalesce(album.subtitle, '')),
            membership.position IS NULL,
            membership.position ASC,
            CASE membership.membership_source
                WHEN 'provider_snapshot' THEN 0
                ELSE 1
            END,
            lower(track.title)
        "#,
    )?;

    #[derive(Clone)]
    struct AlbumRow {
        collection_entity_id: String,
        collection: CollectionSummary,
        track: TrackSummary,
        track_entity_id: String,
        position: Option<usize>,
        membership_source: String,
    }

    let rows = statement.query_map([], |row| {
        let provider = row.get::<_, String>(1)?;
        let track_provider = row.get::<_, String>(10)?;
        let provider_id = crate::provider::ProviderId::parse(&provider).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                format!("unknown provider '{provider}'").into(),
            )
        })?;
        let track_provider_id =
            crate::provider::ProviderId::parse(&track_provider).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    format!("unknown provider '{track_provider}'").into(),
                )
            })?;
        let album_remote_id: Option<String> = row.get(5)?;
        let album_canonical_url: Option<String> = row.get(6)?;
        let track_duration_seconds: Option<u32> = row.get(15)?;
        let track_bitrate_bps: Option<u32> = row.get(16)?;
        let track_audio_format: Option<String> = row.get(17)?;
        let track_remote_id: Option<String> = row.get(18)?;
        let track_canonical_url: Option<String> = row.get(13)?;
        let album_entity_id: String = row.get(0)?;
        let track_entity_id: String = row.get(7)?;
        let album_title: String = row.get(2)?;
        let album_subtitle: Option<String> = row.get(3)?;
        let album_artwork_url: Option<String> = row.get(4)?;
        let track_title: String = row.get(11)?;
        let track_subtitle: Option<String> = row.get(12)?;
        let track_artwork_url: Option<String> = row.get(14)?;
        let cached_artist: Option<String> = row.get(19)?;
        let cached_album: Option<String> = row.get(20)?;
        let cached_duration_seconds: Option<u32> = row.get(21)?;
        let cached_bitrate_bps: Option<u32> = row.get(22)?;
        let cached_audio_format: Option<String> = row.get(23)?;
        let cached_artwork_path: Option<String> = row.get(24)?;

        let collection_id = album_remote_id
            .clone()
            .unwrap_or_else(|| canonical_id_from_entity_id(&album_entity_id).to_string());
        let collection = CollectionSummary {
            reference: CollectionRef::new(
                provider_id,
                collection_id.clone(),
                CollectionKind::Album,
                album_canonical_url,
            ),
            title: album_title.clone(),
            subtitle: album_subtitle.clone(),
            artwork_url: cached_artwork_path.clone().or(album_artwork_url.clone()),
            track_count: None,
        };

        let track_id = track_remote_id
            .unwrap_or_else(|| canonical_id_from_entity_id(&track_entity_id).to_string());
        Ok::<AlbumRow, rusqlite::Error>(AlbumRow {
            collection_entity_id: album_entity_id,
            collection,
            track: TrackSummary {
                reference: crate::provider::TrackRef::new(
                    track_provider_id,
                    track_id,
                    track_canonical_url,
                    Some(track_title.clone()),
                ),
                title: track_title,
                artist: cached_artist.or(track_subtitle),
                album: cached_album.or_else(|| Some(album_title.clone())),
                collection_id: Some(collection_id),
                collection_title: Some(album_title),
                collection_subtitle: album_subtitle,
                duration_seconds: cached_duration_seconds.or(track_duration_seconds),
                bitrate_bps: cached_bitrate_bps.or(track_bitrate_bps),
                audio_format: cached_audio_format
                    .or(track_audio_format)
                    .as_deref()
                    .and_then(parse_audio_format),
                artwork_url: cached_artwork_path.or(track_artwork_url),
            },
            track_entity_id,
            position: row.get(9)?,
            membership_source: row.get(8)?,
        })
    })?;

    let mut albums = BTreeMap::<String, (CollectionSummary, Vec<AlbumRow>)>::new();
    for row in rows {
        let row = row?;
        let entry = albums
            .entry(row.collection_entity_id.clone())
            .or_insert_with(|| (row.collection.clone(), Vec::new()));
        entry.1.push(row);
    }

    let mut lists = albums
        .into_values()
        .map(|(mut collection, rows)| {
            let mut included_tracks = HashSet::new();
            let mut tracks = Vec::new();
            for row in rows {
                if included_tracks.insert(row.track_entity_id.clone()) {
                    tracks.push((row.position, row.membership_source, row.track));
                }
            }
            tracks.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| source_rank(&left.1).cmp(&source_rank(&right.1)))
                    .then_with(|| left.2.title.cmp(&right.2.title))
            });
            collection.track_count = Some(tracks.len());
            TrackList {
                collection,
                tracks: tracks.into_iter().map(|(_, _, track)| track).collect(),
            }
        })
        .collect::<Vec<_>>();

    lists.sort_by(|left, right| {
        left.collection
            .title
            .to_lowercase()
            .cmp(&right.collection.title.to_lowercase())
            .then_with(|| {
                left.collection
                    .subtitle
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(
                        &right
                            .collection
                            .subtitle
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase(),
                    )
            })
    });

    Ok(lists)
}

pub(super) fn load_collection_track_list(
    connection: &Connection,
    collection: &CollectionRef,
) -> Result<Option<TrackList>> {
    let collection_entity_kind = match collection.kind {
        CollectionKind::Album => "album",
        CollectionKind::Playlist => "playlist",
    };
    let collection_kind = collection.kind;
    let provider = collection.provider;
    let collection_id = canonical_collection_id(collection).to_string();
    let entity_id = format!(
        "{}:{}:{}",
        collection_entity_kind,
        provider.as_str(),
        collection_id
    );

    let exists = connection
        .query_row(
            r#"
            SELECT 1
            FROM library_entities
            WHERE entity_id = ?1
            LIMIT 1
            "#,
            params![entity_id],
            |_row| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(None);
    }

    let mut statement = connection.prepare(
        r#"
        SELECT
            collection.title,
            collection.subtitle,
            collection.canonical_url,
            collection.artwork_url,
            membership.track_entity_id,
            membership.membership_source,
            membership.position,
            track.provider,
            track.title,
            track.subtitle,
            track.canonical_url,
            track.artwork_url,
            track.duration_seconds,
            track.bitrate_bps,
            track.audio_format,
            track_alias.remote_id,
            cached_tracks.artist,
            cached_tracks.album,
            cached_tracks.duration_seconds,
            cached_tracks.bitrate_bps,
            cached_tracks.audio_format,
            cached_tracks.artwork_path
        FROM library_entities collection
        JOIN library_collection_tracks membership
            ON membership.collection_entity_id = collection.entity_id
        JOIN library_entities track
            ON track.entity_id = membership.track_entity_id
        LEFT JOIN library_entity_aliases track_alias
            ON track_alias.entity_id = track.entity_id
           AND track_alias.entity_kind = 'track'
           AND track_alias.provider = track.provider
        LEFT JOIN cached_tracks
            ON cached_tracks.provider = track.provider
           AND cached_tracks.track_id = track_alias.remote_id
        WHERE collection.entity_id = ?1
        ORDER BY
            membership.position IS NULL,
            membership.position ASC,
            CASE membership.membership_source
                WHEN 'provider_snapshot' THEN 0
                ELSE 1
            END,
            lower(track.title)
        "#,
    )?;

    #[derive(Clone)]
    struct CollectionTrackRow {
        track_entity_id: String,
        membership_source: String,
        position: Option<usize>,
        track: TrackSummary,
    }

    let rows = statement.query_map(params![entity_id], |row| {
        let track_provider: String = row.get(7)?;
        let track_provider_id =
            crate::provider::ProviderId::parse(&track_provider).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    format!("unknown provider '{track_provider}'").into(),
                )
            })?;
        let collection_title: String = row.get(0)?;
        let collection_subtitle: Option<String> = row.get(1)?;
        let track_title: String = row.get(8)?;
        let track_subtitle: Option<String> = row.get(9)?;
        let track_duration_seconds: Option<u32> = row.get(12)?;
        let track_bitrate_bps: Option<u32> = row.get(13)?;
        let track_audio_format: Option<String> = row.get(14)?;
        let track_remote_id: Option<String> = row.get(15)?;
        let track_entity_id: String = row.get(4)?;
        let cached_artist: Option<String> = row.get(16)?;
        let cached_album: Option<String> = row.get(17)?;
        let cached_duration_seconds: Option<u32> = row.get(18)?;
        let cached_bitrate_bps: Option<u32> = row.get(19)?;
        let cached_audio_format: Option<String> = row.get(20)?;
        let cached_artwork_path: Option<String> = row.get(21)?;

        let track_id = track_remote_id
            .unwrap_or_else(|| canonical_id_from_entity_id(&track_entity_id).to_string());
        Ok::<CollectionTrackRow, rusqlite::Error>(CollectionTrackRow {
            track_entity_id,
            membership_source: row.get(5)?,
            position: row.get(6)?,
            track: TrackSummary {
                reference: crate::provider::TrackRef::new(
                    track_provider_id,
                    track_id,
                    row.get(10)?,
                    Some(track_title.clone()),
                ),
                title: track_title,
                artist: cached_artist.or(track_subtitle),
                album: cached_album.or_else(|| Some(collection_title.clone())),
                collection_id: Some(collection_id.clone()),
                collection_title: Some(collection_title),
                collection_subtitle: collection_subtitle,
                duration_seconds: cached_duration_seconds.or(track_duration_seconds),
                bitrate_bps: cached_bitrate_bps.or(track_bitrate_bps),
                audio_format: cached_audio_format
                    .or(track_audio_format)
                    .as_deref()
                    .and_then(parse_audio_format),
                artwork_url: cached_artwork_path.or_else(|| row.get(11).ok().flatten()),
            },
        })
    })?;

    let collection_title: String = connection.query_row(
        "SELECT title FROM library_entities WHERE entity_id = ?1",
        params![format!(
            "{}:{}:{}",
            collection_entity_kind,
            provider.as_str(),
            collection_id
        )],
        |row| row.get(0),
    )?;
    let collection_subtitle: Option<String> = connection.query_row(
        "SELECT subtitle FROM library_entities WHERE entity_id = ?1",
        params![format!(
            "{}:{}:{}",
            collection_entity_kind,
            provider.as_str(),
            collection_id
        )],
        |row| row.get(0),
    )?;
    let collection_artwork_url: Option<String> = connection.query_row(
        "SELECT artwork_url FROM library_entities WHERE entity_id = ?1",
        params![format!(
            "{}:{}:{}",
            collection_entity_kind,
            provider.as_str(),
            collection_id
        )],
        |row| row.get(0),
    )?;

    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    for row in rows {
        let row = row?;
        if seen.insert(row.track_entity_id) {
            tracks.push((row.position, row.membership_source, row.track));
        }
    }
    tracks.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| source_rank(&left.1).cmp(&source_rank(&right.1)))
            .then_with(|| left.2.title.cmp(&right.2.title))
    });

    Ok(Some(TrackList {
        collection: CollectionSummary {
            reference: CollectionRef::new(
                provider,
                collection_id,
                collection_kind,
                collection.canonical_url.clone(),
            ),
            title: collection_title,
            subtitle: collection_subtitle,
            artwork_url: collection_artwork_url,
            track_count: Some(tracks.len()),
        },
        tracks: tracks.into_iter().map(|(_, _, track)| track).collect(),
    }))
}

pub(super) fn playlist_track_lists(connection: &Connection) -> Result<Vec<TrackList>> {
    let liked_playlist_entity_id = liked_playlist_entity_id();
    let mut statement = connection.prepare(
        r#"
        SELECT provider, canonical_url, entity_id
        FROM library_entities
        WHERE kind = 'playlist' AND provider IS NOT NULL
        ORDER BY
            CASE WHEN entity_id = ?1 THEN 0 ELSE 1 END,
            CASE WHEN provider = ?2 THEN 0 ELSE 1 END,
            lower(title),
            lower(coalesce(subtitle, ''))
        "#,
    )?;

    let rows = statement.query_map(
        params![
            liked_playlist_entity_id,
            crate::provider::ProviderId::Local.as_str()
        ],
        |row| {
            let provider: String = row.get(0)?;
            let provider_id = crate::provider::ProviderId::parse(&provider).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    format!("unknown provider '{provider}'").into(),
                )
            })?;
            let entity_id: String = row.get(2)?;
            Ok::<CollectionRef, rusqlite::Error>(CollectionRef::new(
                provider_id,
                canonical_id_from_entity_id(&entity_id).to_string(),
                CollectionKind::Playlist,
                row.get(1)?,
            ))
        },
    )?;

    let mut lists = Vec::new();
    for row in rows {
        let collection = row?;
        if let Some(track_list) = load_collection_track_list(connection, &collection)? {
            lists.push(track_list);
        }
    }

    Ok(lists)
}

pub(super) fn liked_track_keys(connection: &Connection) -> Result<HashSet<String>> {
    let mut statement = connection.prepare(
        r#"
        SELECT track.provider, membership.track_entity_id
        FROM library_collection_tracks membership
        JOIN library_entities track
            ON track.entity_id = membership.track_entity_id
        WHERE membership.collection_entity_id = ?1
        ORDER BY membership.position ASC, membership.created_at ASC
        "#,
    )?;

    let rows = statement.query_map(params![liked_playlist_entity_id()], |row| {
        let provider: String = row.get(0)?;
        let provider_id = crate::provider::ProviderId::parse(&provider).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown provider '{provider}'").into(),
            )
        })?;
        let track_entity_id: String = row.get(1)?;
        Ok::<String, rusqlite::Error>(format!(
            "{}:{}",
            provider_id.as_str(),
            canonical_id_from_entity_id(&track_entity_id)
        ))
    })?;

    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row?);
    }

    Ok(keys)
}

pub(super) fn toggle_liked_track(connection: &Connection, track: &TrackSummary) -> Result<bool> {
    let playlist = liked_playlist_summary();
    let playlist_entity_id = liked_playlist_entity_id();
    let track_entity_id = entity_id_for_track(track);

    upsert_collection_entity(connection, EntityKind::Playlist, &playlist)?;

    let existing_membership = connection
        .query_row(
            r#"
            SELECT 1
            FROM library_collection_tracks
            WHERE collection_entity_id = ?1
              AND track_entity_id = ?2
              AND membership_source = ?3
            LIMIT 1
            "#,
            params![
                playlist_entity_id,
                track_entity_id,
                SYSTEM_PLAYLIST_MEMBERSHIP_SOURCE
            ],
            |_row| Ok(()),
        )
        .optional()?
        .is_some();

    if existing_membership {
        connection.execute(
            r#"
            DELETE FROM library_collection_tracks
            WHERE collection_entity_id = ?1
              AND track_entity_id = ?2
              AND membership_source = ?3
            "#,
            params![
                playlist_entity_id,
                track_entity_id,
                SYSTEM_PLAYLIST_MEMBERSHIP_SOURCE
            ],
        )?;
        return Ok(false);
    }

    upsert_track_entity(connection, track, track.artwork_url.as_deref())?;
    let next_position = next_playlist_position(
        connection,
        &playlist_entity_id,
        SYSTEM_PLAYLIST_MEMBERSHIP_SOURCE,
    )?;
    connection.execute(
        r#"
        INSERT INTO library_collection_tracks (
            collection_entity_id,
            track_entity_id,
            membership_source,
            position,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, unixepoch())
        ON CONFLICT(collection_entity_id, track_entity_id, membership_source) DO UPDATE SET
            position = excluded.position,
            updated_at = unixepoch()
        "#,
        params![
            playlist_entity_id,
            track_entity_id,
            SYSTEM_PLAYLIST_MEMBERSHIP_SOURCE,
            next_position
        ],
    )?;

    Ok(true)
}

fn upsert_collection_entity(
    connection: &Connection,
    kind: EntityKind,
    collection: &CollectionSummary,
) -> Result<()> {
    let entity_id = entity_id_for_collection(kind, &collection.reference);
    connection.execute(
        r#"
        INSERT INTO library_entities (
            entity_id,
            kind,
            provider,
            title,
            subtitle,
            canonical_url,
            artwork_url,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())
        ON CONFLICT(entity_id) DO UPDATE SET
            title = excluded.title,
            subtitle = COALESCE(excluded.subtitle, subtitle),
            canonical_url = COALESCE(excluded.canonical_url, canonical_url),
            artwork_url = COALESCE(excluded.artwork_url, artwork_url),
            updated_at = unixepoch()
        "#,
        params![
            entity_id,
            kind.as_str(),
            collection.reference.provider.as_str(),
            &collection.title,
            collection.subtitle.as_deref(),
            collection.reference.canonical_url.as_deref(),
            collection.artwork_url.as_deref(),
        ],
    )?;

    upsert_alias(
        connection,
        &entity_id,
        kind,
        collection.reference.provider.as_str(),
        canonical_collection_id(&collection.reference),
        collection.reference.canonical_url.as_deref(),
    )?;

    Ok(())
}

fn upsert_track_entity(
    connection: &Connection,
    track: &TrackSummary,
    artwork_url: Option<&str>,
) -> Result<()> {
    let entity_id = entity_id_for_track(track);
    connection.execute(
        r#"
        INSERT INTO library_entities (
            entity_id,
            kind,
            provider,
            title,
            subtitle,
            canonical_url,
            artwork_url,
            duration_seconds,
            bitrate_bps,
            audio_format,
            updated_at
        ) VALUES (?1, 'track', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch())
        ON CONFLICT(entity_id) DO UPDATE SET
            title = excluded.title,
            subtitle = COALESCE(excluded.subtitle, subtitle),
            canonical_url = COALESCE(excluded.canonical_url, canonical_url),
            artwork_url = COALESCE(excluded.artwork_url, artwork_url),
            duration_seconds = COALESCE(excluded.duration_seconds, duration_seconds),
            bitrate_bps = COALESCE(excluded.bitrate_bps, bitrate_bps),
            audio_format = COALESCE(excluded.audio_format, audio_format),
            updated_at = unixepoch()
        "#,
        params![
            entity_id,
            track.reference.provider.as_str(),
            &track.title,
            track.artist.as_deref(),
            track.reference.canonical_url.as_deref(),
            artwork_url,
            track.duration_seconds,
            track.bitrate_bps,
            track.audio_format.as_ref().map(serialize_audio_format),
        ],
    )?;

    upsert_alias(
        connection,
        &entity_id,
        EntityKind::Track,
        track.reference.provider.as_str(),
        &track.reference.id,
        track.reference.canonical_url.as_deref(),
    )?;

    Ok(())
}

fn upsert_alias(
    connection: &Connection,
    entity_id: &str,
    kind: EntityKind,
    provider: &str,
    remote_id: &str,
    canonical_url: Option<&str>,
) -> Result<()> {
    connection.execute(
        r#"
        INSERT INTO library_entity_aliases (
            entity_id,
            provider,
            entity_kind,
            remote_id,
            canonical_url,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
        ON CONFLICT(provider, entity_kind, remote_id) DO UPDATE SET
            entity_id = excluded.entity_id,
            canonical_url = COALESCE(excluded.canonical_url, canonical_url),
            updated_at = unixepoch()
        "#,
        params![entity_id, provider, kind.as_str(), remote_id, canonical_url],
    )?;

    Ok(())
}

fn inferred_collection_summary(
    track: &TrackSummary,
    artwork_url: Option<&str>,
) -> Option<CollectionSummary> {
    let collection_id = track.collection_id.clone()?;
    let title = track
        .collection_title
        .clone()
        .or_else(|| track.album.clone())
        .unwrap_or_else(|| "Unknown album".to_string());

    Some(CollectionSummary {
        reference: CollectionRef::new(
            track.reference.provider,
            collection_id,
            CollectionKind::Album,
            None,
        ),
        title,
        subtitle: track
            .collection_subtitle
            .clone()
            .or_else(|| track.artist.clone()),
        artwork_url: artwork_url.map(str::to_string),
        track_count: None,
    })
}

fn liked_playlist_summary() -> CollectionSummary {
    CollectionSummary {
        reference: CollectionRef::new(
            crate::provider::ProviderId::Local,
            LIKED_PLAYLIST_ID,
            CollectionKind::Playlist,
            None,
        ),
        title: LIKED_PLAYLIST_TITLE.to_string(),
        subtitle: Some(LIKED_PLAYLIST_SUBTITLE.to_string()),
        artwork_url: None,
        track_count: None,
    }
}

fn liked_playlist_entity_id() -> String {
    entity_id_for_collection(EntityKind::Playlist, &liked_playlist_summary().reference)
}

fn entity_id_for_collection(kind: EntityKind, collection: &CollectionRef) -> String {
    format!(
        "{}:{}:{}",
        kind.as_str(),
        collection.provider.as_str(),
        canonical_collection_id(collection)
    )
}

fn entity_id_for_track(track: &TrackSummary) -> String {
    format!(
        "{}:{}:{}",
        EntityKind::Track.as_str(),
        track.reference.provider.as_str(),
        track.reference.id
    )
}

fn canonical_collection_id(collection: &CollectionRef) -> &str {
    &collection.id
}

fn canonical_id_from_entity_id(entity_id: &str) -> &str {
    entity_id.splitn(3, ':').nth(2).unwrap_or(entity_id)
}

fn source_rank(source: &str) -> u8 {
    match source {
        "provider_snapshot" => 0,
        "cached_track" => 1,
        SYSTEM_PLAYLIST_MEMBERSHIP_SOURCE => 2,
        _ => 2,
    }
}

fn next_playlist_position(
    connection: &Connection,
    collection_entity_id: &str,
    membership_source: &str,
) -> Result<usize> {
    let next_position = connection.query_row(
        r#"
        SELECT COALESCE(MAX(position) + 1, 0)
        FROM library_collection_tracks
        WHERE collection_entity_id = ?1
          AND membership_source = ?2
        "#,
        params![collection_entity_id, membership_source],
        |row| row.get::<_, usize>(0),
    )?;

    Ok(next_position)
}

fn parse_audio_format(value: &str) -> Option<crate::provider::AudioFormat> {
    Some(match value {
        "mp3" => crate::provider::AudioFormat::Mp3,
        "flac" => crate::provider::AudioFormat::Flac,
        "opus" => crate::provider::AudioFormat::Opus,
        "aac" => crate::provider::AudioFormat::Aac,
        "m4a" => crate::provider::AudioFormat::M4a,
        _ => crate::provider::AudioFormat::Unknown(value.to_string()),
    })
}

fn serialize_audio_format(value: &crate::provider::AudioFormat) -> String {
    match value {
        crate::provider::AudioFormat::Mp3 => "mp3".to_string(),
        crate::provider::AudioFormat::Flac => "flac".to_string(),
        crate::provider::AudioFormat::Opus => "opus".to_string(),
        crate::provider::AudioFormat::Aac => "aac".to_string(),
        crate::provider::AudioFormat::M4a => "m4a".to_string(),
        crate::provider::AudioFormat::Unknown(label) => label.to_ascii_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AudioFormat, CollectionKind, ProviderId, TrackRef};
    use rusqlite::Connection;

    fn fixture_provider() -> ProviderId {
        ProviderId::parse("fixture_remote").expect("fixture provider id should parse")
    }

    #[test]
    fn canonical_collection_id_uses_remote_collection_id() {
        let collection =
            CollectionRef::new(fixture_provider(), "album-1", CollectionKind::Album, None);

        assert_eq!(canonical_collection_id(&collection), "album-1");
    }

    #[test]
    fn entity_ids_are_stable_for_remote_album_and_track() {
        let collection =
            CollectionRef::new(fixture_provider(), "album-1", CollectionKind::Album, None);
        let track = TrackSummary {
            reference: TrackRef::new(fixture_provider(), "track-1", None, None),
            title: "Track One".to_string(),
            artist: None,
            album: None,
            collection_id: Some("album-1".to_string()),
            collection_title: Some("Album One".to_string()),
            collection_subtitle: None,
            duration_seconds: None,
            bitrate_bps: None,
            audio_format: None,
            artwork_url: None,
        };

        assert_eq!(
            entity_id_for_collection(EntityKind::Album, &collection),
            "album:fixture_remote:album-1"
        );
        assert_eq!(entity_id_for_track(&track), "track:fixture_remote:track-1");
    }

    #[test]
    fn load_collection_track_list_preserves_provider_metadata_without_cached_track() {
        let connection = Connection::open_in_memory().expect("in-memory sqlite");
        initialize_schema(&connection).expect("schema");
        connection
            .execute_batch(
                r#"
                CREATE TABLE cached_tracks (
                    provider TEXT NOT NULL,
                    track_id TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    duration_seconds INTEGER,
                    bitrate_bps INTEGER,
                    audio_format TEXT,
                    artwork_path TEXT
                );
                "#,
            )
            .expect("cached_tracks table");

        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    fixture_provider(),
                    "album-1",
                    CollectionKind::Album,
                    None,
                ),
                title: "Album One".to_string(),
                subtitle: Some("Artist One".to_string()),
                artwork_url: Some("https://example.com/album.jpg".to_string()),
                track_count: Some(1),
            },
            tracks: vec![TrackSummary {
                reference: TrackRef::new(
                    fixture_provider(),
                    "track-1",
                    Some("https://example.com/track-1.mp3".to_string()),
                    Some("Track One".to_string()),
                ),
                title: "Track One".to_string(),
                artist: Some("Artist One".to_string()),
                album: Some("Album One".to_string()),
                collection_id: Some("album-1".to_string()),
                collection_title: Some("Album One".to_string()),
                collection_subtitle: Some("Artist One".to_string()),
                duration_seconds: Some(301),
                bitrate_bps: Some(320_000),
                audio_format: Some(AudioFormat::Mp3),
                artwork_url: Some("https://example.com/track.jpg".to_string()),
            }],
        };

        sync_collection_track_list(&connection, &track_list).expect("sync track list");
        let loaded = load_collection_track_list(&connection, &track_list.collection.reference)
            .expect("load")
            .expect("track list should exist");

        assert_eq!(loaded.tracks.len(), 1);
        assert_eq!(loaded.tracks[0].duration_seconds, Some(301));
        assert_eq!(loaded.tracks[0].bitrate_bps, Some(320_000));
        assert_eq!(loaded.tracks[0].audio_format, Some(AudioFormat::Mp3));
    }

    #[test]
    fn album_track_lists_preserve_provider_metadata_without_cached_track() {
        let connection = Connection::open_in_memory().expect("in-memory sqlite");
        initialize_schema(&connection).expect("schema");
        connection
            .execute_batch(
                r#"
                CREATE TABLE cached_tracks (
                    provider TEXT NOT NULL,
                    track_id TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    duration_seconds INTEGER,
                    bitrate_bps INTEGER,
                    audio_format TEXT,
                    artwork_path TEXT
                );
                "#,
            )
            .expect("cached_tracks table");

        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    fixture_provider(),
                    "album-1",
                    CollectionKind::Album,
                    None,
                ),
                title: "Album One".to_string(),
                subtitle: Some("Artist One".to_string()),
                artwork_url: None,
                track_count: Some(1),
            },
            tracks: vec![TrackSummary {
                reference: TrackRef::new(fixture_provider(), "track-1", None, None),
                title: "Track One".to_string(),
                artist: Some("Artist One".to_string()),
                album: Some("Album One".to_string()),
                collection_id: Some("album-1".to_string()),
                collection_title: Some("Album One".to_string()),
                collection_subtitle: Some("Artist One".to_string()),
                duration_seconds: Some(301),
                bitrate_bps: Some(320_000),
                audio_format: Some(AudioFormat::Mp3),
                artwork_url: None,
            }],
        };

        sync_collection_track_list(&connection, &track_list).expect("sync track list");
        sync_cached_track(&connection, &track_list.tracks[0], Some(0), None)
            .expect("sync cached track");
        let albums = album_track_lists(&connection).expect("album track lists");

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].tracks.len(), 1);
        assert_eq!(albums[0].tracks[0].duration_seconds, Some(301));
        assert_eq!(albums[0].tracks[0].bitrate_bps, Some(320_000));
        assert_eq!(albums[0].tracks[0].audio_format, Some(AudioFormat::Mp3));
    }

    #[test]
    fn album_track_lists_exclude_discover_only_albums() {
        let connection = Connection::open_in_memory().expect("in-memory sqlite");
        initialize_schema(&connection).expect("schema");
        connection
            .execute_batch(
                r#"
                CREATE TABLE cached_tracks (
                    provider TEXT NOT NULL,
                    track_id TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    duration_seconds INTEGER,
                    bitrate_bps INTEGER,
                    audio_format TEXT,
                    artwork_path TEXT
                );
                "#,
            )
            .expect("cached_tracks table");

        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    fixture_provider(),
                    "album-1",
                    CollectionKind::Album,
                    None,
                ),
                title: "Album One".to_string(),
                subtitle: Some("Artist One".to_string()),
                artwork_url: None,
                track_count: Some(1),
            },
            tracks: vec![TrackSummary {
                reference: TrackRef::new(fixture_provider(), "track-1", None, None),
                title: "Track One".to_string(),
                artist: Some("Artist One".to_string()),
                album: Some("Album One".to_string()),
                collection_id: Some("album-1".to_string()),
                collection_title: Some("Album One".to_string()),
                collection_subtitle: Some("Artist One".to_string()),
                duration_seconds: Some(301),
                bitrate_bps: Some(320_000),
                audio_format: Some(AudioFormat::Mp3),
                artwork_url: None,
            }],
        };

        sync_collection_track_list(&connection, &track_list).expect("sync track list");

        let albums = album_track_lists(&connection).expect("album track lists");

        assert!(albums.is_empty());
    }

    #[test]
    fn toggle_liked_track_creates_playlist_and_membership() {
        let connection = Connection::open_in_memory().expect("in-memory sqlite");
        initialize_schema(&connection).expect("schema");
        connection
            .execute_batch(
                r#"
                CREATE TABLE cached_tracks (
                    provider TEXT NOT NULL,
                    track_id TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    duration_seconds INTEGER,
                    bitrate_bps INTEGER,
                    audio_format TEXT,
                    artwork_path TEXT
                );
                "#,
            )
            .expect("cached_tracks table");

        let track = TrackSummary {
            reference: TrackRef::new(
                fixture_provider(),
                "track-1",
                Some("https://example.com/track-1.mp3".to_string()),
                Some("Track One".to_string()),
            ),
            title: "Track One".to_string(),
            artist: Some("Artist One".to_string()),
            album: Some("Album One".to_string()),
            collection_id: Some("album-1".to_string()),
            collection_title: Some("Album One".to_string()),
            collection_subtitle: Some("Artist One".to_string()),
            duration_seconds: Some(301),
            bitrate_bps: Some(320_000),
            audio_format: Some(AudioFormat::Mp3),
            artwork_url: Some("https://example.com/track.jpg".to_string()),
        };

        let liked = toggle_liked_track(&connection, &track).expect("toggle like");
        let keys = liked_track_keys(&connection).expect("liked track keys");
        let playlists = playlist_track_lists(&connection).expect("playlist track lists");

        assert!(liked);
        assert!(keys.contains("fixture_remote:track-1"));
        assert_eq!(playlists.len(), 1);
        assert_eq!(
            playlists[0].collection.reference.provider,
            ProviderId::Local
        );
        assert_eq!(playlists[0].collection.reference.id, LIKED_PLAYLIST_ID);
        assert_eq!(playlists[0].collection.title, LIKED_PLAYLIST_TITLE);
        assert_eq!(playlists[0].tracks.len(), 1);
        assert_eq!(
            playlists[0].tracks[0].reference.provider,
            fixture_provider()
        );
        assert_eq!(playlists[0].tracks[0].reference.id, "track-1");
    }

    #[test]
    fn toggle_liked_track_removes_membership_but_keeps_playlist() {
        let connection = Connection::open_in_memory().expect("in-memory sqlite");
        initialize_schema(&connection).expect("schema");
        connection
            .execute_batch(
                r#"
                CREATE TABLE cached_tracks (
                    provider TEXT NOT NULL,
                    track_id TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    duration_seconds INTEGER,
                    bitrate_bps INTEGER,
                    audio_format TEXT,
                    artwork_path TEXT
                );
                "#,
            )
            .expect("cached_tracks table");

        let track = TrackSummary {
            reference: TrackRef::new(
                fixture_provider(),
                "track-1",
                None,
                Some("Track One".into()),
            ),
            title: "Track One".to_string(),
            artist: Some("Artist One".to_string()),
            album: Some("Album One".to_string()),
            collection_id: Some("album-1".to_string()),
            collection_title: Some("Album One".to_string()),
            collection_subtitle: Some("Artist One".to_string()),
            duration_seconds: Some(301),
            bitrate_bps: Some(320_000),
            audio_format: Some(AudioFormat::Mp3),
            artwork_url: None,
        };

        assert!(toggle_liked_track(&connection, &track).expect("initial like"));
        let liked = toggle_liked_track(&connection, &track).expect("toggle unlike");
        let keys = liked_track_keys(&connection).expect("liked track keys");
        let playlists = playlist_track_lists(&connection).expect("playlist track lists");

        assert!(!liked);
        assert!(keys.is_empty());
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].collection.reference.id, LIKED_PLAYLIST_ID);
        assert!(playlists[0].tracks.is_empty());
    }
}
