# Entity Plan

## Goal

Replace the current "cache plus synthetic view rebuild" model with a local entity model where albums,
artists, playlists, and tracks have stable local identities and remote provider ids become aliases.

## Problems To Solve

- The same album can exist as multiple app identities:
  - discover collection: `provider:remote_id`
  - local albums entry: `local-album:provider:remote_id`
- Playback, discover, and local library compare different ids and drift apart.
- Track/album metadata is split between `cached_tracks`, saved collection track lists, session state,
  and UI-specific repair logic.
- We have compensating invalidation/refetch code instead of durable entities.

## Design Principles

- Providers stay provider-native. They keep returning provider ids, URLs, and track lists.
- The app ingests provider data into local entities.
- Local entities are persistent and stable.
- Remote ids and canonical URLs are aliases, not primary app identity.
- Metadata is accumulated gradually instead of replacing one surface with another.
- File availability is attached to track entities, not inferred by rebuilding album definitions.

## Initial Schema

- `library_entities`
  - `entity_id`, `kind`, `provider`, `title`, `subtitle`, `canonical_url`, `artwork_url`
- `library_entity_aliases`
  - maps `(provider, entity_kind, remote_id, canonical_url)` to `entity_id`
- `library_collection_tracks`
  - ordered links between album/playlist entities and track entities
  - includes a `membership_source` so imported/cached/remote-snapshot links can coexist during migration

## Target Schema

- `library_entities`
  - stable local ids for album / artist / playlist / track
- `library_entity_aliases`
  - remote ids and canonical URLs for provider-backed entities
- `library_collection_tracks`
  - album-track and playlist-track order
- `library_artist_links`
  - artist -> album / track relationships
- `library_track_files`
  - local file availability, artwork, quality, source
- `library_entity_refresh`
  - refresh bookkeeping, provenance, timestamps, stale policies

## Implementation Phases

1. Canonical identity
   - Normalize cross-surface comparison so a local album and discover album resolve to the same logical album.
   - Stop selection/playback/highlight mismatches.
2. Add entity tables
   - Create additive tables and start writing album/track identities and aliases on current ingest paths.
3. Ingest provider snapshots
   - Save discover/provider track lists into entity tables, not just `app_state`.
   - Save ordered album membership explicitly.
4. Ingest cached/local files
   - Save track entities and file availability from `cached_tracks` and imports.
   - Stop rebuilding local albums from ad hoc merge logic.
5. Move UI reads to entities
   - Albums/artists/playlists/discover/playback all read through local entity ids.
   - Session state stores entity ids instead of surface-specific ids.
6. Remove workaround invalidation
   - Delete implicit album hydration on left-click.
   - Remove broad provider cache clears and search-result cache clears.
   - Remove provider-specific refetch hacks.
   - Replace full library rebuilds with targeted entity updates.

## Workaround Cleanup Targets

- `maybe_hydrate_local_album`
- `clear_search_result_collection_caches`
- `clear_provider_collection_cache` on auth/provider toggles
- `should_refetch_cached_track_list`
- repeated `refresh_local_library_views` full rebuilds
- `visible_local_track_list_override` preservation logic caused by unstable identity
- runtime metadata reindex from saved collection caches

## Refresh Semantics

- Imported local files: never remote refresh.
- Provider-backed albums/playlists: explicit refresh action first, possible background stale refresh later.
- Artwork refresh: field-level entity update, not track-list replacement.
- Track availability updates: update track-file state only.

## Playlist / Likes Model

- Playlists are first-class local entities even before remote write support exists.
- A future provider playlist can attach a remote alias to an existing local playlist entity.
- Likes are a system playlist entity with stable local identity.

## Current Implementation Slice

- Canonical cross-surface collection identity in UI comparisons.
- Additive entity tables in the library database.
- Current collection/cached-track write paths begin populating entity rows and aliases.
