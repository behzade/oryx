use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use lofty::picture::{MimeType, PictureType};
use lofty::prelude::{Accessor, AudioFile, ItemKey, TaggedFileExt};
use rusqlite::{Connection, params};
use serde::Deserialize;

use crate::library::Library;
use crate::metadata::{
    CoverArtClient, MetadataPolicy, MetadataResolver, MetadataTrackInput, MusicBrainzClient,
    ResolvedAlbumMetadata, ResolvedTrackMetadata,
};
use crate::provider::{ProviderId, TrackRef, TrackSummary};

const ARTWORK_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub struct ImportSummary {
    pub imported_tracks: usize,
    pub imported_albums: usize,
    pub skipped_files: usize,
    pub error_samples: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ArtworkBackfillSummary {
    pub inspected_albums: usize,
    pub updated_albums: usize,
    pub skipped_albums: usize,
    pub error_samples: Vec<String>,
}

impl ArtworkBackfillSummary {
    fn record_error(&mut self, message: impl Into<String>) {
        self.skipped_albums += 1;
        if self.error_samples.len() < 3 {
            self.error_samples.push(message.into());
        }
    }
}

impl ImportSummary {
    fn record_error(&mut self, message: impl Into<String>) {
        self.skipped_files += 1;
        if self.error_samples.len() < 3 {
            self.error_samples.push(message.into());
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportReview {
    pub source_root: PathBuf,
    pub staging_root: Option<PathBuf>,
    pub albums: Vec<ImportAlbumReview>,
}

impl ImportReview {
    pub fn ready_track_count(&self) -> usize {
        self.albums
            .iter()
            .map(ImportAlbumReview::ready_track_count)
            .sum()
    }

    pub fn unresolved_track_count(&self) -> usize {
        self.albums
            .iter()
            .map(ImportAlbumReview::unresolved_track_count)
            .sum()
    }

    pub fn skipped_track_count(&self) -> usize {
        self.albums
            .iter()
            .map(ImportAlbumReview::skipped_track_count)
            .sum()
    }

    pub fn matched_track_count(&self) -> usize {
        self.ready_track_count()
    }

    pub fn track_mut(&mut self, source_path: &Path) -> Option<&mut ImportTrackReview> {
        self.albums
            .iter_mut()
            .flat_map(|album| album.tracks.iter_mut())
            .find(|track| track.source_path == source_path)
    }

    pub fn refresh_album_summaries(&mut self) {
        for album in &mut self.albums {
            album.refresh_summary_from_tracks();
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportAlbumReview {
    pub source_label: String,
    pub detected_album: Option<ResolvedAlbumMetadata>,
    pub artwork_url: Option<String>,
    pub tracks: Vec<ImportTrackReview>,
    pub warnings: Vec<String>,
}

impl ImportAlbumReview {
    pub fn ready_track_count(&self) -> usize {
        self.tracks.iter().filter(|track| track.is_ready()).count()
    }

    pub fn unresolved_track_count(&self) -> usize {
        self.tracks
            .iter()
            .filter(|track| track.needs_attention())
            .count()
    }

    pub fn skipped_track_count(&self) -> usize {
        self.tracks.iter().filter(|track| track.skipped).count()
    }

    pub fn refresh_summary_from_tracks(&mut self) {
        let mut ready_tracks = self
            .tracks
            .iter()
            .filter(|track| !track.skipped)
            .filter_map(|track| track.detected_track.as_ref());
        let Some(first_track) = ready_tracks.next() else {
            self.detected_album = None;
            self.artwork_url = self
                .tracks
                .iter()
                .find_map(|track| track.artwork_url.clone());
            return;
        };

        let same_album = ready_tracks.all(|track| {
            track.album == first_track.album
                && track.album_artist == first_track.album_artist
                && track.release_id == first_track.release_id
        });

        self.artwork_url = self
            .tracks
            .iter()
            .find_map(|track| track.artwork_url.clone());
        self.detected_album = same_album.then(|| ResolvedAlbumMetadata {
            title: first_track.album.clone(),
            artist: first_track.album_artist.clone(),
            artwork_url: self.artwork_url.clone(),
            release_id: first_track.release_id.clone(),
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ImportMetadataField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    DiscNumber,
    TrackNumber,
}

impl ImportMetadataField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::AlbumArtist => "Album Artist",
            Self::DiscNumber => "Disc #",
            Self::TrackNumber => "Track #",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportMetadataSource {
    LocalTags,
    OnlineServices,
    ManualEntry,
}

impl ImportMetadataSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalTags => "Local tags",
            Self::OnlineServices => "Online services",
            Self::ManualEntry => "Manual",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportTrackMetadataDraft {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub disc_number: String,
    pub track_number: String,
}

impl ImportTrackMetadataDraft {
    fn missing_required_fields(&self) -> Vec<ImportMetadataField> {
        let mut missing = Vec::new();
        if self.title.trim().is_empty() {
            missing.push(ImportMetadataField::Title);
        }
        if self.artist.trim().is_empty() {
            missing.push(ImportMetadataField::Artist);
        }
        if self.album.trim().is_empty() {
            missing.push(ImportMetadataField::Album);
        }
        missing
    }

    fn into_resolved_track_metadata(self, path: PathBuf) -> Option<ResolvedTrackMetadata> {
        let title = self.title.trim().to_string();
        let artist = self.artist.trim().to_string();
        let album = self.album.trim().to_string();
        if title.is_empty() || artist.is_empty() || album.is_empty() {
            return None;
        }

        let album_artist = if self.album_artist.trim().is_empty() {
            artist.clone()
        } else {
            self.album_artist.trim().to_string()
        };

        Some(ResolvedTrackMetadata {
            path,
            title,
            artist,
            album,
            album_artist,
            disc_number: parse_optional_u32(&self.disc_number),
            track_number: parse_optional_u32(&self.track_number),
            recording_id: None,
            release_id: None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportTrackReview {
    pub source_path: PathBuf,
    pub analysis_path: PathBuf,
    pub original_title: String,
    pub detected_track: Option<ResolvedTrackMetadata>,
    pub metadata_source: Option<ImportMetadataSource>,
    pub artwork_url: Option<String>,
    pub manual_metadata: ImportTrackMetadataDraft,
    pub missing_fields: Vec<ImportMetadataField>,
    pub skipped: bool,
    pub manual_mode: bool,
    pub issue: Option<String>,
}

impl ImportTrackReview {
    pub fn is_ready(&self) -> bool {
        !self.skipped && self.detected_track.is_some()
    }

    pub fn needs_attention(&self) -> bool {
        !self.skipped && self.detected_track.is_none()
    }

    pub fn metadata_source_label(&self) -> Option<&'static str> {
        self.metadata_source.map(ImportMetadataSource::label)
    }

    pub fn mark_skipped(&mut self, skipped: bool) {
        self.skipped = skipped;
    }

    pub fn begin_manual_entry(&mut self) {
        self.skipped = false;
        self.manual_mode = true;
        self.refresh_from_manual_metadata();
    }

    pub fn apply_online_metadata(
        &mut self,
        metadata: ResolvedTrackMetadata,
        artwork_url: Option<String>,
    ) {
        self.skipped = false;
        self.manual_mode = false;
        self.detected_track = Some(metadata);
        self.metadata_source = Some(ImportMetadataSource::OnlineServices);
        self.artwork_url = artwork_url;
        self.missing_fields.clear();
        self.issue = None;
    }

    pub fn set_manual_field(&mut self, field: ImportMetadataField, value: String) {
        self.manual_mode = true;
        self.skipped = false;
        match field {
            ImportMetadataField::Title => self.manual_metadata.title = value,
            ImportMetadataField::Artist => self.manual_metadata.artist = value,
            ImportMetadataField::Album => self.manual_metadata.album = value,
            ImportMetadataField::AlbumArtist => self.manual_metadata.album_artist = value,
            ImportMetadataField::DiscNumber => self.manual_metadata.disc_number = value,
            ImportMetadataField::TrackNumber => self.manual_metadata.track_number = value,
        }
        self.refresh_from_manual_metadata();
    }

    fn refresh_from_manual_metadata(&mut self) {
        self.missing_fields = self.manual_metadata.missing_required_fields();
        self.detected_track = self
            .manual_metadata
            .clone()
            .into_resolved_track_metadata(self.analysis_path.clone());
        self.metadata_source = self
            .detected_track
            .as_ref()
            .map(|_| ImportMetadataSource::ManualEntry);
        self.artwork_url = None;
        self.issue = if self.detected_track.is_some() {
            None
        } else {
            Some(format!(
                "Provide {} to import this file.",
                format_missing_fields(&self.missing_fields)
            ))
        };
    }
}

#[derive(Clone, Debug)]
struct ImportSourceFile {
    source_path: PathBuf,
    analysis_path: PathBuf,
}

#[derive(Clone, Debug, Default)]
struct ImportMetadataContext {
    tracks_by_path: HashMap<PathBuf, ResolvedTrackMetadata>,
    album_artwork_url: Option<String>,
}

pub(super) fn stage_local_selection(
    library: &Library,
    selections: &[PathBuf],
    metadata_policy: MetadataPolicy,
) -> Result<ImportReview> {
    let (source_root, audio_files) = collect_import_audio_files(library, selections)?;
    let (import_files, staging_root) = normalize_import_files(library, &source_root, audio_files)?;
    let groups = group_audio_files_by_directory(&source_root, import_files);
    let mut albums = Vec::with_capacity(groups.len());

    for (source_dir, group_files) in groups {
        match stage_album_review(&source_root, &source_dir, &group_files, metadata_policy) {
            Ok(review) => albums.push(review),
            Err(error) => {
                if let Some(staging_root) = staging_root.as_ref() {
                    let _ = fs::remove_dir_all(staging_root);
                }
                return Err(error);
            }
        }
    }

    Ok(ImportReview {
        source_root,
        staging_root,
        albums,
    })
}

pub(super) fn commit_import_review(
    library: &Library,
    review: &ImportReview,
) -> Result<ImportSummary> {
    let mut connection = library.open_connection()?;
    let mut summary = ImportSummary::default();
    let mut imported_albums = HashSet::new();

    for album in &review.albums {
        for track in &album.tracks {
            if track.skipped {
                summary.skipped_files += 1;
                continue;
            }

            let Some(detected_track) = track.detected_track.clone() else {
                summary.record_error(format!(
                    "{}: {}",
                    track.source_path.display(),
                    track
                        .issue
                        .clone()
                        .unwrap_or_else(|| "No accepted import match.".to_string())
                ));
                continue;
            };

            match import_audio_file(
                library,
                &mut connection,
                &review.source_root,
                &track.analysis_path,
                &track.source_path,
                detected_track,
                track
                    .artwork_url
                    .as_deref()
                    .or(album.artwork_url.as_deref()),
            ) {
                Ok(Some(collection_id)) => {
                    summary.imported_tracks += 1;
                    imported_albums.insert(collection_id);
                }
                Ok(None) => {
                    summary.skipped_files += 1;
                }
                Err(error) => {
                    let message = format!("{}: {error:#}", track.source_path.display());
                    eprintln!("import skipped: {message}");
                    summary.record_error(message);
                }
            }
        }
    }

    summary.imported_albums = imported_albums.len();
    if let Some(staging_root) = review.staging_root.as_ref() {
        let _ = fs::remove_dir_all(staging_root);
    }
    Ok(summary)
}

pub(super) fn cleanup_import_review(review: &ImportReview) -> Result<()> {
    if let Some(staging_root) = review.staging_root.as_ref()
        && staging_root.is_dir()
    {
        fs::remove_dir_all(staging_root).with_context(|| {
            format!("Failed to remove import staging {}", staging_root.display())
        })?;
    }
    Ok(())
}

pub(super) fn resolve_import_track_online(
    analysis_path: &Path,
) -> Result<(ResolvedTrackMetadata, Option<String>)> {
    let metadata = MetadataResolver::new(MetadataPolicy::AutoResolveHighConfidence)
        .resolve_track_from_file(analysis_path)?
        .context("No online metadata match found for this file")?;
    let artwork_url = metadata
        .release_id
        .as_deref()
        .map(|release_id| CoverArtClient::new().fetch_release_artwork_url(release_id))
        .transpose()?
        .flatten();
    Ok((metadata, artwork_url))
}

pub(super) fn backfill_local_artwork(library: &Library) -> Result<ArtworkBackfillSummary> {
    let mut connection = library.open_connection()?;
    let mut statement = connection.prepare(
        r#"
        SELECT collection_id, audio_path, artwork_path
        FROM cached_tracks
        WHERE provider = ?1
        ORDER BY collection_id, audio_path
        "#,
    )?;
    let rows = statement.query_map([ProviderId::Local.as_str()], |row| {
        let collection_id: String = row.get(0)?;
        let audio_path: String = row.get(1)?;
        let artwork_path: Option<String> = row.get(2)?;
        Ok((
            collection_id,
            PathBuf::from(audio_path),
            artwork_path.map(PathBuf::from),
        ))
    })?;

    let mut albums = BTreeMap::<String, Vec<(PathBuf, Option<PathBuf>)>>::new();
    let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for row in rows {
        let (collection_id, audio_path, artwork_path) = row;
        albums
            .entry(collection_id)
            .or_default()
            .push((audio_path, artwork_path));
    }

    let mut summary = ArtworkBackfillSummary::default();
    eprintln!("artwork backfill: scanning local albums for missing covers");
    for (collection_id, tracks) in albums {
        let has_existing = tracks
            .iter()
            .any(|(_, artwork_path)| artwork_path.as_ref().is_some_and(|path| path.is_file()));
        if has_existing {
            continue;
        }
        summary.inspected_albums += 1;

        let Some(first_audio_path) = tracks.first().map(|(audio_path, _)| audio_path) else {
            summary.skipped_albums += 1;
            eprintln!("artwork backfill: skipped empty local album group {collection_id}");
            continue;
        };
        let Some(album_dir) = first_audio_path.parent() else {
            let message = format!(
                "{}: Missing album directory for local track",
                first_audio_path.display()
            );
            eprintln!("artwork backfill: {message}");
            summary.record_error(message);
            continue;
        };
        eprintln!(
            "artwork backfill: missing cover for {} ({} track(s))",
            album_dir.display(),
            tracks.len()
        );
        let audio_files = tracks
            .iter()
            .map(|(audio_path, _)| ImportSourceFile {
                source_path: audio_path.clone(),
                analysis_path: audio_path.clone(),
            })
            .collect::<Vec<_>>();

        match stage_album_review(
            album_dir,
            album_dir,
            &audio_files,
            MetadataPolicy::AutoResolveHighConfidence,
        ) {
            Ok(review) => {
                if let Some(album) = review.detected_album.as_ref() {
                    eprintln!(
                        "artwork backfill: detected album '{}' by '{}' for {}",
                        album.title,
                        album.artist,
                        album_dir.display()
                    );
                } else {
                    eprintln!(
                        "artwork backfill: could not determine album metadata for {}",
                        album_dir.display()
                    );
                }
                let Some(artwork_url) = review.artwork_url.as_deref() else {
                    summary.skipped_albums += 1;
                    eprintln!(
                        "artwork backfill: no artwork url found for {}",
                        album_dir.display()
                    );
                    continue;
                };
                eprintln!("artwork backfill: downloading cover from {}", artwork_url);

                match import_remote_picture_strict(artwork_url, album_dir) {
                    Ok(Some(artwork_path)) => {
                        update_collection_artwork_path(
                            &mut connection,
                            &collection_id,
                            artwork_path.as_path(),
                        )?;
                        summary.updated_albums += 1;
                        eprintln!(
                            "artwork backfill: saved cover to {}",
                            artwork_path.display()
                        );
                    }
                    Ok(None) => {
                        summary.skipped_albums += 1;
                        eprintln!(
                            "artwork backfill: artwork request returned no file for {}",
                            album_dir.display()
                        );
                    }
                    Err(error) => {
                        let message = format!("{}: {error:#}", album_dir.display());
                        eprintln!("artwork backfill: {message}");
                        summary.record_error(message);
                    }
                }
            }
            Err(error) => {
                let message = format!("{}: {error:#}", album_dir.display());
                eprintln!("artwork backfill: {message}");
                summary.record_error(message);
            }
        }
    }

    eprintln!(
        "artwork backfill: inspected {} missing-cover album(s), updated {}, skipped {}",
        summary.inspected_albums, summary.updated_albums, summary.skipped_albums
    );

    Ok(summary)
}

fn prepare_import_metadata_context(
    audio_files: &[PathBuf],
    metadata_policy: MetadataPolicy,
) -> Result<(MetadataResolver, ImportMetadataContext)> {
    let resolver = MetadataResolver::new(metadata_policy);
    let track_inputs = audio_files
        .iter()
        .cloned()
        .map(|path| MetadataTrackInput { path })
        .collect::<Vec<_>>();
    let resolution = resolver.resolve_album_from_files(&track_inputs)?;
    let album_artwork_url = resolution
        .as_ref()
        .and_then(|resolution| resolution.album.as_ref())
        .and_then(|album| album.artwork_url.clone());
    let tracks_by_path = resolution
        .map(|resolution| {
            resolution
                .tracks
                .into_iter()
                .map(|track| (track.path.clone(), track))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    Ok((
        resolver,
        ImportMetadataContext {
            tracks_by_path,
            album_artwork_url,
        },
    ))
}

fn stage_album_review(
    source_root: &Path,
    source_dir: &Path,
    audio_files: &[ImportSourceFile],
    metadata_policy: MetadataPolicy,
) -> Result<ImportAlbumReview> {
    if matches!(metadata_policy, MetadataPolicy::TagsOnly) {
        return stage_album_review_from_local_tags(source_root, source_dir, audio_files);
    }

    let (resolver, metadata_context) =
        prepare_import_metadata_context(&sample_audio_files(audio_files), metadata_policy)?;
    let mut warnings = Vec::new();
    let mut tracks_by_path = HashMap::new();
    let mut issue_by_path = HashMap::new();
    let mut detected_album = derive_album_metadata(
        metadata_context.tracks_by_path.values().next(),
        metadata_context.album_artwork_url.clone(),
    );

    if let Some(release_id) = dominant_release_id(&metadata_context.tracks_by_path) {
        let analysis_paths = audio_files
            .iter()
            .map(|file| file.analysis_path.clone())
            .collect::<Vec<_>>();
        let release_tracks =
            MusicBrainzClient::new().resolve_release_tracks(&release_id, &analysis_paths)?;
        if !release_tracks.is_empty() {
            tracks_by_path = release_tracks
                .into_iter()
                .map(|track| (track.path.clone(), track))
                .collect();
            if detected_album
                .as_ref()
                .and_then(|album| album.artwork_url.clone())
                .is_none()
            {
                let artwork_url = CoverArtClient::new().fetch_release_artwork_url(&release_id)?;
                if let Some(album) = detected_album.as_mut() {
                    album.artwork_url = artwork_url.clone();
                }
            }
        }
    }

    if tracks_by_path.is_empty() {
        let dominant = resolve_dominant_album_tracks(audio_files, &resolver)?;
        if let Some(album) = dominant.album {
            detected_album = Some(album);
        }
        tracks_by_path = dominant.tracks_by_path;
        issue_by_path = dominant.issue_by_path;
        warnings.extend(dominant.warnings);
    } else if tracks_by_path.len() < audio_files.len() {
        warnings.push(format!(
            "Matched {} of {} track(s) to the detected album.",
            tracks_by_path.len(),
            audio_files.len()
        ));
    }

    let artwork_url = detected_album
        .as_ref()
        .and_then(|album| album.artwork_url.clone());
    let source_label = source_dir
        .strip_prefix(source_root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| {
            source_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Import")
                .to_string()
        });
    let mut tracks = Vec::with_capacity(audio_files.len());
    for file in audio_files {
        let path = &file.analysis_path;
        tracks.push(ImportTrackReview {
            source_path: file.source_path.clone(),
            analysis_path: file.analysis_path.clone(),
            original_title: inferred_title_for_display(&file.source_path),
            detected_track: tracks_by_path.get(path).cloned(),
            metadata_source: tracks_by_path
                .get(path)
                .map(|_| ImportMetadataSource::OnlineServices),
            artwork_url: artwork_url.clone(),
            manual_metadata: ImportTrackMetadataDraft::default(),
            missing_fields: Vec::new(),
            skipped: false,
            manual_mode: false,
            issue: issue_by_path.remove(path).or_else(|| {
                if tracks_by_path.contains_key(path) {
                    None
                } else {
                    Some("No track match found in the detected album.".to_string())
                }
            }),
        });
    }

    Ok(ImportAlbumReview {
        source_label,
        detected_album,
        artwork_url,
        tracks,
        warnings,
    })
}

fn stage_album_review_from_local_tags(
    source_root: &Path,
    source_dir: &Path,
    audio_files: &[ImportSourceFile],
) -> Result<ImportAlbumReview> {
    let source_label = source_dir
        .strip_prefix(source_root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| {
            source_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Import")
                .to_string()
        });

    let tracks = audio_files
        .iter()
        .map(build_local_track_review)
        .collect::<Vec<_>>();

    let mut review = ImportAlbumReview {
        source_label,
        detected_album: None,
        artwork_url: None,
        tracks,
        warnings: Vec::new(),
    };
    review.refresh_summary_from_tracks();
    Ok(review)
}

fn build_local_track_review(file: &ImportSourceFile) -> ImportTrackReview {
    let original_title = inferred_title_for_display(&file.source_path);
    match read_track_metadata_draft(&file.analysis_path) {
        Ok(local_metadata) => {
            let missing_fields = local_metadata.missing_required_fields();
            let detected_track = local_metadata
                .clone()
                .into_resolved_track_metadata(file.analysis_path.clone());
            let issue = if missing_fields.is_empty() {
                None
            } else {
                Some(format!(
                    "Missing {} in local tags.",
                    format_missing_fields(&missing_fields)
                ))
            };

            ImportTrackReview {
                source_path: file.source_path.clone(),
                analysis_path: file.analysis_path.clone(),
                original_title,
                detected_track,
                metadata_source: issue.is_none().then_some(ImportMetadataSource::LocalTags),
                artwork_url: None,
                manual_metadata: local_metadata,
                missing_fields,
                skipped: false,
                manual_mode: false,
                issue,
            }
        }
        Err(error) => ImportTrackReview {
            source_path: file.source_path.clone(),
            analysis_path: file.analysis_path.clone(),
            original_title,
            detected_track: None,
            metadata_source: None,
            artwork_url: None,
            manual_metadata: ImportTrackMetadataDraft::default(),
            missing_fields: vec![
                ImportMetadataField::Title,
                ImportMetadataField::Artist,
                ImportMetadataField::Album,
            ],
            skipped: false,
            manual_mode: false,
            issue: Some(format!("{error:#}")),
        },
    }
}

fn import_audio_file(
    library: &Library,
    connection: &mut Connection,
    source_root: &Path,
    analysis_path: &Path,
    source_path: &Path,
    metadata: ResolvedTrackMetadata,
    remote_artwork_url: Option<&str>,
) -> Result<Option<String>> {
    let tagged_file = lofty::read_from_path(analysis_path)
        .with_context(|| format!("Failed to read audio tags from {}", analysis_path.display()))?;
    let duration_seconds = Some(
        tagged_file
            .properties()
            .duration()
            .as_secs()
            .min(u32::MAX as u64) as u32,
    );
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let ResolvedTrackMetadata {
        path: _,
        title,
        artist,
        album,
        album_artist,
        disc_number,
        track_number,
        recording_id: _,
        release_id: _,
    } = metadata;

    let disc_number = disc_number.unwrap_or(1).max(1);
    let collection_id = stable_id(&(album_artist.as_str(), album.as_str()));
    let track_id = stable_id(&(
        collection_id.as_str(),
        disc_number,
        track_number.unwrap_or(0),
        title.as_str(),
        analysis_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("audio"),
    ));

    let album_dir = library
        .library_root
        .join(ProviderId::Local.as_str())
        .join(sanitize_path_component(&album_artist))
        .join(sanitize_path_component(&album));
    fs::create_dir_all(&album_dir).with_context(|| {
        format!(
            "Failed to create import album directory {}",
            album_dir.display()
        )
    })?;

    let extension = analysis_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    let file_prefix = track_number
        .map(|number| format!("{number:02} "))
        .unwrap_or_default();
    let destination_audio_path = resolve_unique_destination(
        &album_dir,
        &format!("{file_prefix}{}", title),
        extension,
        &track_id,
    );
    copy_if_needed(analysis_path, &destination_audio_path)?;

    let artwork_path = ensure_imported_artwork(
        source_root,
        source_path,
        tag,
        &album_dir,
        &collection_id,
        remote_artwork_url,
    )?;
    let track_position =
        track_number.map(|number| ((disc_number - 1) * 1000 + number - 1) as usize);
    let canonical_audio_path = destination_audio_path.to_string_lossy().into_owned();
    let destination_audio_path_string = destination_audio_path.to_string_lossy().into_owned();
    let collection_id_for_db = collection_id.clone();
    let album_for_db = album.clone();
    let album_artist_for_db = album_artist.clone();
    let artwork_path_string = artwork_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    connection.execute(
        r#"
        INSERT INTO cached_tracks (
            provider,
            track_id,
            canonical_url,
            artist,
            album,
            title,
            collection_id,
            collection_title,
            collection_subtitle,
            track_position,
            duration_seconds,
            stream_url,
            audio_path,
            artwork_path,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, unixepoch())
        ON CONFLICT(provider, track_id) DO UPDATE SET
            canonical_url = excluded.canonical_url,
            artist = excluded.artist,
            album = excluded.album,
            title = excluded.title,
            collection_id = excluded.collection_id,
            collection_title = excluded.collection_title,
            collection_subtitle = excluded.collection_subtitle,
            track_position = excluded.track_position,
            duration_seconds = excluded.duration_seconds,
            stream_url = excluded.stream_url,
            audio_path = excluded.audio_path,
            artwork_path = COALESCE(excluded.artwork_path, artwork_path),
            updated_at = unixepoch()
        "#,
        params![
            ProviderId::Local.as_str(),
            track_id.clone(),
            canonical_audio_path.clone(),
            artist.clone(),
            album.clone(),
            title.clone(),
            collection_id_for_db.clone(),
            album_for_db.clone(),
            album_artist_for_db.clone(),
            track_position,
            duration_seconds,
            destination_audio_path_string.clone(),
            destination_audio_path_string,
            artwork_path_string,
        ],
    )?;
    let imported_track = TrackSummary {
        reference: TrackRef::new(
            ProviderId::Local,
            track_id,
            Some(canonical_audio_path),
            Some(title.clone()),
        ),
        title,
        artist: Some(artist),
        album: Some(album.clone()),
        collection_id: Some(collection_id_for_db.clone()),
        collection_title: Some(album_for_db),
        collection_subtitle: Some(album_artist_for_db),
        duration_seconds,
        bitrate_bps: None,
        audio_format: None,
        artwork_url: artwork_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    };
    super::entities::sync_cached_track(
        connection,
        &imported_track,
        track_position,
        artwork_path.as_deref(),
    )?;

    Ok(Some(collection_id))
}

fn collect_import_audio_files(
    library: &Library,
    selections: &[PathBuf],
) -> Result<(PathBuf, Vec<PathBuf>)> {
    if selections.is_empty() {
        bail!("No files or folders were selected for import");
    }

    let local_root = library.library_root.join(ProviderId::Local.as_str());
    let mut audio_files = BTreeSet::new();
    let mut roots = Vec::new();

    for selection in selections {
        let canonical = fs::canonicalize(selection)
            .with_context(|| format!("Failed to access import path {}", selection.display()))?;
        if canonical.starts_with(&local_root) {
            bail!("Cannot import from the managed local library itself");
        }

        let metadata = fs::metadata(&canonical)
            .with_context(|| format!("Failed to inspect import path {}", canonical.display()))?;
        if metadata.is_dir() {
            roots.push(canonical.clone());
            for audio_file in walk_audio_files(&canonical)? {
                audio_files.insert(audio_file);
            }
        } else if metadata.is_file() {
            if is_supported_audio_file(&canonical) {
                roots.push(
                    canonical
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| canonical.clone()),
                );
                audio_files.insert(canonical);
            }
        } else {
            bail!(
                "Import path '{}' is neither a file nor a folder",
                canonical.display()
            );
        }
    }

    let audio_files = audio_files.into_iter().collect::<Vec<_>>();
    if audio_files.is_empty() {
        bail!("No supported audio files were found in the selected import paths");
    }

    let source_root = common_ancestor_path(&roots)
        .or_else(|| common_ancestor_path(&audio_files))
        .context("Could not determine a shared import root")?;

    Ok((source_root, audio_files))
}

fn group_audio_files_by_directory(
    source_root: &Path,
    audio_files: Vec<ImportSourceFile>,
) -> Vec<(PathBuf, Vec<ImportSourceFile>)> {
    let mut grouped = BTreeMap::<PathBuf, Vec<ImportSourceFile>>::new();
    for file in audio_files {
        let key = file
            .source_path
            .parent()
            .unwrap_or(source_root)
            .to_path_buf();
        grouped.entry(key).or_default().push(file);
    }

    grouped.into_iter().collect()
}

fn sample_audio_files(audio_files: &[ImportSourceFile]) -> Vec<PathBuf> {
    if audio_files.len() <= 3 {
        return audio_files
            .iter()
            .map(|file| file.analysis_path.clone())
            .collect();
    }

    let mut indexes = vec![0usize, audio_files.len() / 2, audio_files.len() - 1];
    indexes.sort_unstable();
    indexes.dedup();
    indexes
        .into_iter()
        .map(|index| audio_files[index].analysis_path.clone())
        .collect()
}

fn dominant_release_id(tracks_by_path: &HashMap<PathBuf, ResolvedTrackMetadata>) -> Option<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for release_id in tracks_by_path
        .values()
        .filter_map(|track| track.release_id.clone())
    {
        *counts.entry(release_id).or_default() += 1;
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(release_id, _)| release_id)
}

fn derive_album_metadata(
    track: Option<&ResolvedTrackMetadata>,
    artwork_url: Option<String>,
) -> Option<ResolvedAlbumMetadata> {
    track.map(|track| ResolvedAlbumMetadata {
        title: track.album.clone(),
        artist: track.album_artist.clone(),
        artwork_url,
        release_id: track.release_id.clone(),
    })
}

#[derive(Default)]
struct DominantAlbumResolution {
    album: Option<ResolvedAlbumMetadata>,
    tracks_by_path: HashMap<PathBuf, ResolvedTrackMetadata>,
    issue_by_path: HashMap<PathBuf, String>,
    warnings: Vec<String>,
}

fn resolve_dominant_album_tracks(
    audio_files: &[ImportSourceFile],
    resolver: &MetadataResolver,
) -> Result<DominantAlbumResolution> {
    let mut successes = Vec::<ResolvedTrackMetadata>::new();
    let mut issue_by_path = HashMap::new();

    for file in audio_files {
        match resolver.resolve_track_from_file(&file.analysis_path) {
            Ok(Some(track)) => successes.push(track),
            Ok(None) => {
                issue_by_path.insert(
                    file.analysis_path.clone(),
                    "No metadata match found for this file.".to_string(),
                );
            }
            Err(error) => {
                issue_by_path.insert(file.analysis_path.clone(), format!("{error:#}"));
            }
        }
    }

    if successes.is_empty() {
        return Ok(DominantAlbumResolution {
            issue_by_path,
            warnings: vec!["No confident album match was found.".to_string()],
            ..Default::default()
        });
    }

    let mut counts = BTreeMap::<String, usize>::new();
    for track in &successes {
        *counts.entry(album_identity_key(track)).or_default() += 1;
    }
    let Some((winner, winner_count)) = counts.into_iter().max_by_key(|(_, count)| *count) else {
        return Ok(DominantAlbumResolution::default());
    };

    let mut album = None;
    let mut tracks_by_path = HashMap::new();
    for track in successes {
        if album_identity_key(&track) == winner {
            album.get_or_insert_with(|| ResolvedAlbumMetadata {
                title: track.album.clone(),
                artist: track.album_artist.clone(),
                artwork_url: None,
                release_id: track.release_id.clone(),
            });
            tracks_by_path.insert(track.path.clone(), track);
        } else {
            issue_by_path.insert(
                track.path.clone(),
                "Matched a different album than the dominant folder match.".to_string(),
            );
        }
    }

    let mut warnings = Vec::new();
    if winner_count < audio_files.len() {
        warnings.push(format!(
            "Accepted the dominant album match for {} of {} track(s).",
            winner_count,
            audio_files.len()
        ));
    }

    if let Some(album) = album.as_mut()
        && album.artwork_url.is_none()
        && let Some(release_id) = album.release_id.clone()
    {
        album.artwork_url = CoverArtClient::new().fetch_release_artwork_url(&release_id)?;
    }

    Ok(DominantAlbumResolution {
        album,
        tracks_by_path,
        issue_by_path,
        warnings,
    })
}

fn album_identity_key(track: &ResolvedTrackMetadata) -> String {
    if let Some(release_id) = track.release_id.as_deref() {
        return format!("release:{release_id}");
    }

    format!("{}::{}", track.album_artist, track.album)
}

fn normalize_import_files(
    library: &Library,
    source_root: &Path,
    audio_files: Vec<PathBuf>,
) -> Result<(Vec<ImportSourceFile>, Option<PathBuf>)> {
    let mut import_files = Vec::with_capacity(audio_files.len());
    let mut staging_root = None;

    for source_path in audio_files {
        let normalized_path = if needs_normalized_copy(&source_path)? {
            let staging_root_path =
                staging_root.get_or_insert_with(|| create_import_staging_root(library));
            normalize_source_audio_to_staging(source_root, &source_path, staging_root_path)?
        } else {
            source_path.clone()
        };

        import_files.push(ImportSourceFile {
            source_path,
            analysis_path: normalized_path,
        });
    }

    Ok((import_files, staging_root))
}

fn walk_audio_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("Failed to read {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                stack.push(path);
                continue;
            }

            if file_type.is_file() && is_supported_audio_file(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn read_track_metadata_draft(path: &Path) -> Result<ImportTrackMetadataDraft> {
    let tagged_file = lofty::read_from_path(path)
        .with_context(|| format!("Failed to inspect audio tags from {}", path.display()))?;
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .context("Audio file did not contain readable metadata tags")?;

    Ok(ImportTrackMetadataDraft {
        title: tag
            .title()
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        artist: tag
            .artist()
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        album: tag
            .album()
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        album_artist: tag
            .get_string(&ItemKey::AlbumArtist)
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        disc_number: tag
            .disk()
            .map(|value| value.to_string())
            .unwrap_or_default(),
        track_number: tag
            .track()
            .map(|value| value.to_string())
            .unwrap_or_default(),
    })
}

fn parse_optional_u32(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse::<u32>().ok().filter(|value| *value > 0)
    }
}

fn format_missing_fields(fields: &[ImportMetadataField]) -> String {
    let labels = fields.iter().map(|field| field.label()).collect::<Vec<_>>();
    match labels.as_slice() {
        [] => "metadata".to_string(),
        [label] => label.to_string(),
        [first, second] => format!("{first} and {second}"),
        _ => {
            let last = labels.last().copied().unwrap_or("metadata");
            format!("{}, and {}", labels[..labels.len() - 1].join(", "), last)
        }
    }
}

fn is_supported_audio_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("mp3")
            | Some("flac")
            | Some("m4a")
            | Some("mp4")
            | Some("webm")
            | Some("aac")
            | Some("wav")
            | Some("aiff")
            | Some("aif")
            | Some("ogg")
            | Some("oga")
            | Some("opus")
            | Some("wv")
            | Some("ape")
            | Some("mpc")
    )
}

fn ensure_imported_artwork(
    source_root: &Path,
    source_path: &Path,
    tag: Option<&lofty::tag::Tag>,
    album_dir: &Path,
    collection_id: &str,
    remote_artwork_url: Option<&str>,
) -> Result<Option<PathBuf>> {
    let existing = find_existing_cover(album_dir)?;
    if existing.is_some() {
        return Ok(existing);
    }

    if let Some(path) = import_embedded_picture(tag, album_dir)? {
        return Ok(Some(path));
    }

    if let Some(path) =
        import_source_directory_picture(source_root, source_path, album_dir, collection_id)?
    {
        return Ok(Some(path));
    }

    if let Some(path) = import_remote_picture(remote_artwork_url, album_dir)? {
        return Ok(Some(path));
    }

    Ok(None)
}

fn import_embedded_picture(
    tag: Option<&lofty::tag::Tag>,
    album_dir: &Path,
) -> Result<Option<PathBuf>> {
    let Some(tag) = tag else {
        return Ok(None);
    };

    let picture = tag
        .get_picture_type(PictureType::CoverFront)
        .or_else(|| tag.pictures().first());
    let Some(picture) = picture else {
        return Ok(None);
    };

    let extension = picture.mime_type().and_then(MimeType::ext).unwrap_or("jpg");
    let destination = album_dir.join(format!("cover.{extension}"));
    fs::write(&destination, picture.data())
        .with_context(|| format!("Failed to write artwork {}", destination.display()))?;
    Ok(Some(destination))
}

fn import_source_directory_picture(
    source_root: &Path,
    source_path: &Path,
    album_dir: &Path,
    collection_id: &str,
) -> Result<Option<PathBuf>> {
    let mut candidates = Vec::new();
    if let Some(parent) = source_path.parent() {
        candidates.push(parent.to_path_buf());
    }
    if let Some(relative_parent) = source_path
        .strip_prefix(source_root)
        .ok()
        .and_then(Path::parent)
    {
        candidates.push(source_root.join(relative_parent));
    }

    for directory in candidates {
        for name in ["cover", "folder", "front", "artwork"] {
            for extension in ["jpg", "jpeg", "png", "gif"] {
                let candidate = directory.join(format!("{name}.{extension}"));
                if !candidate.is_file() {
                    continue;
                }

                let destination = album_dir.join(format!(
                    "cover-{}.{}",
                    collection_id,
                    candidate
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or(extension)
                ));
                copy_if_needed(&candidate, &destination)?;
                return Ok(Some(destination));
            }
        }
    }

    Ok(None)
}

fn find_existing_cover(album_dir: &Path) -> Result<Option<PathBuf>> {
    for entry in fs::read_dir(album_dir)
        .with_context(|| format!("Failed to inspect artwork in {}", album_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if entry.file_type()?.is_file() && name.starts_with("cover.") {
            return Ok(Some(path));
        }
        if entry.file_type()?.is_file() && name.starts_with("cover-") {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn resolve_unique_destination(
    album_dir: &Path,
    title: &str,
    extension: &str,
    track_id: &str,
) -> PathBuf {
    let preferred = album_dir.join(format!("{}.{}", sanitize_path_component(title), extension));
    if !preferred.exists() {
        return preferred;
    }

    album_dir.join(format!(
        "{} [{}].{}",
        sanitize_path_component(title),
        &track_id[..8.min(track_id.len())],
        extension
    ))
}

fn copy_if_needed(source: &Path, destination: &Path) -> Result<()> {
    if destination.is_file() {
        return Ok(());
    }

    fs::copy(source, destination).with_context(|| {
        format!(
            "Failed to copy imported audio from {} to {}",
            source.display(),
            destination.display()
        )
    })?;

    Ok(())
}

fn remux_webm_to_opus(source: &Path, destination: &Path) -> Result<()> {
    let output = Command::new("ffmpeg")
        .arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(source)
        .arg("-vn")
        .arg("-c:a")
        .arg("copy")
        .arg(destination)
        .output()
        .with_context(|| format!("Failed to launch ffmpeg for {}", source.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to remux {} to {}: {}",
            source.display(),
            destination.display(),
            stderr.trim()
        );
    }

    Ok(())
}

fn import_remote_picture(
    remote_artwork_url: Option<&str>,
    album_dir: &Path,
) -> Result<Option<PathBuf>> {
    let Some(remote_artwork_url) = remote_artwork_url else {
        return Ok(None);
    };

    let response = match import_remote_picture_response(remote_artwork_url) {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };

    save_remote_picture(response, album_dir)
}

fn import_remote_picture_strict(
    remote_artwork_url: &str,
    album_dir: &Path,
) -> Result<Option<PathBuf>> {
    let response = import_remote_picture_response(remote_artwork_url)?;
    save_remote_picture(response, album_dir)
}

fn import_remote_picture_response(remote_artwork_url: &str) -> Result<ureq::Response> {
    ureq::AgentBuilder::new()
        .timeout(ARTWORK_DOWNLOAD_TIMEOUT)
        .build()
        .get(remote_artwork_url)
        .set("User-Agent", "Oryx/0.1")
        .call()
        .with_context(|| format!("Failed to download artwork from {remote_artwork_url}"))
}

fn save_remote_picture(response: ureq::Response, album_dir: &Path) -> Result<Option<PathBuf>> {
    let extension = response
        .header("Content-Type")
        .and_then(content_type_extension)
        .unwrap_or("jpg");
    let destination = album_dir.join(format!("cover.{extension}"));
    if destination.is_file() {
        return Ok(Some(destination));
    }

    let mut reader = response.into_reader();
    let mut file = fs::File::create(&destination)
        .with_context(|| format!("Failed to create artwork file {}", destination.display()))?;
    std::io::copy(&mut reader, &mut file)
        .with_context(|| format!("Failed to save artwork to {}", destination.display()))?;
    Ok(Some(destination))
}

fn content_type_extension(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next()?.trim() {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn sanitize_path_component(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());

    for ch in input.chars() {
        let replacement = match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        };
        sanitized.push(replacement);
    }

    let sanitized = sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['.', ' '])
        .to_string();

    if sanitized.is_empty() {
        "Untitled".to_string()
    } else {
        sanitized
    }
}

fn probe_media_file(path: &Path) -> Result<MediaProbe> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("stream=codec_type,codec_name")
        .arg("-of")
        .arg("json")
        .arg(path)
        .output()
        .with_context(|| format!("Failed to launch ffprobe for {}", path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ffprobe failed for {}: {}", path.display(), stderr.trim());
    }

    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("Failed to parse ffprobe output for {}", path.display()))
}

#[derive(Debug, Deserialize)]
struct MediaProbe {
    #[serde(default)]
    streams: Vec<MediaProbeStream>,
}

#[derive(Debug, Deserialize)]
struct MediaProbeStream {
    codec_name: Option<String>,
    codec_type: Option<String>,
}

fn inferred_title_for_display(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .unwrap_or("Unknown Title");
    strip_numeric_suffix(stem).trim().to_string()
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

fn stable_id(value: &impl Hash) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn create_import_staging_root(_library: &Library) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    std::env::temp_dir()
        .join("oryx-import")
        .join(format!("stage-{}-{}", stamp, std::process::id()))
}

fn needs_normalized_copy(path: &Path) -> Result<bool> {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("webm"))
    {
        return Ok(false);
    }

    Ok(probe_media_file(path)?.streams.into_iter().any(|stream| {
        stream.codec_type.as_deref() == Some("audio")
            && stream.codec_name.as_deref() == Some("opus")
    }))
}

fn normalize_source_audio_to_staging(
    source_root: &Path,
    source_path: &Path,
    staging_root: &Path,
) -> Result<PathBuf> {
    let relative_parent = source_path
        .strip_prefix(source_root)
        .ok()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""));
    let staging_dir = staging_root.join(relative_parent);
    fs::create_dir_all(&staging_dir)
        .with_context(|| format!("Failed to create import staging {}", staging_dir.display()))?;

    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    let destination = staging_dir.join(format!("{stem}.opus"));
    if destination.is_file() {
        return Ok(destination);
    }

    remux_webm_to_opus(source_path, &destination)?;
    Ok(destination)
}

fn common_ancestor_path(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut components = paths.first()?.components().collect::<Vec<_>>();

    for path in &paths[1..] {
        let other = path.components().collect::<Vec<_>>();
        let shared_len = components
            .iter()
            .zip(other.iter())
            .take_while(|(left, right)| left == right)
            .count();
        components.truncate(shared_len);
        if components.is_empty() {
            break;
        }
    }

    if components.is_empty() {
        return None;
    }

    let mut root = PathBuf::new();
    for component in components {
        root.push(component.as_os_str());
    }
    Some(root)
}

fn update_collection_artwork_path(
    connection: &mut Connection,
    collection_id: &str,
    artwork_path: &Path,
) -> Result<()> {
    connection.execute(
        r#"
        UPDATE cached_tracks
        SET artwork_path = ?1, updated_at = unixepoch()
        WHERE provider = ?2 AND collection_id = ?3
        "#,
        params![
            artwork_path.to_string_lossy().into_owned(),
            ProviderId::Local.as_str(),
            collection_id,
        ],
    )?;
    Ok(())
}
