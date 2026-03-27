use std::collections::HashSet;

use crate::provider::{AudioFormat, TrackList, TrackSummary};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) struct AudioQuality {
    pub(in crate::app) audio_format: Option<AudioFormat>,
    pub(in crate::app) bitrate_bps: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum AudioQualityGrade {
    Lossless,
    High,
    Standard,
    Low,
}

impl AudioQualityGrade {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Lossless => "Lossless",
            Self::High => "High",
            Self::Standard => "Standard",
            Self::Low => "Low",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Standard => 1,
            Self::High => 2,
            Self::Lossless => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum CollectionQualitySummary {
    Uniform(AudioQualityGrade),
    Range {
        lowest: AudioQualityGrade,
        highest: AudioQualityGrade,
    },
}

pub(in crate::app) fn normalized_audio_quality(quality: &AudioQuality) -> Option<AudioQuality> {
    if quality.audio_format.is_none() && quality.bitrate_bps.is_none() {
        return None;
    }

    let audio_format = quality.audio_format.clone();
    let bitrate_bps = match audio_format.as_ref() {
        Some(AudioFormat::Flac) => None,
        Some(format) => quality
            .bitrate_bps
            .map(|bitrate_bps| normalize_lossy_bitrate_bps(format, bitrate_bps)),
        None => quality.bitrate_bps.map(normalize_unknown_bitrate_bps),
    };

    Some(AudioQuality {
        audio_format,
        bitrate_bps,
    })
}

pub(in crate::app) fn normalized_audio_quality_grade(
    quality: &AudioQuality,
) -> Option<AudioQualityGrade> {
    let quality = normalized_audio_quality(quality)?;

    match (quality.audio_format, quality.bitrate_bps) {
        (Some(AudioFormat::Flac), _) => Some(AudioQualityGrade::Lossless),
        (Some(AudioFormat::Opus), Some(bitrate_bps)) => Some(match bitrate_bps / 1000 {
            0..=127 => AudioQualityGrade::Low,
            128..=159 => AudioQualityGrade::Standard,
            _ => AudioQualityGrade::High,
        }),
        (Some(AudioFormat::Aac | AudioFormat::M4a), Some(bitrate_bps)) => {
            Some(match bitrate_bps / 1000 {
                0..=159 => AudioQualityGrade::Low,
                160..=255 => AudioQualityGrade::Standard,
                _ => AudioQualityGrade::High,
            })
        }
        (Some(AudioFormat::Mp3), Some(bitrate_bps)) => Some(match bitrate_bps / 1000 {
            0..=191 => AudioQualityGrade::Low,
            192..=319 => AudioQualityGrade::Standard,
            _ => AudioQualityGrade::High,
        }),
        (Some(AudioFormat::Unknown(_)), Some(bitrate_bps)) | (None, Some(bitrate_bps)) => {
            Some(match bitrate_bps / 1000 {
                0..=159 => AudioQualityGrade::Low,
                160..=255 => AudioQualityGrade::Standard,
                _ => AudioQualityGrade::High,
            })
        }
        (Some(_), None) => None,
        (None, None) => None,
    }
}

pub(in crate::app) fn normalized_audio_quality_label(quality: &AudioQuality) -> Option<String> {
    let quality = normalized_audio_quality(quality)?;

    match (quality.audio_format, quality.bitrate_bps) {
        (Some(AudioFormat::Flac), _) => Some("FLAC".to_string()),
        (Some(format), Some(bitrate_bps)) => {
            Some(format!("{} {}k", format.label(), bitrate_bps / 1000))
        }
        (Some(format), None) => Some(format.label().to_string()),
        (None, Some(bitrate_bps)) => Some(format!("{}k", bitrate_bps / 1000)),
        (None, None) => None,
    }
}

pub(in crate::app) fn summarize_audio_quality_grades(
    grades: impl IntoIterator<Item = AudioQualityGrade>,
) -> Option<CollectionQualitySummary> {
    let mut unique_grades = grades.into_iter().collect::<Vec<_>>();
    if unique_grades.is_empty() {
        return None;
    }

    unique_grades.sort_by_key(|grade| grade.rank());
    unique_grades.dedup();

    if unique_grades.len() == 1 {
        return Some(CollectionQualitySummary::Uniform(unique_grades[0]));
    }

    Some(CollectionQualitySummary::Range {
        lowest: *unique_grades
            .first()
            .expect("at least one grade should exist"),
        highest: *unique_grades
            .last()
            .expect("at least one grade should exist"),
    })
}

pub(in crate::app) fn summarize_track_list_quality(
    track_list: &TrackList,
) -> Option<CollectionQualitySummary> {
    summarize_collection_quality(track_list.tracks.iter())
}

pub(in crate::app) fn summarize_collection_quality<'a>(
    tracks: impl Iterator<Item = &'a TrackSummary>,
) -> Option<CollectionQualitySummary> {
    let mut qualities = HashSet::new();
    for track in tracks {
        if let Some(quality) = audio_quality_from_track_summary(track) {
            if let Some(grade) = normalized_audio_quality_grade(&quality) {
                qualities.insert(grade);
            }
        }
    }

    summarize_audio_quality_grades(qualities)
}

pub(in crate::app) fn audio_quality_from_track_summary(
    track: &TrackSummary,
) -> Option<AudioQuality> {
    audio_quality_from_parts(track.audio_format.clone(), track.bitrate_bps)
}

fn audio_quality_from_parts(
    audio_format: Option<AudioFormat>,
    bitrate_bps: Option<u32>,
) -> Option<AudioQuality> {
    if audio_format.is_none() && bitrate_bps.is_none() {
        return None;
    }

    Some(AudioQuality {
        audio_format,
        bitrate_bps,
    })
}

fn normalize_lossy_bitrate_bps(format: &AudioFormat, bitrate_bps: u32) -> u32 {
    let tiers_kbps: &[u32] = match format {
        AudioFormat::Mp3 => &[96, 128, 160, 192, 224, 256, 320],
        AudioFormat::Aac | AudioFormat::M4a => &[96, 128, 160, 192, 256, 320],
        AudioFormat::Opus => &[96, 128, 160, 192, 256, 320],
        AudioFormat::Unknown(_) => &[96, 128, 160, 192, 224, 256, 320],
        AudioFormat::Flac => return bitrate_bps,
    };

    nearest_bitrate_tier_bps(bitrate_bps, tiers_kbps)
}

fn normalize_unknown_bitrate_bps(bitrate_bps: u32) -> u32 {
    let rounded_kbps = ((bitrate_bps + 500) / 1000).max(1);
    rounded_kbps * 1000
}

fn nearest_bitrate_tier_bps(bitrate_bps: u32, tiers_kbps: &[u32]) -> u32 {
    let measured_kbps = ((bitrate_bps + 500) / 1000).max(1);
    let nearest_kbps = tiers_kbps
        .iter()
        .copied()
        .min_by_key(|tier_kbps| measured_kbps.abs_diff(*tier_kbps))
        .unwrap_or(measured_kbps);
    nearest_kbps * 1000
}

#[cfg(test)]
#[path = "quality_tests.rs"]
mod tests;
