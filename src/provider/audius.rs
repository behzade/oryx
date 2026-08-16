use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::{
    CollectionKind, CollectionRef, CollectionSummary, DownloadRequest, MusicProvider,
    PlaybackCachePolicy, ProviderId, SearchResult, SongData, StreamRequest, TrackList, TrackRef,
    TrackSummary, network_agent,
};

const API_BASE: &str = "https://api.audius.co/v1";
const APP_NAME: &str = "Oryx";
const SEARCH_LIMIT: &str = "20";

#[derive(Clone, Debug, Default)]
pub struct AudiusProvider;

impl AudiusProvider {
    pub fn new() -> Self {
        Self
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let url = format!("{API_BASE}{path}");
        let mut request = network_agent().get(&url).query("app_name", APP_NAME);
        for (name, value) in query {
            request = request.query(name, value);
        }

        let response = request.call().map_err(|error| match error {
            ureq::Error::Status(status, response) => {
                audius_status_error(path, status, response.header("cf-ray").is_some())
            }
            ureq::Error::Transport(error) => {
                anyhow!(error).context(format!("Audius request failed for {path}"))
            }
        })?;
        let body = response
            .into_string()
            .context("Failed to read the Audius response")?;
        serde_json::from_str(&body).context("Failed to parse the Audius response")
    }

    fn track(&self, id: &str) -> Result<AudiusTrack> {
        validate_resource_id(id)?;
        self.get::<ApiResponse<OneOrMany<AudiusTrack>>>(&format!("/tracks/{id}"), &[])?
            .data
            .into_one()
            .context("Audius returned no track")
    }

    fn playlist(&self, id: &str) -> Result<AudiusPlaylist> {
        validate_resource_id(id)?;
        self.get::<ApiResponse<OneOrMany<AudiusPlaylist>>>(&format!("/playlists/{id}"), &[])?
            .data
            .into_one()
            .context("Audius returned no playlist")
    }
}

#[async_trait]
impl MusicProvider for AudiusProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Audius
    }

    fn playback_cache_policy(&self) -> PlaybackCachePolicy {
        PlaybackCachePolicy::SessionOnly
    }

    fn allows_download(&self, _track: &TrackSummary) -> bool {
        false
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let tracks = self
            .get::<ApiResponse<Vec<AudiusTrack>>>(
                "/tracks/search",
                &[("query", query), ("limit", SEARCH_LIMIT)],
            )?
            .data;
        let playlists = self
            .get::<ApiResponse<Vec<AudiusPlaylist>>>(
                "/playlists/search",
                &[("query", query), ("limit", SEARCH_LIMIT)],
            )?
            .data;

        let mut results = tracks
            .into_iter()
            .filter(AudiusTrack::is_playable)
            .map(|track| SearchResult::Track(track.into_summary(None)))
            .collect::<Vec<_>>();
        results.extend(
            playlists
                .into_iter()
                .map(|playlist| SearchResult::Collection(playlist.into_summary())),
        );
        Ok(results)
    }

    async fn get_track_list(&self, collection: &CollectionRef) -> Result<TrackList> {
        ensure_audius_provider(collection.provider)?;
        let playlist = self.playlist(&collection.id)?;
        let summary = playlist.into_summary();
        let tracks = self
            .get::<ApiResponse<Vec<AudiusTrack>>>(
                &format!("/playlists/{}/tracks", collection.id),
                &[],
            )?
            .data
            .into_iter()
            .filter(AudiusTrack::is_playable)
            .map(|track| track.into_summary(Some(&summary)))
            .collect();

        Ok(TrackList {
            collection: summary,
            tracks,
        })
    }

    async fn get_song_data(&self, track: &TrackRef) -> Result<SongData> {
        ensure_audius_provider(track.provider)?;
        let resolved = self.track(&track.id)?;
        if !resolved.is_playable() {
            bail!("Audius does not allow this track to stream");
        }

        let id = resolved.id.clone();
        Ok(SongData {
            track: resolved.into_summary(None),
            stream: StreamRequest {
                url: format!("{API_BASE}/tracks/{id}/stream?app_name={APP_NAME}"),
                headers: Vec::new(),
                supports_byte_ranges: true,
            },
        })
    }

    fn get_artwork_request(&self, artwork_url: &str) -> Option<DownloadRequest> {
        if !artwork_url.starts_with("https://") && !artwork_url.starts_with("http://") {
            return None;
        }

        Some(DownloadRequest {
            url: artwork_url.to_string(),
            headers: Vec::new(),
            mime_type: artwork_mime_type(artwork_url).map(str::to_string),
            supports_byte_ranges: true,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    fn into_one(self) -> Option<T> {
        match self {
            Self::One(value) => Some(value),
            Self::Many(values) => values.into_iter().next(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AudiusUser {
    name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AudiusArtwork {
    #[serde(rename = "150x150")]
    small: Option<String>,
    #[serde(rename = "480x480")]
    medium: Option<String>,
    #[serde(rename = "1000x1000")]
    large: Option<String>,
}

impl AudiusArtwork {
    fn best(self) -> Option<String> {
        self.medium.or(self.large).or(self.small)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AudiusAlbumBacklink {
    playlist_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct AudiusTrack {
    id: String,
    title: String,
    duration: Option<u32>,
    permalink: Option<String>,
    #[serde(default)]
    user: AudiusUser,
    artwork: Option<AudiusArtwork>,
    is_streamable: Option<bool>,
    album_backlink: Option<AudiusAlbumBacklink>,
}

impl AudiusTrack {
    fn is_playable(&self) -> bool {
        self.is_streamable != Some(false)
    }

    fn into_summary(self, collection: Option<&CollectionSummary>) -> TrackSummary {
        let album = self
            .album_backlink
            .and_then(|album| album.playlist_name)
            .or_else(|| collection.map(|collection| collection.title.clone()));
        TrackSummary {
            reference: TrackRef::new(
                ProviderId::Audius,
                self.id,
                canonical_url(self.permalink),
                Some(self.title.clone()),
            ),
            title: self.title,
            artist: self.user.name,
            album,
            collection_id: collection.map(|collection| collection.reference.id.clone()),
            collection_title: collection.map(|collection| collection.title.clone()),
            collection_subtitle: collection.and_then(|collection| collection.subtitle.clone()),
            duration_seconds: self.duration,
            bitrate_bps: None,
            audio_format: None,
            artwork_url: self.artwork.and_then(AudiusArtwork::best),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct AudiusPlaylist {
    id: String,
    playlist_name: String,
    is_album: bool,
    permalink: Option<String>,
    #[serde(default)]
    user: AudiusUser,
    artwork: Option<AudiusArtwork>,
    track_count: Option<usize>,
}

impl AudiusPlaylist {
    fn into_summary(self) -> CollectionSummary {
        CollectionSummary {
            reference: CollectionRef::new(
                ProviderId::Audius,
                self.id,
                if self.is_album {
                    CollectionKind::Album
                } else {
                    CollectionKind::Playlist
                },
                canonical_url(self.permalink),
            ),
            title: self.playlist_name,
            subtitle: self.user.name,
            artwork_url: self.artwork.and_then(AudiusArtwork::best),
            track_count: self.track_count,
        }
    }
}

fn ensure_audius_provider(provider: ProviderId) -> Result<()> {
    if provider != ProviderId::Audius {
        bail!("Audius cannot resolve provider '{provider}'");
    }
    Ok(())
}

fn validate_resource_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("Invalid Audius resource id");
    }
    Ok(())
}

fn canonical_url(permalink: Option<String>) -> Option<String> {
    permalink.map(|permalink| {
        if permalink.starts_with("http://") || permalink.starts_with("https://") {
            permalink
        } else {
            format!("https://audius.co{}", ensure_leading_slash(&permalink))
        }
    })
}

fn ensure_leading_slash(value: &str) -> String {
    if value.starts_with('/') {
        value.to_string()
    } else {
        format!("/{value}")
    }
}

fn artwork_mime_type(url: &str) -> Option<&'static str> {
    let path = url.split('?').next()?.to_ascii_lowercase();
    if path.ends_with(".png") {
        Some("image/png")
    } else if path.ends_with(".webp") {
        Some("image/webp")
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        Some("image/jpeg")
    } else {
        None
    }
}

fn audius_status_error(path: &str, status: u16, cloudflare: bool) -> anyhow::Error {
    if status == 403 && cloudflare {
        anyhow!(
            "Audius blocked this network request (HTTP 403 from Cloudflare). Try another network or disable Audius in Sources."
        )
    } else {
        anyhow!("Audius returned HTTP {status} for {path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_playable_track() {
        let track: AudiusTrack = serde_json::from_str(
            r#"{
                "id":"mWB22",
                "title":"Night Drive",
                "duration":183,
                "permalink":"/artist/night-drive",
                "user":{"name":"Artist"},
                "artwork":{"480x480":"https://images.example/cover.jpg"},
                "is_streamable":true,
                "album_backlink":{"playlist_name":"Album"}
            }"#,
        )
        .unwrap();

        let summary = track.into_summary(None);
        assert_eq!(summary.reference.provider, ProviderId::Audius);
        assert_eq!(summary.reference.id, "mWB22");
        assert_eq!(
            summary.reference.canonical_url.as_deref(),
            Some("https://audius.co/artist/night-drive")
        );
        assert_eq!(summary.artist.as_deref(), Some("Artist"));
        assert_eq!(summary.album.as_deref(), Some("Album"));
        assert_eq!(summary.duration_seconds, Some(183));
    }

    #[test]
    fn rejects_unsafe_resource_ids() {
        assert!(validate_resource_id("mWB22").is_ok());
        assert!(validate_resource_id("../tracks").is_err());
        assert!(validate_resource_id("").is_err());
    }

    #[test]
    fn maps_album_kind() {
        let playlist: AudiusPlaylist = serde_json::from_str(
            r#"{
                "id":"album1",
                "playlist_name":"An Album",
                "is_album":true,
                "permalink":"/artist/album/an-album",
                "user":{"name":"Artist"},
                "track_count":8
            }"#,
        )
        .unwrap();

        let summary = playlist.into_summary();
        assert_eq!(summary.reference.kind, CollectionKind::Album);
        assert_eq!(summary.track_count, Some(8));
    }

    #[test]
    fn uses_session_cache_and_blocks_offline_downloads() {
        let provider = AudiusProvider::new();
        let track = TrackSummary::unresolved(TrackRef::new(
            ProviderId::Audius,
            "mWB22",
            None,
            Some("Ambient Snow".to_string()),
        ));

        assert_eq!(
            provider.playback_cache_policy(),
            PlaybackCachePolicy::SessionOnly
        );
        assert!(!provider.allows_download(&track));
        assert_eq!(ProviderId::parse("audius"), Some(ProviderId::Audius));
    }

    #[test]
    fn explains_cloudflare_blocks() {
        let error = audius_status_error("/tracks/search", 403, true);
        assert!(error.to_string().contains("HTTP 403 from Cloudflare"));
        assert!(error.to_string().contains("Try another network"));
    }
}
