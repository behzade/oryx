mod coordinator;

use std::collections::HashSet;

use crate::provider::{CollectionSummary, ProviderId, SharedProvider, TrackList};

pub(super) struct DiscoverModule {
    enabled_search_providers: HashSet<ProviderId>,
    search_results: Vec<CollectionSummary>,
    selected_collection_id: Option<String>,
    track_list: Option<TrackList>,
    search_loading: bool,
    track_list_loading: bool,
    source_picker_open: bool,
    search_nonce: u64,
    track_list_nonce: u64,
}

impl DiscoverModule {
    pub(super) fn new(
        enabled_search_providers: HashSet<ProviderId>,
        search_results: Vec<CollectionSummary>,
        selected_collection_id: Option<String>,
        track_list: Option<TrackList>,
    ) -> Self {
        Self {
            enabled_search_providers,
            search_results,
            selected_collection_id,
            track_list,
            search_loading: false,
            track_list_loading: false,
            source_picker_open: false,
            search_nonce: 0,
            track_list_nonce: 0,
        }
    }

    pub(super) fn search_results(&self) -> Vec<CollectionSummary> {
        self.search_results.clone()
    }

    pub(super) fn selected_collection_id(&self) -> Option<String> {
        self.selected_collection_id.clone()
    }

    pub(super) fn track_list(&self) -> Option<TrackList> {
        self.track_list.clone()
    }

    pub(super) fn search_loading(&self) -> bool {
        self.search_loading
    }

    pub(super) fn track_list_loading(&self) -> bool {
        self.track_list_loading
    }

    pub(super) fn source_picker_open(&self) -> bool {
        self.source_picker_open
    }

    pub(super) fn enabled_provider_ids(&self, providers: &[SharedProvider]) -> Vec<ProviderId> {
        let mut ids = providers
            .iter()
            .map(|provider| provider.id())
            .filter(|provider_id| *provider_id != ProviderId::Local)
            .filter(|provider_id| self.enabled_search_providers.contains(provider_id))
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub(super) fn active_source_label(&self, providers: &[SharedProvider]) -> String {
        let enabled = self.enabled_provider_ids(providers);
        match enabled.len() {
            0 => "None".to_string(),
            1 => enabled
                .first()
                .and_then(|provider_id| {
                    providers
                        .iter()
                        .find(|provider| provider.id() == *provider_id)
                        .map(|provider| provider.display_name().to_string())
                })
                .unwrap_or_else(|| "1 enabled".to_string()),
            count => format!("{count} enabled"),
        }
    }

    pub(super) fn active_providers(&self, providers: &[SharedProvider]) -> Vec<SharedProvider> {
        self.enabled_provider_ids(providers)
            .into_iter()
            .filter_map(|provider_id| {
                providers
                    .iter()
                    .find(|provider| provider.id() == provider_id)
                    .cloned()
            })
            .collect()
    }

    pub(super) fn is_provider_enabled(&self, provider_id: ProviderId) -> bool {
        self.enabled_search_providers.contains(&provider_id)
    }

    pub(super) fn enable_provider(&mut self, provider_id: ProviderId) {
        self.enabled_search_providers.insert(provider_id);
    }

    pub(super) fn disable_provider(&mut self, provider_id: ProviderId) {
        self.enabled_search_providers.remove(&provider_id);
    }

    pub(super) fn retain_available_providers(&mut self, providers: &[SharedProvider]) {
        let available = providers
            .iter()
            .map(|provider| provider.id())
            .collect::<HashSet<_>>();
        self.enabled_search_providers
            .retain(|provider_id| available.contains(provider_id));
    }

    pub(super) fn close_source_picker(&mut self) {
        self.source_picker_open = false;
    }

    pub(super) fn toggle_source_picker(&mut self) {
        self.source_picker_open = !self.source_picker_open;
    }

    pub(super) fn reset_scope(&mut self) {
        self.search_loading = false;
        self.track_list_loading = false;
        self.search_results.clear();
        self.selected_collection_id = None;
        self.track_list = None;
        self.source_picker_open = false;
    }

    pub(super) fn begin_search(&mut self) -> u64 {
        self.search_nonce += 1;
        self.search_loading = true;
        self.track_list_loading = false;
        self.search_results.clear();
        self.selected_collection_id = None;
        self.track_list = None;
        self.search_nonce
    }

    pub(super) fn search_nonce_matches(&self, nonce: u64) -> bool {
        self.search_nonce == nonce
    }

    pub(super) fn finish_search(&mut self, results: Vec<CollectionSummary>) {
        self.search_loading = false;
        self.search_results = results;
    }

    pub(super) fn fail_search(&mut self) {
        self.search_loading = false;
    }

    pub(super) fn replace_search_results(&mut self, results: Vec<CollectionSummary>) {
        self.search_results = results;
    }

    pub(super) fn should_skip_collection_load(&self, collection_key: &str) -> bool {
        self.selected_collection_id.as_deref() == Some(collection_key) && self.track_list_loading
    }

    pub(super) fn begin_collection_load(&mut self, collection_key: String) -> u64 {
        self.track_list_nonce += 1;
        self.selected_collection_id = Some(collection_key);
        self.track_list_nonce
    }

    pub(super) fn begin_track_list_fetch(&mut self) {
        self.track_list_loading = true;
    }

    pub(super) fn track_list_nonce_matches(&self, nonce: u64) -> bool {
        self.track_list_nonce == nonce
    }

    pub(super) fn finish_track_list_load(&mut self, track_list: TrackList) {
        self.track_list_loading = false;
        self.track_list = Some(track_list);
    }

    pub(super) fn fail_track_list_load(&mut self) {
        self.track_list_loading = false;
    }

    pub(super) fn replace_track_list(&mut self, track_list: TrackList) {
        self.track_list = Some(track_list);
    }

    pub(super) fn sync_browser_playback_context(
        &mut self,
        collection_id: Option<String>,
        track_list: Option<TrackList>,
    ) {
        self.selected_collection_id = collection_id;
        self.track_list = track_list;
    }
}
