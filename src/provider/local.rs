use anyhow::{Result, bail};
use async_trait::async_trait;

use super::{
    CollectionRef, DownloadRequest, MusicProvider, ProviderId, SearchResult, SongData,
    StreamRequest, TrackList, TrackRef, TrackSummary,
};

pub struct LocalProvider;

impl LocalProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MusicProvider for LocalProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Local
    }

    async fn search(&self, _query: &str) -> Result<Vec<SearchResult>> {
        bail!("Local provider does not support search")
    }

    async fn get_track_list(&self, _collection: &CollectionRef) -> Result<TrackList> {
        bail!("Local provider track lists are loaded from the managed library")
    }

    async fn get_song_data(&self, track: &TrackRef) -> Result<SongData> {
        let url = track
            .canonical_url
            .clone()
            .unwrap_or_else(|| track.id.clone());

        Ok(SongData {
            track: TrackSummary::unresolved(track.clone()),
            stream: StreamRequest {
                url,
                headers: Vec::new(),
                supports_byte_ranges: true,
            },
        })
    }

    fn get_artwork_request(&self, _artwork_url: &str) -> Option<DownloadRequest> {
        None
    }
}
