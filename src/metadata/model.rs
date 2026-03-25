#![allow(dead_code)]

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataTrackInput {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataResolution {
    pub source: MetadataSource,
    pub confidence: MetadataConfidence,
    pub album: Option<ResolvedAlbumMetadata>,
    pub tracks: Vec<ResolvedTrackMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedAlbumMetadata {
    pub title: String,
    pub artist: String,
    pub artwork_url: Option<String>,
    pub release_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTrackMetadata {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub disc_number: Option<u32>,
    pub track_number: Option<u32>,
    pub recording_id: Option<String>,
    pub release_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum MetadataConfidence {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataSource {
    LocalTags,
    AcoustId,
    MusicBrainz,
}
