use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

use super::super::{
    CollectionKind, CollectionRef, CollectionSummary, SearchResult, TrackRef, TrackSummary,
};
use super::config::{
    HtmlSearchResponseSpec, JsonSearchResponseSpec, SearchItemKind, SearchResponseSpec,
    SearchResultKindSpec,
};
use super::provider::ConfiguredProvider;
use super::util::{
    extract_html_fields_from_item, extract_json_fields, parse_duration_value, resolve_json_items,
    resolve_search_item_kind, selector, url_path_id,
};

impl ConfiguredProvider {
    pub(super) fn fetch_search_response(&self, query: &str) -> Result<String> {
        let cookie_header = self.ensure_authenticated_session()?;
        let context = self.build_context(Some(query), None, None, None);
        self.fetch_text(
            &self.manifest.search.request,
            &context,
            cookie_header.as_deref(),
        )
    }

    pub(super) fn parse_search_results(&self, body: &str) -> Result<Vec<SearchResult>> {
        match &self.manifest.search.response {
            SearchResponseSpec::Html(spec) => self.parse_search_results_html(body, spec),
            SearchResponseSpec::Json(spec) => self.parse_search_results_json(body, spec),
        }
    }

    fn parse_search_results_html(
        &self,
        body: &str,
        spec: &HtmlSearchResponseSpec,
    ) -> Result<Vec<SearchResult>> {
        let document = scraper::Html::parse_document(body);
        let item_selector = selector(&spec.item_selector)?;
        let mut results = Vec::new();

        for item in document.select(&item_selector) {
            let fields = extract_html_fields_from_item(item, &spec.fields)?;
            if let Some(result) = self.build_search_result(&fields, &spec.result_kind) {
                results.push(result);
            }
        }

        Ok(results)
    }

    fn parse_search_results_json(
        &self,
        body: &str,
        spec: &JsonSearchResponseSpec,
    ) -> Result<Vec<SearchResult>> {
        let json: Value = serde_json::from_str(body).context("invalid JSON search response")?;
        let mut results = Vec::new();

        for item in resolve_json_items(&json, &spec.items_path)? {
            let fields = extract_json_fields(item, &spec.fields)?;
            if let Some(result) = self.build_search_result(&fields, &spec.result_kind) {
                results.push(result);
            }
        }

        Ok(results)
    }

    fn build_search_result(
        &self,
        fields: &HashMap<String, String>,
        kind_spec: &SearchResultKindSpec,
    ) -> Option<SearchResult> {
        let resolved = resolve_search_item_kind(fields, kind_spec)
            .unwrap_or(SearchItemKind::Collection(CollectionKind::Album));
        let provider_id = self.provider_id();
        let url = fields.get("url").cloned();
        let id = fields
            .get("id")
            .cloned()
            .or_else(|| url.as_deref().map(url_path_id))
            .or_else(|| fields.get("title").cloned())?;
        let title = fields
            .get("title")
            .cloned()
            .or_else(|| url.clone())
            .unwrap_or_else(|| id.clone());
        let subtitle = fields
            .get("subtitle")
            .cloned()
            .or_else(|| fields.get("artist").cloned())
            .filter(|value| !value.is_empty());
        let artwork_url = fields
            .get("artwork_url")
            .cloned()
            .filter(|value| !value.is_empty());

        Some(match resolved {
            SearchItemKind::Collection(kind) => SearchResult::Collection(CollectionSummary {
                reference: CollectionRef::new(provider_id, id, kind, url),
                title,
                subtitle,
                artwork_url,
                track_count: fields
                    .get("track_count")
                    .and_then(|value| value.parse::<usize>().ok()),
            }),
            SearchItemKind::Track => {
                let reference = TrackRef::new(provider_id, id, url, Some(title.clone()));
                SearchResult::Track(TrackSummary {
                    reference,
                    title,
                    artist: fields.get("artist").cloned().or(subtitle),
                    album: fields.get("album").cloned(),
                    collection_id: fields.get("collection_id").cloned(),
                    collection_title: fields.get("collection_title").cloned(),
                    collection_subtitle: fields.get("collection_subtitle").cloned(),
                    duration_seconds: fields
                        .get("duration_seconds")
                        .and_then(|value| parse_duration_value(value)),
                    bitrate_bps: None,
                    audio_format: None,
                    artwork_url,
                })
            }
        })
    }
}
