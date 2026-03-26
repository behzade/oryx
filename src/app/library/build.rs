use std::collections::{BTreeMap, HashMap, HashSet};

use crate::library::CachedLibraryTrack;
use crate::provider::{
    CollectionKind, CollectionRef, CollectionSummary, ProviderId, TrackList, TrackSummary,
};

use super::super::track_cache_key;
use super::quality::{
    AudioQuality, AudioQualityGrade, CollectionQualitySummary, audio_quality_from_track,
    normalized_audio_quality_grade,
};

const SYSTEM_PLAYLIST_SUBTITLE: &str = "System playlist";

pub(in crate::app) fn build_cached_quality_maps(
    tracks: &[CachedLibraryTrack],
) -> (
    HashMap<String, AudioQuality>,
    HashMap<String, CollectionQualitySummary>,
) {
    let mut track_qualities = HashMap::new();
    let mut collection_qualities = HashMap::<String, HashSet<AudioQualityGrade>>::new();

    for cached_track in tracks {
        let Some(quality) = audio_quality_from_track(&cached_track.track) else {
            continue;
        };

        track_qualities.insert(track_cache_key(&cached_track.track), quality.clone());

        if let Some(collection_id) = cached_track.track.collection_id.as_deref() {
            let Some(grade) = normalized_audio_quality_grade(&quality) else {
                continue;
            };
            collection_qualities
                .entry(format!(
                    "{}:{collection_id}",
                    cached_track.collection_provider.as_str()
                ))
                .or_default()
                .insert(grade);
        }
    }

    let collection_summaries = collection_qualities
        .into_iter()
        .map(|(collection_key, qualities)| {
            let summary = if qualities.len() <= 1 {
                CollectionQualitySummary::Uniform(
                    qualities
                        .into_iter()
                        .next()
                        .expect("single quality should exist"),
                )
            } else {
                CollectionQualitySummary::Mixed
            };
            (collection_key, summary)
        })
        .collect();

    (track_qualities, collection_summaries)
}

pub(in crate::app) fn enrich_track_list_with_cached_qualities(
    track_list: &mut TrackList,
    cached_track_qualities: &HashMap<String, AudioQuality>,
) {
    for track in &mut track_list.tracks {
        if let Some(quality) = cached_track_qualities.get(&track_cache_key(track)) {
            track.audio_format = quality.audio_format.clone();
            track.bitrate_bps = quality.bitrate_bps;
        }
    }
}

pub(in crate::app) fn filtered_cached_library_tracks(
    tracks: Vec<CachedLibraryTrack>,
) -> Vec<CachedLibraryTrack> {
    tracks
}

pub(in crate::app) fn filtered_cached_album_track_lists(
    track_lists: Vec<TrackList>,
    cached_track_ids: &HashSet<String>,
) -> Vec<TrackList> {
    track_lists
        .into_iter()
        .filter(|track_list| track_list.collection.reference.kind == CollectionKind::Album)
        .filter(|track_list| {
            track_list.collection.subtitle.as_deref() != Some(SYSTEM_PLAYLIST_SUBTITLE)
        })
        .filter(|track_list| {
            track_list
                .tracks
                .iter()
                .any(|track| cached_track_ids.contains(&track_cache_key(track)))
        })
        .collect()
}

pub(in crate::app) fn build_local_artist_lists(albums: &[TrackList]) -> Vec<TrackList> {
    struct ArtistGroup {
        artwork_url: Option<String>,
        album_ids: HashSet<String>,
        tracks: Vec<TrackSummary>,
    }

    let mut sorted_albums = albums.to_vec();
    sorted_albums.sort_by(|left, right| {
        left.collection
            .title
            .to_lowercase()
            .cmp(&right.collection.title.to_lowercase())
    });

    let mut groups = BTreeMap::<String, ArtistGroup>::new();

    for album in sorted_albums {
        let fallback_artist = album.collection.subtitle.clone();
        for track in album.tracks {
            let artist = track
                .collection_subtitle
                .clone()
                .or_else(|| track.artist.clone())
                .or_else(|| fallback_artist.clone())
                .unwrap_or_else(|| "Unknown artist".to_string());
            let group = groups.entry(artist.clone()).or_insert_with(|| ArtistGroup {
                artwork_url: None,
                album_ids: HashSet::new(),
                tracks: Vec::new(),
            });
            group
                .album_ids
                .insert(album.collection.reference.id.clone());
            if group.artwork_url.is_none() {
                group.artwork_url = album
                    .collection
                    .artwork_url
                    .clone()
                    .or_else(|| track.artwork_url.clone());
            }
            group.tracks.push(track);
        }
    }

    groups
        .into_iter()
        .map(|(artist, group)| {
            let subtitle = match group.album_ids.len() {
                0 => "No albums".to_string(),
                1 => "1 album".to_string(),
                count => format!("{count} albums"),
            };
            TrackList {
                collection: CollectionSummary {
                    reference: CollectionRef::new(
                        ProviderId::Local,
                        format!("local-artist:{artist}"),
                        CollectionKind::Album,
                        None,
                    ),
                    title: artist,
                    subtitle: Some(subtitle),
                    artwork_url: group.artwork_url,
                    track_count: Some(group.tracks.len()),
                },
                tracks: group.tracks,
            }
        })
        .collect()
}
#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
