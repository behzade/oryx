use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::super::{CollectionRef, CollectionSummary, TrackList, TrackRef, TrackSummary};
use super::config::{
    HtmlScriptTrackListResponseSpec, HtmlTrackListResponseSpec, JsonTrackListResponseSpec,
    TrackListResponseSpec,
};
use super::provider::ConfiguredProvider;
use super::util::{
    extract_html_fields_from_document, extract_html_fields_from_item,
    extract_indexed_html_fields_from_document, extract_js_fields, extract_json_fields,
    extract_script_block, parse_duration_value, resolve_json_items, selector, split_js_objects,
};

impl ConfiguredProvider {
    pub(super) fn fetch_track_list_response(&self, collection: &CollectionRef) -> Result<String> {
        let cookie_header = self.ensure_authenticated_session()?;
        let context = self.build_context(None, Some(collection), None, None);
        let request = self.manifest.track_list.request.with_default_url(
            collection.canonical_url.clone().or_else(|| {
                self.provider_id()
                    .collection_url(collection.kind, &collection.id)
            }),
        );
        self.fetch_text(&request, &context, cookie_header.as_deref())
    }

    pub(super) fn parse_track_list(
        &self,
        collection: &CollectionRef,
        body: &str,
    ) -> Result<TrackList> {
        match &self.manifest.track_list.response {
            TrackListResponseSpec::Html(spec) => self.parse_track_list_html(collection, body, spec),
            TrackListResponseSpec::HtmlScript(spec) => {
                self.parse_track_list_html_script(collection, body, spec)
            }
            TrackListResponseSpec::Json(spec) => self.parse_track_list_json(collection, body, spec),
        }
    }

    fn parse_track_list_html(
        &self,
        collection: &CollectionRef,
        body: &str,
        spec: &HtmlTrackListResponseSpec,
    ) -> Result<TrackList> {
        let document = scraper::Html::parse_document(body);
        let collection_fields =
            extract_html_fields_from_document(&document, &spec.collection_fields)?;
        let item_selector = selector(&spec.track_item_selector)?;

        let collection_title = collection_fields
            .get("title")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| collection.id.clone());
        let collection_subtitle = collection_fields
            .get("subtitle")
            .cloned()
            .filter(|value| !value.is_empty());
        let collection_artwork_url = collection_fields
            .get("artwork_url")
            .cloned()
            .filter(|value| !value.is_empty());

        let mut tracks = Vec::new();
        for item in document.select(&item_selector) {
            let fields = extract_html_fields_from_item(item, &spec.track_fields)?;
            if let Some(track) = self.build_track_summary(
                collection,
                &collection_title,
                collection_subtitle.as_deref(),
                collection_artwork_url.as_deref(),
                &fields,
            ) {
                tracks.push(track);
            }
        }

        if tracks.is_empty() {
            bail!("configured provider track parser did not resolve any playable tracks");
        }

        Ok(TrackList {
            collection: CollectionSummary {
                reference: collection.clone(),
                title: collection_title,
                subtitle: collection_subtitle,
                artwork_url: collection_artwork_url,
                track_count: Some(tracks.len()),
            },
            tracks,
        })
    }

    fn parse_track_list_html_script(
        &self,
        collection: &CollectionRef,
        body: &str,
        spec: &HtmlScriptTrackListResponseSpec,
    ) -> Result<TrackList> {
        let document = scraper::Html::parse_document(body);
        let collection_fields =
            extract_html_fields_from_document(&document, &spec.collection_fields)?;
        let indexed_fields =
            extract_indexed_html_fields_from_document(&document, &spec.indexed_html_fields)?;
        let playlist_script = extract_script_block(body, &spec.script_start, &spec.script_end)?;
        let track_objects = split_js_objects(&playlist_script);

        let collection_title = collection_fields
            .get("title")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| collection.id.clone());
        let collection_subtitle = collection_fields
            .get("subtitle")
            .cloned()
            .filter(|value| !value.is_empty());
        let collection_artwork_url = collection_fields
            .get("artwork_url")
            .cloned()
            .filter(|value| !value.is_empty());

        let mut tracks = Vec::new();
        let mut visible_index = 0usize;

        for object in track_objects {
            let mut fields = extract_js_fields(&object, &spec.track_fields);
            for (field_name, values) in &indexed_fields {
                if let Some(value) = values.get(visible_index).cloned() {
                    if !value.is_empty() {
                        fields.insert(field_name.clone(), value);
                    }
                }
            }

            if spec.skip_if_field_contains.iter().any(|rule| {
                fields
                    .get(&rule.field)
                    .map(|value| value.contains(&rule.contains))
                    .unwrap_or(false)
            }) {
                continue;
            }

            if let Some(track) = self.build_track_summary(
                collection,
                &collection_title,
                collection_subtitle.as_deref(),
                collection_artwork_url.as_deref(),
                &fields,
            ) {
                tracks.push(track);
                visible_index += 1;
            }
        }

        if let Some(field_name) = spec.strict_indexed_field.as_deref() {
            let expected = indexed_fields
                .get(field_name)
                .map(|items| items.len())
                .unwrap_or(0);
            if expected > 0 && tracks.len() < expected {
                let message = spec
                    .count_mismatch_message
                    .clone()
                    .unwrap_or_else(|| {
                        "configured provider only resolved {resolved} of {expected} visible tracks"
                            .to_string()
                    })
                    .replace("{resolved}", &tracks.len().to_string())
                    .replace("{expected}", &expected.to_string());
                bail!("{message}");
            }
        }

        if tracks.is_empty() {
            bail!(
                "{}",
                spec.no_tracks_message.as_deref().unwrap_or(
                    "configured provider track parser did not resolve any playable tracks"
                )
            );
        }

        Ok(TrackList {
            collection: CollectionSummary {
                reference: collection.clone(),
                title: collection_title,
                subtitle: collection_subtitle,
                artwork_url: collection_artwork_url,
                track_count: Some(tracks.len()),
            },
            tracks,
        })
    }

    fn parse_track_list_json(
        &self,
        collection: &CollectionRef,
        body: &str,
        spec: &JsonTrackListResponseSpec,
    ) -> Result<TrackList> {
        let json: Value = serde_json::from_str(body).context("invalid JSON track list response")?;
        let collection_fields = extract_json_fields(&json, &spec.collection_fields)?;
        let collection_title = collection_fields
            .get("title")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| collection.id.clone());
        let collection_subtitle = collection_fields
            .get("subtitle")
            .cloned()
            .filter(|value| !value.is_empty());
        let collection_artwork_url = collection_fields
            .get("artwork_url")
            .cloned()
            .filter(|value| !value.is_empty());

        let mut tracks = Vec::new();
        for item in resolve_json_items(&json, &spec.tracks_path)? {
            let fields = extract_json_fields(item, &spec.track_fields)?;
            if let Some(track) = self.build_track_summary(
                collection,
                &collection_title,
                collection_subtitle.as_deref(),
                collection_artwork_url.as_deref(),
                &fields,
            ) {
                tracks.push(track);
            }
        }

        if tracks.is_empty() {
            bail!("configured provider track parser did not resolve any playable tracks");
        }

        Ok(TrackList {
            collection: CollectionSummary {
                reference: collection.clone(),
                title: collection_title,
                subtitle: collection_subtitle,
                artwork_url: collection_artwork_url,
                track_count: Some(tracks.len()),
            },
            tracks,
        })
    }

    pub(super) fn build_track_summary(
        &self,
        collection: &CollectionRef,
        collection_title: &str,
        collection_subtitle: Option<&str>,
        collection_artwork_url: Option<&str>,
        fields: &HashMap<String, String>,
    ) -> Option<TrackSummary> {
        let source_url = fields
            .get("source_url")
            .cloned()
            .or_else(|| fields.get("url").cloned())?;
        let title = fields
            .get("title")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| source_url.clone());
        let artist = fields
            .get("artist")
            .cloned()
            .filter(|value| !value.is_empty())
            .or_else(|| collection_subtitle.map(str::to_string));
        let album = fields
            .get("album")
            .cloned()
            .filter(|value| !value.is_empty())
            .or_else(|| Some(collection_title.to_string()));
        let artwork_url = fields
            .get("artwork_url")
            .cloned()
            .filter(|value| !value.is_empty())
            .or_else(|| collection_artwork_url.map(str::to_string));

        Some(TrackSummary {
            reference: TrackRef::direct_url(self.provider_id(), source_url, Some(title.clone())),
            title,
            artist,
            album,
            collection_id: Some(collection.id.clone()),
            collection_title: Some(collection_title.to_string()),
            collection_subtitle: collection_subtitle.map(str::to_string),
            duration_seconds: fields
                .get("duration_seconds")
                .and_then(|value| parse_duration_value(value)),
            bitrate_bps: None,
            audio_format: None,
            artwork_url,
        })
    }
}
