use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;

use super::super::{
    CollectionRef, DownloadRequest, HttpHeader, MusicProvider, ProviderId,
    ProviderMetadataRegistration, SearchResult, SongData, StreamRequest, TrackList, TrackRef,
    TrackSummary, network_agent, register_provider_metadata,
};
use super::config::{
    AuthSpec, HttpMethod, ProviderManifest, RequestSpec, StoredCredentials, ValidationSpec,
};
use super::util::{
    collection_kind_label, cookie_header, merge_response_cookies, mime_type_from_url,
    render_optional_request_url, render_template,
};

pub(super) struct ConfiguredProvider {
    pub(super) manifest: ProviderManifest,
    pub(super) auth_state: Mutex<ConfiguredAuthState>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ConfiguredAuthState {
    pub credentials: Option<StoredCredentials>,
    pub session_cookie: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SmokeTestStatus {
    Skipped,
    PendingAuth,
    Passed,
}

impl ConfiguredProvider {
    pub(super) fn from_manifest(manifest: ProviderManifest) -> Result<Self> {
        let provider_id = ProviderId::parse(&manifest.id)
            .ok_or_else(|| anyhow!("provider id {:?} is invalid", manifest.id))?;
        register_provider_metadata(
            provider_id,
            ProviderMetadataRegistration {
                display_name: Some(manifest.display_name.clone()),
                short_display_name: manifest.short_display_name.clone(),
                search_rank_bias: manifest.search_rank_bias,
                collection_urls: manifest.collection_urls.clone(),
            },
        );

        Ok(Self {
            manifest,
            auth_state: Mutex::new(ConfiguredAuthState::default()),
        })
    }

    pub(super) fn provider_id(&self) -> ProviderId {
        ProviderId::parse(&self.manifest.id)
            .expect("configured provider id should already validate")
    }

    pub(super) fn default_headers(&self, cookie_header: Option<&str>) -> Vec<HttpHeader> {
        let mut headers = self
            .manifest
            .default_headers
            .iter()
            .map(|(name, value)| HttpHeader::new(name.clone(), value.clone()))
            .collect::<Vec<_>>();

        if let Some(cookie_header) = cookie_header {
            headers.push(HttpHeader::new("Cookie", cookie_header));
        }

        headers
    }

    pub(super) fn direct_stream_request(
        &self,
        url: &str,
        cookie_header: Option<&str>,
    ) -> StreamRequest {
        StreamRequest {
            url: url.to_string(),
            headers: self.default_headers(cookie_header),
            supports_byte_ranges: self.manifest.song.supports_byte_ranges,
        }
    }

    pub(super) fn build_context(
        &self,
        query: Option<&str>,
        collection: Option<&CollectionRef>,
        track: Option<&TrackRef>,
        credentials: Option<&StoredCredentials>,
    ) -> HashMap<String, String> {
        let mut context = HashMap::new();
        context.insert("provider.id".to_string(), self.manifest.id.clone());
        context.insert(
            "provider.display_name".to_string(),
            self.manifest.display_name.clone(),
        );

        if let Some(query) = query {
            context.insert("query".to_string(), query.to_string());
        }

        if let Some(collection) = collection {
            context.insert("collection.id".to_string(), collection.id.clone());
            context.insert(
                "collection.kind".to_string(),
                collection_kind_label(collection.kind).to_string(),
            );
            if let Some(url) = collection.canonical_url.as_deref() {
                context.insert("collection.canonical_url".to_string(), url.to_string());
            }
        }

        if let Some(track) = track {
            context.insert("track.id".to_string(), track.id.clone());
            if let Some(url) = track.canonical_url.as_deref() {
                context.insert("track.canonical_url".to_string(), url.to_string());
            }
            if let Some(title_hint) = track.title_hint.as_deref() {
                context.insert("track.title_hint".to_string(), title_hint.to_string());
            }
        }

        if let Some(credentials) = credentials {
            context.insert("auth.username".to_string(), credentials.username.clone());
            context.insert("auth.password".to_string(), credentials.password.clone());
        }

        context
    }

    pub(super) fn ensure_authenticated_session(&self) -> Result<Option<String>> {
        let Some(auth) = self.manifest.auth.as_ref() else {
            return Ok(None);
        };

        let stored_credentials = self
            .auth_state
            .lock()
            .ok()
            .and_then(|state| state.credentials.clone());

        let Some(credentials) = stored_credentials else {
            return Ok(None);
        };

        if let Some(cookie_header) = self
            .auth_state
            .lock()
            .ok()
            .and_then(|state| state.session_cookie.clone())
        {
            return Ok(Some(cookie_header));
        }

        let cookie_header = self.run_auth_flow(auth, &credentials)?;
        if let Ok(mut state) = self.auth_state.lock() {
            state.session_cookie = Some(cookie_header.clone());
        }
        Ok(Some(cookie_header))
    }

    pub(super) fn run_auth_flow(
        &self,
        auth: &AuthSpec,
        credentials: &StoredCredentials,
    ) -> Result<String> {
        let mut cookie_jar = BTreeMap::new();
        let context = self.build_context(None, None, None, Some(credentials));

        if let Some(preflight) = auth.preflight.as_ref() {
            let response = self.execute_request(preflight, &context, None)?;
            merge_response_cookies(&response, &mut cookie_jar);
        }

        let mut submit_request = auth.submit.request.clone();
        submit_request.form.insert(
            auth.submit.username_field.clone(),
            "{auth.username}".to_string(),
        );
        submit_request.form.insert(
            auth.submit.password_field.clone(),
            "{auth.password}".to_string(),
        );

        let cookie_header_value = cookie_header(&cookie_jar);
        let response =
            self.execute_request(&submit_request, &context, cookie_header_value.as_deref())?;
        merge_response_cookies(&response, &mut cookie_jar);

        let Some(cookie_header) = cookie_header(&cookie_jar) else {
            bail!("provider authentication did not return any session cookies");
        };

        if let Some(verify) = auth.verify.as_ref() {
            let response = self.execute_request(&verify.request, &context, Some(&cookie_header))?;
            let body = response
                .into_string()
                .context("failed to read provider auth verification response body")?;

            for required in &verify.contains {
                let rendered = render_template(required, &context);
                if !body.contains(&rendered) {
                    bail!(
                        "provider auth verification failed: expected response to contain {rendered:?}"
                    );
                }
            }
            for blocked in &verify.not_contains {
                let rendered = render_template(blocked, &context);
                if body.contains(&rendered) {
                    bail!(
                        "provider auth verification failed: response still contained {rendered:?}"
                    );
                }
            }
        }

        Ok(cookie_header)
    }

    pub(super) fn execute_request(
        &self,
        request: &RequestSpec,
        context: &HashMap<String, String>,
        cookie_header: Option<&str>,
    ) -> Result<ureq::Response> {
        let url = render_optional_request_url(request.url.as_deref(), context)?;
        let method = request.method.unwrap_or(HttpMethod::Get);
        let mut prepared = match method {
            HttpMethod::Get => network_agent().get(&url),
            HttpMethod::Post => network_agent().post(&url),
        };

        for (name, value) in &self.manifest.default_headers {
            prepared = prepared.set(name, &render_template(value, context));
        }
        for (name, value) in &request.headers {
            prepared = prepared.set(name, &render_template(value, context));
        }
        if let Some(cookie_header) = cookie_header {
            prepared = prepared.set("Cookie", cookie_header);
        }

        for (name, value) in &request.query {
            prepared = prepared.query(name, &render_template(value, context));
        }

        if !request.form.is_empty() {
            let encoded = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(
                    request
                        .form
                        .iter()
                        .map(|(name, value)| (name.as_str(), render_template(value, context))),
                )
                .finish();
            let prepared = prepared.set(
                "Content-Type",
                request
                    .content_type
                    .as_deref()
                    .unwrap_or("application/x-www-form-urlencoded"),
            );
            return prepared
                .send_string(&encoded)
                .with_context(|| format!("{} request to {url} failed", method.as_label()));
        }

        if let Some(body) = request.body.as_deref() {
            let prepared = if let Some(content_type) = request.content_type.as_deref() {
                prepared.set("Content-Type", content_type)
            } else {
                prepared
            };
            return prepared
                .send_string(&render_template(body, context))
                .with_context(|| format!("{} request to {url} failed", method.as_label()));
        }

        prepared
            .call()
            .with_context(|| format!("{} request to {url} failed", method.as_label()))
    }

    pub(super) fn fetch_text(
        &self,
        request: &RequestSpec,
        context: &HashMap<String, String>,
        cookie_header: Option<&str>,
    ) -> Result<String> {
        self.execute_request(request, context, cookie_header)?
            .into_string()
            .context("failed to read provider response body")
    }

    pub(super) fn validate_configuration(&self) -> Result<SmokeTestStatus> {
        let Some(validation) = self.manifest.validation.as_ref() else {
            return Ok(SmokeTestStatus::Skipped);
        };

        if self.manifest.auth.is_some()
            && validation.skip_if_not_authenticated
            && !self.has_stored_credentials()
        {
            return Ok(SmokeTestStatus::PendingAuth);
        }

        let search_body = self.fetch_search_response(&validation.example_query)?;
        let results = self.parse_search_results(&search_body)?;
        if results.len() < validation.expect_min_results {
            bail!(
                "provider validation search returned {} results, expected at least {}",
                results.len(),
                validation.expect_min_results
            );
        }

        if !validation.test_first_collection && !validation.test_first_track {
            return Ok(SmokeTestStatus::Passed);
        }

        if let Some(collection) = results.iter().find_map(|result| match result {
            SearchResult::Collection(collection) => Some(collection.clone()),
            SearchResult::Track(_) => None,
        }) {
            return self.validate_collection_resolution(validation, &collection.reference);
        }

        if validation.test_first_track {
            if let Some(track) = results.iter().find_map(|result| match result {
                SearchResult::Track(track) => Some(track.clone()),
                SearchResult::Collection(_) => None,
            }) {
                self.validate_track_resolution(validation, &track)?;
                return Ok(SmokeTestStatus::Passed);
            }
        }

        bail!("provider validation did not find a collection or track result to resolve")
    }

    fn validate_collection_resolution(
        &self,
        validation: &ValidationSpec,
        collection: &CollectionRef,
    ) -> Result<SmokeTestStatus> {
        let track_list_body = self.fetch_track_list_response(collection)?;
        let track_list = self.parse_track_list(collection, &track_list_body)?;
        if !validation.test_first_track {
            return Ok(SmokeTestStatus::Passed);
        }

        let track = track_list
            .tracks
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("provider validation track list resolved no tracks"))?;
        self.validate_track_resolution(validation, &track)?;
        Ok(SmokeTestStatus::Passed)
    }

    fn validate_track_resolution(
        &self,
        validation: &ValidationSpec,
        track: &TrackSummary,
    ) -> Result<()> {
        let song = self.resolve_song_data(&track.reference)?;
        if validation.require_stream_url && song.stream.url.trim().is_empty() {
            bail!("provider validation resolved an empty stream url");
        }
        Ok(())
    }
}

#[async_trait]
impl MusicProvider for ConfiguredProvider {
    fn id(&self) -> ProviderId {
        self.provider_id()
    }

    fn requires_credentials(&self) -> bool {
        self.manifest
            .auth
            .as_ref()
            .map(|auth| auth.required)
            .unwrap_or(false)
    }

    fn has_stored_credentials(&self) -> bool {
        if self.manifest.auth.is_none() {
            return true;
        }

        self.auth_state
            .lock()
            .map(|state| state.credentials.is_some())
            .unwrap_or(false)
    }

    fn authenticate(&self, username: &str, password: &str) -> Result<()> {
        let Some(auth) = self.manifest.auth.as_ref() else {
            bail!("provider does not define an authentication flow");
        };

        let credentials = StoredCredentials {
            username: username.trim().to_string(),
            password: password.to_string(),
        };
        if credentials.username.is_empty() || credentials.password.is_empty() {
            bail!("provider username and password are required");
        }

        let cookie_header = self.run_auth_flow(auth, &credentials)?;
        let mut state = self
            .auth_state
            .lock()
            .map_err(|_| anyhow!("provider auth state lock was poisoned"))?;
        state.credentials = Some(credentials);
        state.session_cookie = Some(cookie_header);
        Ok(())
    }

    fn restore_credentials(&self, serialized: &str) -> Result<()> {
        let credentials: StoredCredentials = serde_json::from_str(serialized)
            .context("failed to deserialize provider credentials")?;
        let mut state = self
            .auth_state
            .lock()
            .map_err(|_| anyhow!("provider auth state lock was poisoned"))?;
        state.credentials = Some(credentials);
        state.session_cookie = None;
        Ok(())
    }

    fn export_credentials(&self) -> Option<String> {
        self.auth_state
            .lock()
            .ok()
            .and_then(|state| state.credentials.clone())
            .and_then(|credentials| serde_json::to_string(&credentials).ok())
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let body = self.fetch_search_response(query)?;
        self.parse_search_results(&body)
    }

    async fn get_track_list(&self, collection: &CollectionRef) -> Result<TrackList> {
        let body = self.fetch_track_list_response(collection)?;
        self.parse_track_list(collection, &body)
    }

    async fn get_song_data(&self, track: &TrackRef) -> Result<SongData> {
        self.resolve_song_data(track)
    }

    fn get_artwork_request(&self, artwork_url: &str) -> Option<DownloadRequest> {
        Some(DownloadRequest {
            url: artwork_url.to_string(),
            headers: self.default_headers(
                self.auth_state
                    .lock()
                    .ok()
                    .and_then(|state| state.session_cookie.clone())
                    .as_deref(),
            ),
            mime_type: mime_type_from_url(artwork_url),
            supports_byte_ranges: false,
        })
    }
}
