use super::*;
use crate::provider::{CollectionKind, CollectionRef, CollectionSummary, TrackRef};

fn fixture_provider() -> ProviderId {
    ProviderId::parse("fixture_remote").expect("fixture provider id should parse")
}

#[test]
fn preserves_visible_track_list_when_refresh_drops_deleted_track_row() {
    let previous = track_list("album-1", &["track-1", "track-2"]);
    let current = track_list("album-1", &["track-2"]);

    assert!(should_preserve_visible_track_list_after_delete(
        BrowseMode::Albums,
        Some(&previous),
        Some(&current),
        fixture_provider(),
        "track-1",
    ));
}

#[test]
fn skips_override_when_refresh_keeps_deleted_track_row_visible() {
    let previous = track_list("album-1", &["track-1", "track-2"]);
    let current = track_list("album-1", &["track-1", "track-2"]);

    assert!(!should_preserve_visible_track_list_after_delete(
        BrowseMode::Albums,
        Some(&previous),
        Some(&current),
        fixture_provider(),
        "track-1",
    ));
}

fn track_list(collection_id: &str, track_ids: &[&str]) -> TrackList {
    TrackList {
        collection: CollectionSummary {
            reference: CollectionRef::new(
                fixture_provider(),
                collection_id,
                CollectionKind::Album,
                None,
            ),
            title: "Album".to_string(),
            subtitle: None,
            artwork_url: None,
            track_count: Some(track_ids.len()),
        },
        tracks: track_ids
            .iter()
            .map(|track_id| TrackSummary {
                reference: TrackRef::new(fixture_provider(), *track_id, None, None),
                title: (*track_id).to_string(),
                artist: None,
                album: Some("Album".to_string()),
                collection_id: Some(collection_id.to_string()),
                collection_title: Some("Album".to_string()),
                collection_subtitle: None,
                duration_seconds: None,
                bitrate_bps: None,
                audio_format: None,
                artwork_url: None,
            })
            .collect(),
    }
}
