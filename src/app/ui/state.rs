use gpui::Pixels;

use crate::library::ImportReview;
use crate::provider::ProviderId;

#[derive(Clone)]
pub(in crate::app) enum ContextMenuTarget {
    LocalAlbum {
        provider: ProviderId,
        collection_id: String,
        title: String,
    },
    LocalTrack {
        provider: ProviderId,
        track_id: String,
        title: String,
    },
}

#[derive(Clone)]
pub(in crate::app) struct ContextMenuState {
    pub(in crate::app) position: gpui::Point<Pixels>,
    pub(in crate::app) target: ContextMenuTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ProviderLinkPromptMode {
    Import,
    Export,
}

pub(in crate::app) struct UiState {
    pub(in crate::app) downloads_modal_open: bool,
    pub(in crate::app) provider_auth_prompt: Option<ProviderId>,
    pub(in crate::app) provider_auth_error: Option<String>,
    pub(in crate::app) provider_auth_submitting: bool,
    pub(in crate::app) provider_link_prompt: Option<ProviderLinkPromptMode>,
    pub(in crate::app) provider_link_error: Option<String>,
    pub(in crate::app) provider_link_submitting: bool,
    pub(in crate::app) context_menu: Option<ContextMenuState>,
    pub(in crate::app) pending_import_review: Option<ImportReview>,
    pub(in crate::app) import_review_loading: bool,
}

impl UiState {
    pub(in crate::app) fn new() -> Self {
        Self {
            downloads_modal_open: false,
            provider_auth_prompt: None,
            provider_auth_error: None,
            provider_auth_submitting: false,
            provider_link_prompt: None,
            provider_link_error: None,
            provider_link_submitting: false,
            context_menu: None,
            pending_import_review: None,
            import_review_loading: false,
        }
    }

    pub(in crate::app) fn provider_auth_prompt(&self) -> Option<ProviderId> {
        self.provider_auth_prompt
    }

    pub(in crate::app) fn downloads_modal_open(&self) -> bool {
        self.downloads_modal_open
    }

    pub(in crate::app) fn provider_link_prompt(&self) -> Option<ProviderLinkPromptMode> {
        self.provider_link_prompt
    }

    pub(in crate::app) fn context_menu(&self) -> Option<ContextMenuState> {
        self.context_menu.clone()
    }

    pub(in crate::app) fn pending_import_review(&self) -> Option<ImportReview> {
        self.pending_import_review.clone()
    }

    pub(in crate::app) fn import_review_loading(&self) -> bool {
        self.import_review_loading
    }

    pub(in crate::app) fn provider_auth_submitting(&self) -> bool {
        self.provider_auth_submitting
    }

    pub(in crate::app) fn provider_auth_error(&self) -> Option<String> {
        self.provider_auth_error.clone()
    }

    pub(in crate::app) fn provider_link_error(&self) -> Option<String> {
        self.provider_link_error.clone()
    }

    pub(in crate::app) fn provider_link_submitting(&self) -> bool {
        self.provider_link_submitting
    }

    pub(in crate::app) fn open_provider_auth_prompt(&mut self, provider_id: ProviderId) {
        self.downloads_modal_open = false;
        self.provider_auth_prompt = Some(provider_id);
        self.provider_auth_error = None;
        self.provider_auth_submitting = false;
    }

    pub(in crate::app) fn reset_provider_auth_prompt(&mut self) {
        self.provider_auth_prompt = None;
        self.provider_auth_error = None;
        self.provider_auth_submitting = false;
    }

    pub(in crate::app) fn open_provider_link_prompt(&mut self, mode: ProviderLinkPromptMode) {
        self.downloads_modal_open = false;
        self.provider_auth_prompt = None;
        self.provider_auth_error = None;
        self.provider_auth_submitting = false;
        self.provider_link_prompt = Some(mode);
        self.provider_link_error = None;
        self.provider_link_submitting = false;
    }

    pub(in crate::app) fn reset_provider_link_prompt(&mut self) {
        self.provider_link_prompt = None;
        self.provider_link_error = None;
        self.provider_link_submitting = false;
    }

    pub(in crate::app) fn set_provider_link_error(&mut self, error: Option<String>) {
        self.provider_link_error = error;
    }

    pub(in crate::app) fn begin_provider_link_submit(&mut self) {
        self.provider_link_submitting = true;
        self.provider_link_error = None;
    }

    pub(in crate::app) fn finish_provider_link_submit(&mut self) {
        self.provider_link_submitting = false;
    }

    pub(in crate::app) fn set_provider_auth_error(&mut self, error: Option<String>) {
        self.provider_auth_error = error;
    }

    pub(in crate::app) fn begin_provider_auth_submit(&mut self) {
        self.provider_auth_submitting = true;
        self.provider_auth_error = None;
    }

    pub(in crate::app) fn finish_provider_auth_submit(&mut self) {
        self.provider_auth_submitting = false;
    }

    pub(in crate::app) fn toggle_downloads_modal(&mut self) {
        self.downloads_modal_open = !self.downloads_modal_open;
    }

    pub(in crate::app) fn close_downloads_modal(&mut self) -> bool {
        let was_open = self.downloads_modal_open;
        self.downloads_modal_open = false;
        was_open
    }

    pub(in crate::app) fn open_context_menu(
        &mut self,
        position: gpui::Point<Pixels>,
        target: ContextMenuTarget,
    ) {
        self.context_menu = Some(ContextMenuState { position, target });
    }

    pub(in crate::app) fn close_context_menu(&mut self) -> bool {
        self.context_menu.take().is_some()
    }

    pub(in crate::app) fn begin_import_review_analysis(&mut self) {
        self.close_downloads_modal();
        self.reset_provider_auth_prompt();
        self.import_review_loading = true;
    }

    pub(in crate::app) fn begin_import_review_loading(&mut self) {
        self.import_review_loading = true;
    }

    pub(in crate::app) fn finish_import_review_loading(&mut self) {
        self.import_review_loading = false;
    }

    pub(in crate::app) fn set_pending_import_review(&mut self, review: ImportReview) {
        self.pending_import_review = Some(review);
    }

    pub(in crate::app) fn clear_pending_import_review(&mut self) {
        self.pending_import_review = None;
    }

    pub(in crate::app) fn take_pending_import_review(&mut self) -> Option<ImportReview> {
        self.pending_import_review.take()
    }
}
