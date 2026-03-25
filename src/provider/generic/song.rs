use anyhow::{Result, anyhow, bail};

use super::super::{CollectionKind, CollectionRef, SongData, TrackRef, TrackSummary};
use super::provider::ConfiguredProvider;
use super::util::url_path_id;

impl ConfiguredProvider {
    fn is_media_url(&self, url: &str) -> bool {
        self.manifest
            .song
            .media_url_prefixes
            .iter()
            .any(|prefix| url.starts_with(prefix))
            || self.manifest.song.media_url_suffixes.iter().any(|suffix| {
                url.to_ascii_lowercase()
                    .ends_with(&suffix.to_ascii_lowercase())
            })
            || self
                .manifest
                .song
                .media_url_contains
                .iter()
                .any(|pattern| url.contains(pattern))
    }

    fn is_page_url(&self, url: &str) -> bool {
        self.manifest
            .song
            .page_url_prefixes
            .iter()
            .any(|prefix| url.starts_with(prefix))
            || self
                .manifest
                .song
                .page_url_contains
                .iter()
                .any(|pattern| url.contains(pattern))
    }

    fn is_blocked_preview_url(&self, url: &str) -> bool {
        self.manifest
            .song
            .blocked_url_patterns
            .iter()
            .any(|pattern| url.contains(pattern))
    }

    fn fetch_song_page(&self, track: &TrackRef) -> Result<String> {
        let cookie_header = self.ensure_authenticated_session()?;
        let context = self.build_context(None, None, Some(track), None);
        let request = self
            .manifest
            .song
            .page_request
            .with_default_url(track.canonical_url.clone());
        self.fetch_text(&request, &context, cookie_header.as_deref())
    }

    fn song_data_from_page(&self, track: &TrackRef, body: &str) -> Result<SongData> {
        let page_url = track
            .canonical_url
            .as_deref()
            .ok_or_else(|| anyhow!("page-backed track is missing a canonical url"))?;

        let synthetic_collection = CollectionRef::new(
            self.provider_id(),
            url_path_id(page_url),
            CollectionKind::Album,
            Some(page_url.to_string()),
        );
        let track_list = self.parse_track_list(&synthetic_collection, body)?;

        let selected = if let Some(title_hint) = track.title_hint.as_deref() {
            track_list
                .tracks
                .iter()
                .find(|candidate| candidate.title.eq_ignore_ascii_case(title_hint))
                .cloned()
                .or_else(|| {
                    track_list
                        .tracks
                        .iter()
                        .find(|candidate| candidate.title == title_hint)
                        .cloned()
                })
                .unwrap_or_else(|| track_list.tracks[0].clone())
        } else {
            track_list.tracks[0].clone()
        };

        let url = selected
            .reference
            .canonical_url
            .clone()
            .ok_or_else(|| anyhow!("parsed page track did not expose a playable url"))?;
        if self.is_blocked_preview_url(&url) {
            bail!(
                "{}",
                self.manifest
                    .song
                    .blocked_url_message
                    .as_deref()
                    .unwrap_or("provider blocked preview playback for this track")
            );
        }

        Ok(SongData {
            track: selected,
            stream: self.direct_stream_request(
                &url,
                self.auth_state
                    .lock()
                    .ok()
                    .and_then(|state| state.session_cookie.clone())
                    .as_deref(),
            ),
        })
    }

    pub(super) fn resolve_song_data(&self, track: &TrackRef) -> Result<SongData> {
        let url = track
            .canonical_url
            .as_deref()
            .ok_or_else(|| anyhow!("track lookup needs a canonical url or direct media url"))?;

        if self.is_media_url(url) {
            if self.is_blocked_preview_url(url) {
                bail!(
                    "{}",
                    self.manifest
                        .song
                        .blocked_url_message
                        .as_deref()
                        .unwrap_or("provider blocked preview playback for this track")
                );
            }

            let cookie_header = self
                .auth_state
                .lock()
                .ok()
                .and_then(|state| state.session_cookie.clone())
                .or_else(|| self.ensure_authenticated_session().ok().flatten());
            return Ok(SongData {
                track: TrackSummary::unresolved(track.clone()),
                stream: self.direct_stream_request(url, cookie_header.as_deref()),
            });
        }

        if self.is_page_url(url) {
            let body = self.fetch_song_page(track)?;
            return self.song_data_from_page(track, &body);
        }

        bail!("configured provider track lookup could not classify the track url")
    }
}
