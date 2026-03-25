use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::provider::{AudioFormat, ProviderId, TrackSummary};

#[derive(Clone, Debug)]
pub struct Track {
    pub provider: ProviderId,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork_path: Option<PathBuf>,
    pub duration_seconds: Option<u32>,
    pub duration_label: String,
    pub bitrate_bps: Option<u32>,
    pub audio_format: Option<AudioFormat>,
    pub source_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Idle,
    Buffering,
    Paused,
    Playing,
}

impl Default for PlaybackStatus {
    fn default() -> Self {
        Self::Idle
    }
}

impl PlaybackStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Buffering => "Buffering",
            Self::Paused => "Paused",
            Self::Playing => "Playing",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl Default for RepeatMode {
    fn default() -> Self {
        Self::Off
    }
}

impl RepeatMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "Repeat Off",
            Self::All => "Repeat All",
            Self::One => "Repeat One",
        }
    }

    pub fn cycle(&self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

impl Track {
    pub fn from_provider_track(track: TrackSummary) -> Self {
        Self {
            provider: track.reference.provider,
            title: track.title,
            artist: track.artist.unwrap_or_else(|| "Unknown artist".to_string()),
            album: track.album.unwrap_or_else(|| "Unknown album".to_string()),
            artwork_path: None,
            duration_seconds: track.duration_seconds,
            duration_label: format_duration(track.duration_seconds),
            bitrate_bps: track.bitrate_bps,
            audio_format: track.audio_format,
            source_url: track.reference.canonical_url.unwrap_or_default(),
        }
    }

    pub fn from_track_summary_with_source(
        track: TrackSummary,
        source_url: String,
        artwork_path: Option<PathBuf>,
    ) -> Self {
        let mut current = Self::from_provider_track(track);
        current.source_url = source_url;
        current.artwork_path = artwork_path;
        current
    }
}

fn format_duration(duration_seconds: Option<u32>) -> String {
    let Some(total_seconds) = duration_seconds else {
        return "--:--".to_string();
    };

    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}
