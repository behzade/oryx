#![allow(dead_code, unused_imports)]

mod acoustid;
mod cover_art;
mod fingerprint;
mod model;
mod musicbrainz;
mod writeback;

use std::path::Path;

use anyhow::{Context, Result};
use lofty::prelude::{Accessor, ItemKey, TaggedFileExt};

pub use self::acoustid::AcoustIdClient;
pub use self::cover_art::CoverArtClient;
pub use self::fingerprint::{AudioFingerprint, FingerprintResolver};
pub use self::model::{
    MetadataConfidence, MetadataResolution, MetadataSource, MetadataTrackInput,
    ResolvedAlbumMetadata, ResolvedTrackMetadata,
};
pub use self::musicbrainz::MusicBrainzClient;
pub use self::writeback::MetadataWriter;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum MetadataPolicy {
    Off,
    #[default]
    TagsOnly,
    AutoResolveHighConfidence,
}

#[derive(Clone, Debug)]
pub struct MetadataResolver {
    policy: MetadataPolicy,
}

impl MetadataResolver {
    pub fn new(policy: MetadataPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> MetadataPolicy {
        self.policy
    }

    pub fn resolve_album_from_files(
        &self,
        tracks: &[MetadataTrackInput],
    ) -> Result<Option<MetadataResolution>> {
        match self.policy {
            MetadataPolicy::Off => Ok(None),
            MetadataPolicy::TagsOnly => self.resolve_local_album_from_files(tracks),
            MetadataPolicy::AutoResolveHighConfidence => self
                .resolve_local_album_from_files(tracks)
                .or_else(|_| self.resolve_acoustid_album_from_files(tracks))
                .or_else(|_| self.resolve_musicbrainz_album_from_files(tracks)),
        }
    }

    pub fn resolve_track_from_file(&self, path: &Path) -> Result<Option<ResolvedTrackMetadata>> {
        match self.policy {
            MetadataPolicy::Off => Ok(None),
            MetadataPolicy::TagsOnly => Ok(Some(resolve_local_track_from_path(path)?)),
            MetadataPolicy::AutoResolveHighConfidence => Ok(Some(
                resolve_local_track_from_path(path)
                    .or_else(|_| self.resolve_acoustid_track_from_file(path))
                    .or_else(|_| self.resolve_musicbrainz_track_from_file(path))?,
            )),
        }
    }

    fn resolve_local_album_from_files(
        &self,
        tracks: &[MetadataTrackInput],
    ) -> Result<Option<MetadataResolution>> {
        if tracks.is_empty() {
            return Ok(None);
        }

        let mut resolved_tracks = Vec::with_capacity(tracks.len());
        for track in tracks {
            resolved_tracks.push(resolve_local_track_from_path(&track.path)?);
        }

        let first = resolved_tracks
            .first()
            .expect("resolved_tracks should be non-empty when tracks are non-empty");
        let same_album = resolved_tracks
            .iter()
            .all(|track| track.album == first.album && track.album_artist == first.album_artist);

        let album = same_album.then(|| ResolvedAlbumMetadata {
            title: first.album.clone(),
            artist: first.album_artist.clone(),
            artwork_url: None,
            release_id: None,
        });

        Ok(Some(MetadataResolution {
            source: MetadataSource::LocalTags,
            confidence: MetadataConfidence::High,
            album,
            tracks: resolved_tracks,
        }))
    }

    fn resolve_acoustid_album_from_files(
        &self,
        tracks: &[MetadataTrackInput],
    ) -> Result<Option<MetadataResolution>> {
        let Some(client) = AcoustIdClient::from_env_optional()? else {
            return Ok(None);
        };
        client.resolve_album_from_files(tracks)
    }

    fn resolve_acoustid_track_from_file(&self, path: &Path) -> Result<ResolvedTrackMetadata> {
        let Some(client) = AcoustIdClient::from_env_optional()? else {
            anyhow::bail!("AcoustID client key is not configured")
        };
        client
            .resolve_track_from_file(path)?
            .context("AcoustID did not return a high-confidence match")
    }

    fn resolve_musicbrainz_album_from_files(
        &self,
        tracks: &[MetadataTrackInput],
    ) -> Result<Option<MetadataResolution>> {
        MusicBrainzClient::new().resolve_album_from_files(tracks)
    }

    fn resolve_musicbrainz_track_from_file(&self, path: &Path) -> Result<ResolvedTrackMetadata> {
        MusicBrainzClient::new()
            .resolve_track_from_file(path)?
            .context("MusicBrainz did not return a high-confidence match")
    }
}

fn resolve_local_track_from_path(path: &Path) -> Result<ResolvedTrackMetadata> {
    let tagged_file = lofty::read_from_path(path)
        .with_context(|| format!("Failed to read audio tags from {}", path.display()))?;
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
        .context("Audio file did not contain readable metadata tags")?;

    let title = tag
        .title()
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .context("Audio file is missing a usable track title")?;
    let artist = tag
        .artist()
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .context("Audio file is missing a usable track artist")?;
    let album = tag
        .album()
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .context("Audio file is missing a usable album title")?;
    let album_artist = tag
        .get_string(&ItemKey::AlbumArtist)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| Some(artist.clone()))
        .context("Audio file is missing a usable album artist")?;

    Ok(ResolvedTrackMetadata {
        path: path.to_path_buf(),
        title,
        artist,
        album,
        album_artist,
        disc_number: tag.disk(),
        track_number: tag.track(),
        recording_id: None,
        release_id: None,
    })
}
