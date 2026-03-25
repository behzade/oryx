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
