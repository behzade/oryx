use super::*;
use crate::provider::TrackRef;

fn fixture_provider() -> ProviderId {
    ProviderId::parse("fixture_remote").expect("fixture provider id should parse")
}

#[test]
fn local_artists_group_tracks_across_albums_and_use_local_artist_metadata() {
    let albums = vec![
        album_track_list(
            "album-b",
            "Album B",
            Some("Artist One"),
            Some("https://cdn.example/album-b.jpg"),
            &[
                track_summary(
                    fixture_provider(),
                    "track-2",
                    "Second",
                    Some("album-b"),
                    Some("Album B"),
                    Some("Artist One"),
                ),
                track_summary(
                    fixture_provider(),
                    "track-3",
                    "Third",
                    Some("album-b"),
                    Some("Album B"),
                    Some("Artist One"),
                ),
            ],
        ),
        album_track_list(
            "album-a",
            "Album A",
            Some("Artist One"),
            Some("https://cdn.example/album-a.jpg"),
            &[track_summary(
                fixture_provider(),
                "track-1",
                "First",
                Some("album-a"),
                Some("Album A"),
                Some("Artist One"),
            )],
        ),
    ];

    let artists = build_local_artist_lists(&albums);
    let artist = &artists[0];

    assert_eq!(artists.len(), 1);
    assert_eq!(artist.collection.reference.provider, ProviderId::Local);
    assert_eq!(artist.collection.reference.id, "local-artist:Artist One");
    assert_eq!(artist.collection.title, "Artist One");
    assert_eq!(artist.collection.subtitle.as_deref(), Some("2 albums"));
    assert_eq!(
        artist.collection.artwork_url.as_deref(),
        Some("https://cdn.example/album-a.jpg")
    );
    assert_eq!(artist.collection.track_count, Some(3));
    assert_eq!(artist.tracks.len(), 3);
}

#[test]
fn local_artists_use_expected_artist_fallback_order() {
    let albums = vec![
        album_track_list(
            "album-1",
            "Album One",
            Some("Album Artist"),
            None,
            &[
                track_summary(
                    fixture_provider(),
                    "track-1",
                    "First",
                    Some("album-1"),
                    Some("Album One"),
                    Some("Track Artist"),
                ),
                TrackSummary {
                    collection_subtitle: Some("Collection Artist".to_string()),
                    ..track_summary(
                        fixture_provider(),
                        "track-2",
                        "Second",
                        Some("album-1"),
                        Some("Album One"),
                        Some("Track Artist"),
                    )
                },
            ],
        ),
        album_track_list(
            "album-2",
            "Album Two",
            Some("Album Artist"),
            None,
            &[track_summary(
                fixture_provider(),
                "track-3",
                "Third",
                Some("album-2"),
                Some("Album Two"),
                None,
            )],
        ),
        album_track_list(
            "album-3",
            "Album Three",
            None,
            None,
            &[track_summary(
                fixture_provider(),
                "track-4",
                "Fourth",
                Some("album-3"),
                Some("Album Three"),
                None,
            )],
        ),
    ];

    let artists = build_local_artist_lists(&albums);
    let artist_titles: Vec<_> = artists
        .iter()
        .map(|artist| artist.collection.title.as_str())
        .collect();

    assert_eq!(
        artist_titles,
        vec![
            "Album Artist",
            "Collection Artist",
            "Track Artist",
            "Unknown artist",
        ]
    );
}

#[test]
fn filtered_cached_album_track_lists_excludes_playlists() {
    let lists = vec![
        album_track_list(
            "album-1",
            "Album One",
            Some("Artist One"),
            None,
            &[track_summary(
                fixture_provider(),
                "track-1",
                "Track One",
                Some("album-1"),
                Some("Album One"),
                Some("Artist One"),
            )],
        ),
        TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    ProviderId::Local,
                    "playlist-1",
                    CollectionKind::Playlist,
                    None,
                ),
                title: "Playlist One".to_string(),
                subtitle: None,
                artwork_url: None,
                track_count: Some(1),
            },
            tracks: vec![track_summary(
                fixture_provider(),
                "track-2",
                "Track Two",
                Some("album-2"),
                Some("Album Two"),
                Some("Artist Two"),
            )],
        },
    ];

    let filtered = filtered_cached_album_track_lists(
        lists,
        &HashSet::from([track_cache_key(&track_summary(
            fixture_provider(),
            "track-1",
            "Track One",
            Some("album-1"),
            Some("Album One"),
            Some("Artist One"),
        ))]),
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].collection.reference.kind, CollectionKind::Album);
    assert_eq!(filtered[0].collection.title, "Album One");
}

#[test]
fn filtered_cached_album_track_lists_excludes_system_playlist_ghost_albums() {
    let lists = vec![
        album_track_list(
            "album-1",
            "Album One",
            Some("Artist One"),
            None,
            &[track_summary(
                fixture_provider(),
                "track-1",
                "Track One",
                Some("album-1"),
                Some("Album One"),
                Some("Artist One"),
            )],
        ),
        album_track_list(
            "liked-tracks",
            "Liked Tracks",
            Some("System playlist"),
            None,
            &[track_summary(
                fixture_provider(),
                "track-2",
                "Track Two",
                Some("liked-tracks"),
                Some("Liked Tracks"),
                Some("System playlist"),
            )],
        ),
    ];

    let filtered = filtered_cached_album_track_lists(
        lists,
        &HashSet::from([track_cache_key(&track_summary(
            fixture_provider(),
            "track-1",
            "Track One",
            Some("album-1"),
            Some("Album One"),
            Some("Artist One"),
        ))]),
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].collection.title, "Album One");
}

#[test]
fn filtered_cached_album_track_lists_excludes_albums_without_cached_songs() {
    let cached_track = track_summary(
        fixture_provider(),
        "track-1",
        "Track One",
        Some("album-1"),
        Some("Album One"),
        Some("Artist One"),
    );
    let uncached_track = track_summary(
        fixture_provider(),
        "track-2",
        "Track Two",
        Some("album-2"),
        Some("Album Two"),
        Some("Artist Two"),
    );
    let lists = vec![
        album_track_list(
            "album-1",
            "Album One",
            Some("Artist One"),
            None,
            &[cached_track],
        ),
        album_track_list(
            "album-2",
            "Album Two",
            Some("Artist Two"),
            None,
            &[uncached_track],
        ),
    ];

    let filtered = filtered_cached_album_track_lists(
        lists,
        &HashSet::from([track_cache_key(&track_summary(
            fixture_provider(),
            "track-1",
            "Track One",
            Some("album-1"),
            Some("Album One"),
            Some("Artist One"),
        ))]),
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].collection.title, "Album One");
}

#[test]
fn filtered_cached_album_track_lists_keep_full_track_lists_for_mixed_albums() {
    let cached_track = track_summary(
        fixture_provider(),
        "track-1",
        "Track One",
        Some("album-1"),
        Some("Album One"),
        Some("Artist One"),
    );
    let uncached_track = track_summary(
        fixture_provider(),
        "track-2",
        "Track Two",
        Some("album-1"),
        Some("Album One"),
        Some("Artist One"),
    );
    let lists = vec![album_track_list(
        "album-1",
        "Album One",
        Some("Artist One"),
        None,
        &[cached_track, uncached_track],
    )];

    let filtered = filtered_cached_album_track_lists(
        lists,
        &HashSet::from([track_cache_key(&track_summary(
            fixture_provider(),
            "track-1",
            "Track One",
            Some("album-1"),
            Some("Album One"),
            Some("Artist One"),
        ))]),
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].tracks.len(), 2);
    assert_eq!(filtered[0].tracks[0].title, "Track One");
    assert_eq!(filtered[0].tracks[1].title, "Track Two");
    assert_eq!(filtered[0].collection.track_count, Some(2));
}

#[test]
fn local_artists_keep_uncached_tracks_from_albums_with_cached_songs() {
    let cached_track = track_summary(
        fixture_provider(),
        "track-1",
        "Track One",
        Some("album-1"),
        Some("Album One"),
        Some("Artist One"),
    );
    let uncached_track = track_summary(
        fixture_provider(),
        "track-2",
        "Track Two",
        Some("album-1"),
        Some("Album One"),
        Some("Artist One"),
    );
    let albums = filtered_cached_album_track_lists(
        vec![album_track_list(
            "album-1",
            "Album One",
            Some("Artist One"),
            None,
            &[cached_track, uncached_track],
        )],
        &HashSet::from([track_cache_key(&track_summary(
            fixture_provider(),
            "track-1",
            "Track One",
            Some("album-1"),
            Some("Album One"),
            Some("Artist One"),
        ))]),
    );

    let artists = build_local_artist_lists(&albums);

    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].collection.title, "Artist One");
    assert_eq!(artists[0].tracks.len(), 2);
    assert_eq!(artists[0].tracks[0].title, "Track One");
    assert_eq!(artists[0].tracks[1].title, "Track Two");
}

fn album_track_list(
    id: &str,
    title: &str,
    subtitle: Option<&str>,
    artwork_url: Option<&str>,
    tracks: &[TrackSummary],
) -> TrackList {
    TrackList {
        collection: CollectionSummary {
            reference: CollectionRef::new(fixture_provider(), id, CollectionKind::Album, None),
            title: title.to_string(),
            subtitle: subtitle.map(str::to_string),
            artwork_url: artwork_url.map(str::to_string),
            track_count: Some(tracks.len()),
        },
        tracks: tracks.to_vec(),
    }
}

fn track_summary(
    provider: ProviderId,
    id: &str,
    title: &str,
    collection_id: Option<&str>,
    album: Option<&str>,
    artist: Option<&str>,
) -> TrackSummary {
    TrackSummary {
        reference: TrackRef::new(provider, id, None, Some(title.to_string())),
        title: title.to_string(),
        artist: artist.map(str::to_string),
        album: album.map(str::to_string),
        collection_id: collection_id.map(str::to_string),
        collection_title: album.map(str::to_string),
        collection_subtitle: artist.map(str::to_string),
        duration_seconds: None,
        bitrate_bps: None,
        audio_format: None,
        artwork_url: None,
    }
}
