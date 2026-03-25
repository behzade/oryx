use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use lofty::prelude::AudioFile;
use serde::Deserialize;

use super::cover_art::CoverArtClient;
use super::model::{
    MetadataConfidence, MetadataResolution, MetadataSource, MetadataTrackInput,
    ResolvedAlbumMetadata, ResolvedTrackMetadata,
};

const MUSICBRAINZ_SEARCH_URL: &str = "https://musicbrainz.org/ws/2/recording";
const MUSICBRAINZ_RELEASE_URL: &str = "https://musicbrainz.org/ws/2/release";
const MUSICBRAINZ_USER_AGENT: &str = "Oryx/0.1 (local metadata import)";
const MIN_RECORDING_SCORE: u32 = 85;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Default)]
pub struct MusicBrainzClient {
    cover_art: CoverArtClient,
}

impl MusicBrainzClient {
    pub fn new() -> Self {
        Self {
            cover_art: CoverArtClient::new(),
        }
    }

    pub fn resolve_album_from_files(
        &self,
        tracks: &[MetadataTrackInput],
    ) -> Result<Option<MetadataResolution>> {
        if tracks.is_empty() {
            return Ok(None);
        }

        let mut release_scores = BTreeMap::<String, ReleaseScore>::new();
        let mut per_track_candidates = Vec::with_capacity(tracks.len());

        for track in tracks {
            let inferred = infer_track_query(&track.path)?;
            let candidates = self.search_recordings(&inferred)?;
            if candidates.is_empty() {
                return Ok(None);
            }

            for candidate in &candidates {
                release_scores
                    .entry(candidate.release.id.clone())
                    .or_insert_with(|| ReleaseScore::new(candidate.release.clone()))
                    .register_hit(candidate.score);
            }

            per_track_candidates.push((track.path.clone(), inferred, candidates));
        }

        let Some((best_release_id, score)) = release_scores
            .into_iter()
            .filter(|(_, score)| score.hits == tracks.len())
            .max_by_key(|(_, score)| (score.hits, score.total_score))
        else {
            return Ok(None);
        };

        let Some(release) = self.lookup_release(&best_release_id)? else {
            return Ok(None);
        };

        let mut used_track_ids = HashSet::new();
        let mut resolved_tracks = Vec::with_capacity(tracks.len());
        for (path, inferred, _candidates) in per_track_candidates {
            let Some(track) = best_release_match(&release, &inferred, &used_track_ids) else {
                return Ok(None);
            };
            used_track_ids.insert(track.recording.id.clone());

            resolved_tracks.push(ResolvedTrackMetadata {
                path,
                title: track.recording.title.clone(),
                artist: display_artist_credit(&track.recording.artist_credit)
                    .or_else(|| display_artist_credit(&release.artist_credit))
                    .unwrap_or_else(|| score.release.artist.clone()),
                album: release.title.clone(),
                album_artist: display_artist_credit(&release.artist_credit)
                    .unwrap_or_else(|| score.release.artist.clone()),
                disc_number: Some(track.medium_position),
                track_number: Some(track.position),
                recording_id: Some(track.recording.id.clone()),
                release_id: Some(release.id.clone()),
            });
        }

        let artwork_url = self
            .cover_art
            .fetch_release_artwork_url(&release.id)
            .ok()
            .flatten();

        Ok(Some(MetadataResolution {
            source: MetadataSource::MusicBrainz,
            confidence: MetadataConfidence::High,
            album: Some(ResolvedAlbumMetadata {
                title: release.title.clone(),
                artist: display_artist_credit(&release.artist_credit)
                    .unwrap_or_else(|| score.release.artist.clone()),
                artwork_url,
                release_id: Some(release.id),
            }),
            tracks: resolved_tracks,
        }))
    }

    pub fn resolve_track_from_file(&self, path: &Path) -> Result<Option<ResolvedTrackMetadata>> {
        let inferred = infer_track_query(path)?;
        let Some(candidate) = self.search_recordings(&inferred)?.into_iter().next() else {
            return Ok(None);
        };

        Ok(Some(ResolvedTrackMetadata {
            path: path.to_path_buf(),
            title: candidate.recording.title,
            artist: candidate
                .recording
                .artist
                .unwrap_or_else(|| candidate.release.artist.clone()),
            album: candidate.release.title.clone(),
            album_artist: candidate.release.artist,
            disc_number: None,
            track_number: None,
            recording_id: Some(candidate.recording.id),
            release_id: Some(candidate.release.id),
        }))
    }

    pub fn resolve_release_tracks(
        &self,
        release_id: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<ResolvedTrackMetadata>> {
        let Some(release) = self.lookup_release(release_id)? else {
            return Ok(Vec::new());
        };

        let release_artist = display_artist_credit(&release.artist_credit).unwrap_or_default();
        let mut used_track_ids = HashSet::new();
        let mut resolved_tracks = Vec::new();

        for path in paths {
            let inferred = infer_track_query(path)?;
            let Some(track) = best_release_match(&release, &inferred, &used_track_ids) else {
                continue;
            };
            used_track_ids.insert(track.recording.id.clone());

            resolved_tracks.push(ResolvedTrackMetadata {
                path: path.clone(),
                title: track.recording.title.clone(),
                artist: display_artist_credit(&track.recording.artist_credit)
                    .or_else(|| display_artist_credit(&release.artist_credit))
                    .unwrap_or_else(|| release_artist.clone()),
                album: release.title.clone(),
                album_artist: release_artist.clone(),
                disc_number: Some(track.medium_position),
                track_number: Some(track.position),
                recording_id: Some(track.recording.id.clone()),
                release_id: Some(release.id.clone()),
            });
        }

        Ok(resolved_tracks)
    }

    pub fn lookup_release(&self, release_id: &str) -> Result<Option<ReleaseLookupResponse>> {
        let response = ureq::AgentBuilder::new()
            .timeout(LOOKUP_TIMEOUT)
            .build()
            .get(&format!("{MUSICBRAINZ_RELEASE_URL}/{release_id}"))
            .set("User-Agent", MUSICBRAINZ_USER_AGENT)
            .query("inc", "recordings+artist-credits")
            .query("fmt", "json")
            .call()
            .context("Failed to call MusicBrainz release lookup API")?;

        let release: ReleaseLookupResponse = serde_json::from_reader(response.into_reader())
            .context("Failed to parse MusicBrainz release lookup response")?;

        Ok(Some(release))
    }

    fn search_recordings(&self, inferred: &TrackQuery) -> Result<Vec<RecordingCandidate>> {
        let mut query = format!("recording:\"{}\"", escape_lucene(&inferred.title));
        if let Some(duration_ms) = inferred.duration_ms {
            let min = duration_ms.saturating_sub(1_500);
            let max = duration_ms.saturating_add(1_500);
            query.push_str(&format!(" AND dur:[{min} TO {max}]"));
        }

        let response = ureq::AgentBuilder::new()
            .timeout(LOOKUP_TIMEOUT)
            .build()
            .get(MUSICBRAINZ_SEARCH_URL)
            .set("User-Agent", MUSICBRAINZ_USER_AGENT)
            .query("query", &query)
            .query("fmt", "json")
            .query("limit", "10")
            .call()
            .context("Failed to call MusicBrainz recording search API")?;

        let parsed: RecordingSearchResponse = serde_json::from_reader(response.into_reader())
            .context("Failed to parse MusicBrainz recording search response")?;

        let candidates = parsed
            .recordings
            .into_iter()
            .filter(|recording| recording.score >= MIN_RECORDING_SCORE)
            .flat_map(|recording| {
                let recording_artist = display_artist_credit(&recording.artist_credit);
                recording
                    .releases
                    .into_iter()
                    .map(move |release| RecordingCandidate {
                        score: recording.score,
                        recording: CandidateRecording {
                            id: recording.id.clone(),
                            title: recording.title.clone(),
                            artist: recording_artist.clone(),
                        },
                        release: CandidateRelease {
                            id: release.id,
                            title: release.title,
                            artist: display_artist_credit(&release.artist_credit)
                                .or_else(|| recording_artist.clone())
                                .unwrap_or_default(),
                        },
                    })
            })
            .collect::<Vec<_>>();

        Ok(candidates)
    }
}

#[derive(Clone, Debug)]
struct TrackQuery {
    title: String,
    normalized_title: String,
    duration_ms: Option<u32>,
}

#[derive(Clone, Debug)]
struct RecordingCandidate {
    score: u32,
    recording: CandidateRecording,
    release: CandidateRelease,
}

#[derive(Clone, Debug)]
struct CandidateRecording {
    id: String,
    title: String,
    artist: Option<String>,
}

#[derive(Clone, Debug)]
struct CandidateRelease {
    id: String,
    title: String,
    artist: String,
}

#[derive(Clone, Debug)]
struct ReleaseScore {
    release: CandidateRelease,
    hits: usize,
    total_score: u32,
}

impl ReleaseScore {
    fn new(release: CandidateRelease) -> Self {
        Self {
            release,
            hits: 0,
            total_score: 0,
        }
    }

    fn register_hit(&mut self, score: u32) {
        self.hits += 1;
        self.total_score += score;
    }
}

#[derive(Debug, Deserialize)]
struct RecordingSearchResponse {
    #[serde(default)]
    recordings: Vec<RecordingSearchRecording>,
}

#[derive(Debug, Deserialize)]
struct RecordingSearchRecording {
    id: String,
    title: String,
    #[serde(deserialize_with = "deserialize_score")]
    score: u32,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    releases: Vec<ReleaseSearchRelease>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseSearchRelease {
    id: String,
    title: String,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseLookupResponse {
    id: String,
    title: String,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    media: Vec<ReleaseMedium>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseMedium {
    #[serde(default)]
    position: u32,
    #[serde(default)]
    tracks: Vec<ReleaseTrack>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseTrack {
    #[serde(default)]
    position: u32,
    recording: ReleaseTrackRecording,
}

#[derive(Clone, Debug, Deserialize)]
struct ReleaseTrackRecording {
    id: String,
    title: String,
    #[serde(default)]
    length: Option<u32>,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
}

#[derive(Clone, Debug, Deserialize)]
struct ArtistCredit {
    name: String,
    joinphrase: Option<String>,
}

struct MatchedReleaseTrack<'a> {
    medium_position: u32,
    position: u32,
    recording: &'a ReleaseTrackRecording,
}

fn infer_track_query(path: &Path) -> Result<TrackQuery> {
    let title = infer_title_from_path(path)
        .with_context(|| format!("Could not infer a track title from {}", path.display()))?;
    let normalized_title = normalize_title(&title);
    let tagged_file = lofty::read_from_path(path)
        .with_context(|| format!("Failed to inspect audio file {}", path.display()))?;
    let duration_ms = Some(
        tagged_file
            .properties()
            .duration()
            .as_millis()
            .min(u128::from(u32::MAX)) as u32,
    );

    Ok(TrackQuery {
        title,
        normalized_title,
        duration_ms,
    })
}

fn infer_title_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.trim();
    let without_suffix = strip_numeric_suffix(stem).trim();
    if without_suffix.is_empty() {
        None
    } else {
        Some(without_suffix.to_string())
    }
}

fn strip_numeric_suffix(value: &str) -> &str {
    if let Some(open) = value.rfind('[') {
        let suffix = value[open..].trim();
        if suffix.starts_with('[')
            && suffix.ends_with(']')
            && suffix[1..suffix.len() - 1]
                .chars()
                .all(|ch| ch.is_ascii_digit())
        {
            return value[..open].trim_end();
        }
    }
    value
}

fn best_release_match<'a>(
    release: &'a ReleaseLookupResponse,
    inferred: &TrackQuery,
    used_track_ids: &HashSet<String>,
) -> Option<MatchedReleaseTrack<'a>> {
    let mut candidates = release
        .media
        .iter()
        .flat_map(|medium| {
            medium.tracks.iter().map(move |track| MatchedReleaseTrack {
                medium_position: medium.position.max(1),
                position: track.position.max(1),
                recording: &track.recording,
            })
        })
        .filter(|track| !used_track_ids.contains(&track.recording.id))
        .filter(|track| normalize_title(&track.recording.title) == inferred.normalized_title)
        .collect::<Vec<_>>();

    candidates.sort_by_key(|track| {
        let duration_delta = match (track.recording.length, inferred.duration_ms) {
            (Some(length), Some(target)) => length.abs_diff(target),
            _ => u32::MAX,
        };
        (duration_delta, track.medium_position, track.position)
    });

    candidates.into_iter().next()
}

fn display_artist_credit(credits: &[ArtistCredit]) -> Option<String> {
    if credits.is_empty() {
        return None;
    }

    let rendered = credits
        .iter()
        .map(|credit| {
            format!(
                "{}{}",
                credit.name,
                credit.joinphrase.as_deref().unwrap_or("")
            )
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

fn normalize_title(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_space = false;

    for ch in value.chars() {
        if ch.is_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_lucene(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '+' | '-'
                | '&'
                | '|'
                | '!'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '^'
                | '"'
                | '~'
                | '*'
                | '?'
                | ':'
                | '\\'
                | '/'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn deserialize_score<'de, D>(deserializer: D) -> std::result::Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ScoreValue {
        Integer(u32),
        String(String),
    }

    match ScoreValue::deserialize(deserializer)? {
        ScoreValue::Integer(value) => Ok(value),
        ScoreValue::String(value) => value.parse::<u32>().map_err(serde::de::Error::custom),
    }
}
