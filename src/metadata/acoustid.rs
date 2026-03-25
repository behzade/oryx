use std::collections::BTreeMap;
use std::env;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::fingerprint::FingerprintResolver;
use super::model::{
    MetadataConfidence, MetadataResolution, MetadataSource, MetadataTrackInput,
    ResolvedAlbumMetadata, ResolvedTrackMetadata,
};

const ACOUSTID_LOOKUP_URL: &str = "https://api.acoustid.org/v2/lookup";
const ACOUSTID_CLIENT_KEY_ENV: &str = "ORYX_ACOUSTID_CLIENT_KEY";
const HIGH_CONFIDENCE_SCORE: f64 = 0.90;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
pub struct AcoustIdClient {
    client_key: String,
    fingerprinter: FingerprintResolver,
}

impl AcoustIdClient {
    pub fn new(client_key: impl Into<String>) -> Self {
        Self {
            client_key: client_key.into(),
            fingerprinter: FingerprintResolver::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let client_key = env::var(ACOUSTID_CLIENT_KEY_ENV).with_context(|| {
            format!(
                "AcoustID client key is not configured; set {} to enable automatic metadata lookup",
                ACOUSTID_CLIENT_KEY_ENV
            )
        })?;
        Ok(Self::new(client_key))
    }

    pub fn from_env_optional() -> Result<Option<Self>> {
        match env::var(ACOUSTID_CLIENT_KEY_ENV) {
            Ok(client_key) => Ok(Some(Self::new(client_key))),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "Failed to read {} for AcoustID lookup",
                    ACOUSTID_CLIENT_KEY_ENV
                )
            }),
        }
    }

    pub fn resolve_album_from_files(
        &self,
        tracks: &[MetadataTrackInput],
    ) -> Result<Option<MetadataResolution>> {
        if tracks.is_empty() {
            return Ok(None);
        }

        let mut resolved_tracks = Vec::with_capacity(tracks.len());
        let mut album_keys = BTreeMap::<String, usize>::new();
        let mut best_album = None::<AlbumIdentity>;

        for track in tracks {
            let Some(resolved) = self.resolve_track_from_file(&track.path)? else {
                return Ok(None);
            };

            let key = if let Some(release_id) = resolved.release_id.as_deref() {
                format!("release:{release_id}")
            } else {
                format!("album:{}:{}", resolved.album_artist, resolved.album)
            };
            let count = album_keys.entry(key).or_default();
            *count += 1;

            best_album = Some(AlbumIdentity {
                title: resolved.album.clone(),
                artist: resolved.album_artist.clone(),
                release_id: resolved.release_id.clone(),
            });
            resolved_tracks.push(resolved);
        }

        if album_keys.len() != 1 {
            return Ok(None);
        }

        let Some(best_album) = best_album else {
            return Ok(None);
        };

        Ok(Some(MetadataResolution {
            source: MetadataSource::AcoustId,
            confidence: MetadataConfidence::High,
            album: Some(ResolvedAlbumMetadata {
                title: best_album.title,
                artist: best_album.artist,
                artwork_url: None,
                release_id: best_album.release_id,
            }),
            tracks: resolved_tracks,
        }))
    }

    pub fn resolve_track_from_file(&self, path: &Path) -> Result<Option<ResolvedTrackMetadata>> {
        let Some(fingerprint) = self.fingerprinter.fingerprint_file(path)? else {
            return Ok(None);
        };

        let response = ureq::AgentBuilder::new()
            .timeout(LOOKUP_TIMEOUT)
            .build()
            .get(ACOUSTID_LOOKUP_URL)
            .set("User-Agent", "Oryx/0.1")
            .query("client", &self.client_key)
            .query("duration", &fingerprint.duration_seconds.to_string())
            .query("fingerprint", &fingerprint.value)
            .query("meta", "recordings releases")
            .call()
            .context("Failed to call AcoustID lookup API")?;

        let parsed: LookupResponse = serde_json::from_reader(response.into_reader())
            .context("Failed to parse AcoustID lookup response")?;

        if parsed.status != "ok" {
            bail!("AcoustID lookup failed with status '{}'", parsed.status);
        }

        let Some(result) = parsed
            .results
            .into_iter()
            .find(|result| result.score >= HIGH_CONFIDENCE_SCORE)
        else {
            return Ok(None);
        };

        let Some(recording) = result.recordings.into_iter().next() else {
            return Ok(None);
        };
        let Some(release) = recording.releases.into_iter().next() else {
            return Ok(None);
        };

        let artist = display_artist_names(&recording.artists)
            .or_else(|| display_artist_names(&release.artists))
            .context("AcoustID recording is missing a usable artist")?;
        let album_artist = display_artist_names(&release.artists).unwrap_or_else(|| artist.clone());

        Ok(Some(ResolvedTrackMetadata {
            path: path.to_path_buf(),
            title: recording
                .title
                .filter(|value| !value.trim().is_empty())
                .context("AcoustID recording is missing a usable title")?,
            artist,
            album: release
                .title
                .filter(|value| !value.trim().is_empty())
                .context("AcoustID release is missing a usable album title")?,
            album_artist,
            disc_number: None,
            track_number: None,
            recording_id: Some(recording.id),
            release_id: release.id,
        }))
    }
}

#[derive(Clone, Debug)]
struct AlbumIdentity {
    title: String,
    artist: String,
    release_id: Option<String>,
}

fn display_artist_names(artists: &[Artist]) -> Option<String> {
    if artists.is_empty() {
        return None;
    }

    let rendered = artists
        .iter()
        .enumerate()
        .map(|(index, artist)| {
            let join = artist
                .joinphrase
                .as_deref()
                .unwrap_or_else(|| if index + 1 == artists.len() { "" } else { ", " });
            format!("{}{}", artist.name, join)
        })
        .collect::<String>()
        .trim()
        .to_string();

    if rendered.is_empty() {
        None
    } else {
        Some(rendered)
    }
}

#[derive(Debug, Deserialize)]
struct LookupResponse {
    status: String,
    #[serde(default)]
    results: Vec<LookupResult>,
}

#[derive(Debug, Deserialize)]
struct LookupResult {
    score: f64,
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Debug, Deserialize)]
struct Recording {
    id: String,
    title: Option<String>,
    #[serde(default)]
    artists: Vec<Artist>,
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct Release {
    id: Option<String>,
    title: Option<String>,
    #[serde(default)]
    artists: Vec<Artist>,
}

#[derive(Clone, Debug, Deserialize)]
struct Artist {
    name: String,
    joinphrase: Option<String>,
}
