#![allow(dead_code)]

mod generic;
mod local;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) use self::generic::{ConfiguredProviderImport, ConfiguredProviderImportStatus};
pub use self::local::LocalProvider;

pub(crate) const NETWORK_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const NETWORK_READ_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const NETWORK_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn network_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(NETWORK_CONNECT_TIMEOUT)
        .timeout_read(NETWORK_READ_TIMEOUT)
        .timeout_write(NETWORK_WRITE_TIMEOUT)
        .build()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(&'static str);

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ProviderCollectionUrlTemplates {
    pub album: Option<String>,
    pub playlist: Option<String>,
}

impl ProviderCollectionUrlTemplates {
    fn template_for(&self, kind: CollectionKind) -> Option<&str> {
        match kind {
            CollectionKind::Album => self.album.as_deref(),
            CollectionKind::Playlist => self.playlist.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProviderMetadataRegistration {
    pub display_name: Option<String>,
    pub short_display_name: Option<String>,
    pub search_rank_bias: i32,
    pub collection_urls: ProviderCollectionUrlTemplates,
}

#[derive(Clone, Debug, Default)]
struct ProviderMetadata {
    display_name: Option<&'static str>,
    short_display_name: Option<&'static str>,
    search_rank_bias: i32,
    collection_urls: ProviderCollectionUrlTemplates,
}

#[allow(non_upper_case_globals)]
impl ProviderId {
    pub const Local: Self = Self("local");

    pub fn as_str(self) -> &'static str {
        self.0
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = normalize_provider_id(value)?;
        if normalized == Self::Local.as_str() {
            Some(Self::Local)
        } else {
            Some(Self(intern_provider_id(&normalized)))
        }
    }

    pub fn display_name(self) -> &'static str {
        if self == Self::Local {
            "Local"
        } else {
            lookup_provider_metadata(self)
                .and_then(|metadata| metadata.display_name)
                .unwrap_or_else(|| self.as_str())
        }
    }

    pub fn short_display_name(self) -> &'static str {
        if self == Self::Local {
            "Local"
        } else {
            lookup_provider_metadata(self)
                .and_then(|metadata| metadata.short_display_name.or(metadata.display_name))
                .unwrap_or_else(|| self.as_str())
        }
    }

    pub fn search_rank_bias(self) -> i32 {
        if self == Self::Local {
            0
        } else {
            lookup_provider_metadata(self)
                .map(|metadata| metadata.search_rank_bias)
                .unwrap_or(0)
        }
    }

    pub fn collection_url(self, kind: CollectionKind, collection_id: &str) -> Option<String> {
        lookup_provider_metadata(self)
            .and_then(|metadata| {
                metadata
                    .collection_urls
                    .template_for(kind)
                    .map(str::to_string)
            })
            .map(|template| render_single_variable_template(&template, collection_id))
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ProviderId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value)
            .ok_or_else(|| D::Error::custom(format!("invalid provider id {value:?}")))
    }
}

fn normalize_provider_id(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        .then_some(normalized)
}

fn intern_provider_id(value: &str) -> &'static str {
    static INTERNER: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

    let interner = INTERNER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut interner = interner
        .lock()
        .expect("provider id interner lock should not be poisoned");

    if let Some(existing) = interner.get(value) {
        return existing;
    }

    let leaked = Box::leak(value.to_string().into_boxed_str());
    interner.insert(value.to_string(), leaked);
    leaked
}

fn lookup_provider_metadata(id: ProviderId) -> Option<ProviderMetadata> {
    provider_metadata_registry()
        .lock()
        .ok()
        .and_then(|metadata| metadata.get(id.as_str()).cloned())
}

pub(crate) fn register_provider_metadata(id: ProviderId, metadata: ProviderMetadataRegistration) {
    let provider_metadata = ProviderMetadata {
        display_name: metadata
            .display_name
            .map(|value| Box::leak(value.into_boxed_str()) as &'static str),
        short_display_name: metadata
            .short_display_name
            .map(|value| Box::leak(value.into_boxed_str()) as &'static str),
        search_rank_bias: metadata.search_rank_bias,
        collection_urls: metadata.collection_urls,
    };

    if let Ok(mut registry) = provider_metadata_registry().lock() {
        registry.insert(id.as_str(), provider_metadata);
    }
}

fn provider_metadata_registry() -> &'static Mutex<HashMap<&'static str, ProviderMetadata>> {
    static METADATA: OnceLock<Mutex<HashMap<&'static str, ProviderMetadata>>> = OnceLock::new();
    METADATA.get_or_init(|| Mutex::new(HashMap::new()))
}

fn render_single_variable_template(template: &str, id: &str) -> String {
    template.replace("{collection_id}", id).replace("{id}", id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionKind {
    Album,
    Playlist,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionRef {
    pub provider: ProviderId,
    pub id: String,
    pub kind: CollectionKind,
    pub canonical_url: Option<String>,
}

impl CollectionRef {
    pub fn new(
        provider: ProviderId,
        id: impl Into<String>,
        kind: CollectionKind,
        canonical_url: Option<String>,
    ) -> Self {
        Self {
            provider,
            id: id.into(),
            kind,
            canonical_url,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackRef {
    pub provider: ProviderId,
    pub id: String,
    pub canonical_url: Option<String>,
    pub title_hint: Option<String>,
}

impl TrackRef {
    pub fn new(
        provider: ProviderId,
        id: impl Into<String>,
        canonical_url: Option<String>,
        title_hint: Option<String>,
    ) -> Self {
        Self {
            provider,
            id: id.into(),
            canonical_url,
            title_hint,
        }
    }

    pub fn direct_url(
        provider: ProviderId,
        url: impl Into<String>,
        title_hint: Option<String>,
    ) -> Self {
        let url = url.into();
        Self {
            provider,
            id: url.clone(),
            canonical_url: Some(url),
            title_hint,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionSummary {
    pub reference: CollectionRef,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork_url: Option<String>,
    pub track_count: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioFormat {
    Mp3,
    Flac,
    Opus,
    Aac,
    M4a,
    Unknown(String),
}

impl AudioFormat {
    pub fn label(&self) -> &str {
        match self {
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Opus => "Opus",
            Self::Aac => "AAC",
            Self::M4a => "M4A",
            Self::Unknown(label) => label.as_str(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackSummary {
    pub reference: TrackRef,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub collection_id: Option<String>,
    pub collection_title: Option<String>,
    pub collection_subtitle: Option<String>,
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub bitrate_bps: Option<u32>,
    #[serde(default)]
    pub audio_format: Option<AudioFormat>,
    pub artwork_url: Option<String>,
}

impl TrackSummary {
    pub fn unresolved(reference: TrackRef) -> Self {
        let title = reference
            .title_hint
            .clone()
            .unwrap_or_else(|| reference.id.clone());

        Self {
            reference,
            title,
            artist: None,
            album: None,
            collection_id: None,
            collection_title: None,
            collection_subtitle: None,
            duration_seconds: None,
            bitrate_bps: None,
            audio_format: None,
            artwork_url: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SearchResult {
    Collection(CollectionSummary),
    Track(TrackSummary),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackList {
    pub collection: CollectionSummary,
    pub tracks: Vec<TrackSummary>,
}

#[derive(Clone, Debug)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

impl HttpHeader {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DownloadRequest {
    pub url: String,
    pub headers: Vec<HttpHeader>,
    pub mime_type: Option<String>,
    pub supports_byte_ranges: bool,
}

impl DownloadRequest {
    pub fn from_stream(stream: &StreamRequest) -> Self {
        Self {
            url: stream.url.clone(),
            headers: stream.headers.clone(),
            mime_type: None,
            supports_byte_ranges: stream.supports_byte_ranges,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StreamRequest {
    pub url: String,
    pub headers: Vec<HttpHeader>,
    pub supports_byte_ranges: bool,
}

#[derive(Clone, Debug)]
pub struct SongData {
    pub track: TrackSummary,
    pub stream: StreamRequest,
}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    fn display_name(&self) -> &'static str {
        self.id().display_name()
    }

    fn requires_credentials(&self) -> bool {
        false
    }

    fn has_stored_credentials(&self) -> bool {
        true
    }

    fn authenticate(&self, _username: &str, _password: &str) -> Result<()> {
        Ok(())
    }

    fn restore_credentials(&self, _serialized: &str) -> Result<()> {
        Ok(())
    }

    fn export_credentials(&self) -> Option<String> {
        None
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchResult>>;

    async fn get_track_list(&self, collection: &CollectionRef) -> Result<TrackList>;

    async fn get_song_data(&self, track: &TrackRef) -> Result<SongData>;

    fn get_artwork_request(&self, artwork_url: &str) -> Option<DownloadRequest>;
}

pub type SharedProvider = Arc<dyn MusicProvider>;

pub struct ProviderRegistry {
    providers: Vec<SharedProvider>,
}

impl ProviderRegistry {
    pub fn with_defaults(library: Option<&crate::library::Library>) -> Self {
        let mut providers: Vec<SharedProvider> = vec![Arc::new(LocalProvider::new())];

        match generic::load_configured_providers(library) {
            Ok(mut configured) => {
                configured.sort_by_key(|provider| provider.id().as_str());
                providers.extend(configured);
            }
            Err(error) => eprintln!("failed to load configured providers: {error}"),
        }

        Self { providers }
    }

    pub fn all(&self) -> &[SharedProvider] {
        &self.providers
    }

    pub fn get(&self, id: ProviderId) -> Option<&SharedProvider> {
        self.providers.iter().find(|provider| provider.id() == id)
    }
}

pub(crate) fn import_provider_link(
    library: &crate::library::Library,
    encoded: &str,
) -> Result<ConfiguredProviderImport> {
    generic::import_provider_link(library, encoded)
}

pub(crate) fn export_provider_link(
    library: &crate::library::Library,
    provider_id: ProviderId,
) -> Result<String> {
    generic::export_provider_link(library, provider_id)
}
