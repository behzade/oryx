use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::super::{ProviderCollectionUrlTemplates, ProviderId};

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ProviderManifest {
    pub id: String,
    pub display_name: String,
    pub short_display_name: Option<String>,
    #[serde(default)]
    pub search_rank_bias: i32,
    #[serde(default)]
    pub collection_urls: ProviderCollectionUrlTemplates,
    #[serde(default)]
    pub default_headers: BTreeMap<String, String>,
    pub search: SearchOperation,
    pub track_list: TrackListOperation,
    #[serde(default)]
    pub song: SongOperation,
    pub auth: Option<AuthSpec>,
    pub validation: Option<ValidationSpec>,
}

impl ProviderManifest {
    pub fn validate(&self, path: &Path) -> Result<()> {
        ProviderId::parse(&self.id)
            .ok_or_else(|| anyhow!("provider id {:?} is invalid in {}", self.id, path.display()))?;
        if self.display_name.trim().is_empty() {
            bail!(
                "provider manifest {} is missing display_name",
                path.display()
            );
        }
        if let Some(validation) = self.validation.as_ref() {
            if validation.example_query.trim().is_empty() {
                bail!(
                    "provider manifest {} has an empty validation.example_query",
                    path.display()
                );
            }
            if validation.expect_min_results == 0 {
                bail!(
                    "provider manifest {} must set validation.expect_min_results to at least 1",
                    path.display()
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ValidationSpec {
    pub example_query: String,
    #[serde(default = "default_expect_min_results")]
    pub expect_min_results: usize,
    #[serde(default = "default_true")]
    pub test_first_collection: bool,
    #[serde(default = "default_true")]
    pub test_first_track: bool,
    #[serde(default = "default_true")]
    pub require_stream_url: bool,
    #[serde(default = "default_true")]
    pub skip_if_not_authenticated: bool,
}

fn default_expect_min_results() -> usize {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SearchOperation {
    pub request: RequestSpec,
    pub response: SearchResponseSpec,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct TrackListOperation {
    #[serde(default)]
    pub request: RequestSpec,
    pub response: TrackListResponseSpec,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub(super) struct SongOperation {
    #[serde(default)]
    pub media_url_prefixes: Vec<String>,
    #[serde(default)]
    pub media_url_suffixes: Vec<String>,
    #[serde(default)]
    pub media_url_contains: Vec<String>,
    #[serde(default)]
    pub page_url_prefixes: Vec<String>,
    #[serde(default)]
    pub page_url_contains: Vec<String>,
    #[serde(default)]
    pub blocked_url_patterns: Vec<String>,
    pub blocked_url_message: Option<String>,
    #[serde(default)]
    pub page_request: RequestSpec,
    #[serde(default = "default_supports_byte_ranges")]
    pub supports_byte_ranges: bool,
}

fn default_supports_byte_ranges() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct AuthSpec {
    #[serde(default = "default_auth_required")]
    pub required: bool,
    pub preflight: Option<RequestSpec>,
    pub submit: AuthSubmitSpec,
    pub verify: Option<AuthVerifySpec>,
}

fn default_auth_required() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct AuthSubmitSpec {
    pub request: RequestSpec,
    pub username_field: String,
    pub password_field: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct AuthVerifySpec {
    pub request: RequestSpec,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub not_contains: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct RequestSpec {
    pub method: Option<HttpMethod>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    #[serde(default)]
    pub form: BTreeMap<String, String>,
    pub body: Option<String>,
    pub content_type: Option<String>,
}

impl RequestSpec {
    pub fn with_default_url(&self, url: Option<String>) -> Self {
        if self.url.is_some() {
            return self.clone();
        }

        let mut next = self.clone();
        next.url = url;
        next
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub(super) enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub(super) enum SearchResponseSpec {
    Html(HtmlSearchResponseSpec),
    Json(JsonSearchResponseSpec),
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct HtmlSearchResponseSpec {
    pub item_selector: String,
    #[serde(default)]
    pub fields: BTreeMap<String, HtmlFieldSpec>,
    #[serde(default)]
    pub result_kind: SearchResultKindSpec,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct JsonSearchResponseSpec {
    pub items_path: String,
    #[serde(default)]
    pub fields: BTreeMap<String, JsonFieldSpec>,
    #[serde(default)]
    pub result_kind: SearchResultKindSpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub(super) enum TrackListResponseSpec {
    Html(HtmlTrackListResponseSpec),
    HtmlScript(HtmlScriptTrackListResponseSpec),
    Json(JsonTrackListResponseSpec),
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct HtmlTrackListResponseSpec {
    #[serde(default)]
    pub collection_fields: BTreeMap<String, HtmlFieldSpec>,
    pub track_item_selector: String,
    #[serde(default)]
    pub track_fields: BTreeMap<String, HtmlFieldSpec>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct HtmlScriptTrackListResponseSpec {
    #[serde(default)]
    pub collection_fields: BTreeMap<String, HtmlFieldSpec>,
    pub script_start: String,
    #[serde(default = "default_script_end")]
    pub script_end: String,
    #[serde(default)]
    pub indexed_html_fields: BTreeMap<String, HtmlFieldSpec>,
    #[serde(default)]
    pub track_fields: BTreeMap<String, JsFieldSpec>,
    #[serde(default)]
    pub skip_if_field_contains: Vec<FieldContainsRule>,
    pub strict_indexed_field: Option<String>,
    pub count_mismatch_message: Option<String>,
    pub no_tracks_message: Option<String>,
}

fn default_script_end() -> String {
    "];".to_string()
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct JsonTrackListResponseSpec {
    #[serde(default)]
    pub collection_fields: BTreeMap<String, JsonFieldSpec>,
    pub tracks_path: String,
    #[serde(default)]
    pub track_fields: BTreeMap<String, JsonFieldSpec>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct JsFieldSpec {
    pub field: Option<String>,
    pub value: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub raw: bool,
    #[serde(default)]
    pub transforms: Vec<FieldTransform>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct FieldContainsRule {
    pub field: String,
    pub contains: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct SearchResultKindSpec {
    pub field: Option<String>,
    #[serde(default)]
    pub rules: Vec<SearchResultKindRule>,
    pub default: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SearchResultKindRule {
    pub contains: String,
    pub result: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct HtmlFieldSpec {
    pub selector: Option<String>,
    pub attr: Option<String>,
    #[serde(default)]
    pub text: bool,
    pub value: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub transforms: Vec<FieldTransform>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct JsonFieldSpec {
    pub path: Option<String>,
    pub value: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub transforms: Vec<FieldTransform>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FieldTransform {
    Trim,
    Lowercase,
    Uppercase,
    NormalizeWhitespace,
    DecodeHtml,
    UrlPathId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchItemKind {
    Collection(super::super::CollectionKind),
    Track,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct StoredCredentials {
    pub username: String,
    pub password: String,
}
