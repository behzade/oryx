use super::*;
use crate::provider::{CollectionSummary, TrackRef, TrackSummary};

fn fixture_provider() -> ProviderId {
    ProviderId::parse("fixture_remote").expect("fixture provider id should parse")
}

#[test]
fn provider_collection_ref_for_local_album_prefers_track_provider_and_collection_id() {
    let local_album = TrackList {
        collection: CollectionSummary {
            reference: CollectionRef::new(
                ProviderId::Local,
                "local-album-1",
                CollectionKind::Album,
                None,
            ),
            title: "Collection One".to_string(),
            subtitle: Some("Creator One".to_string()),
            artwork_url: None,
            track_count: Some(2),
        },
        tracks: vec![track_summary(
            fixture_provider(),
            "track-1",
            "Track One",
            Some("collection-one"),
            Some("Collection One"),
            Some("Creator One"),
        )],
    };

    let provider_collection =
        provider_collection_ref_for_local_album(&local_album).expect("provider collection");

    assert_eq!(provider_collection.provider, fixture_provider());
    assert_eq!(provider_collection.id, "collection-one");
    assert_eq!(provider_collection.canonical_url.as_deref(), None);
}

#[test]
fn provider_collection_ref_for_local_album_returns_none_for_local_only_collections() {
    let local_album = TrackList {
        collection: CollectionSummary {
            reference: CollectionRef::new(
                ProviderId::Local,
                "local-album-1",
                CollectionKind::Album,
                None,
            ),
            title: "Offline Album".to_string(),
            subtitle: Some("Offline Artist".to_string()),
            artwork_url: None,
            track_count: Some(1),
        },
        tracks: vec![track_summary(
            ProviderId::Local,
            "track-1",
            "Offline Track",
            Some("local-album-1"),
            Some("Offline Album"),
            Some("Offline Artist"),
        )],
    };

    assert_eq!(provider_collection_ref_for_local_album(&local_album), None);
}

#[test]
fn collection_browser_key_uses_provider_and_remote_id() {
    let provider = fixture_provider();
    let album = CollectionRef::new(provider, "album-1", CollectionKind::Album, None);

    assert_eq!(collection_browser_key(&album), "fixture_remote:album-1");
    assert_eq!(
        collection_entity_key(&album),
        "album:fixture_remote:album-1"
    );
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
