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

#[derive(Clone)]
struct ProviderAuthPromptState {
    provider_id: ProviderId,
    error: Option<String>,
    submitting: bool,
}

#[derive(Clone)]
struct ProviderLinkPromptState {
    mode: ProviderLinkPromptMode,
    error: Option<String>,
    submitting: bool,
}

#[derive(Clone)]
enum PrimaryOverlay {
    None,
    OpenUrlPrompt { error: Option<String> },
    ProviderAuthPrompt(ProviderAuthPromptState),
    ProviderLinkPrompt(ProviderLinkPromptState),
}

#[derive(Clone)]
enum ContextualSurface {
    AppMenu,
    ContextMenu(ContextMenuState),
}

#[derive(Clone)]
enum ImportReviewState {
    Idle,
    Analyzing,
    Reviewing(ImportReview),
    Committing(ImportReview),
}

pub(in crate::app) struct UiState {
    primary_overlay: PrimaryOverlay,
    downloads_modal_open: bool,
    contextual_surface: Option<ContextualSurface>,
    import_review: ImportReviewState,
}

impl UiState {
    fn dismiss_contextual_surfaces(&mut self) {
        self.contextual_surface = None;
    }

    fn dismiss_primary_overlay(&mut self) {
        self.dismiss_contextual_surfaces();
        self.primary_overlay = PrimaryOverlay::None;
        self.downloads_modal_open = false;
    }

    pub(in crate::app) fn new() -> Self {
        Self {
            primary_overlay: PrimaryOverlay::None,
            downloads_modal_open: false,
            contextual_surface: None,
            import_review: ImportReviewState::Idle,
        }
    }

    pub(in crate::app) fn provider_auth_prompt(&self) -> Option<ProviderId> {
        match &self.primary_overlay {
            PrimaryOverlay::ProviderAuthPrompt(state) => Some(state.provider_id),
            _ => None,
        }
    }

    pub(in crate::app) fn app_menu_open(&self) -> bool {
        matches!(self.contextual_surface, Some(ContextualSurface::AppMenu))
    }

    pub(in crate::app) fn downloads_modal_open(&self) -> bool {
        self.downloads_modal_open
    }

    pub(in crate::app) fn open_url_prompt_open(&self) -> bool {
        matches!(self.primary_overlay, PrimaryOverlay::OpenUrlPrompt { .. })
    }

    pub(in crate::app) fn open_url_error(&self) -> Option<String> {
        match &self.primary_overlay {
            PrimaryOverlay::OpenUrlPrompt { error } => error.clone(),
            _ => None,
        }
    }

    pub(in crate::app) fn provider_link_prompt(&self) -> Option<ProviderLinkPromptMode> {
        match &self.primary_overlay {
            PrimaryOverlay::ProviderLinkPrompt(state) => Some(state.mode),
            _ => None,
        }
    }

    pub(in crate::app) fn context_menu(&self) -> Option<ContextMenuState> {
        match &self.contextual_surface {
            Some(ContextualSurface::ContextMenu(state)) => Some(state.clone()),
            _ => None,
        }
    }

    pub(in crate::app) fn pending_import_review(&self) -> Option<ImportReview> {
        match &self.import_review {
            ImportReviewState::Reviewing(review) | ImportReviewState::Committing(review) => {
                Some(review.clone())
            }
            _ => None,
        }
    }

    pub(in crate::app) fn import_review_loading(&self) -> bool {
        matches!(
            self.import_review,
            ImportReviewState::Analyzing | ImportReviewState::Committing(_)
        )
    }

    pub(in crate::app) fn provider_auth_submitting(&self) -> bool {
        match &self.primary_overlay {
            PrimaryOverlay::ProviderAuthPrompt(state) => state.submitting,
            _ => false,
        }
    }

    pub(in crate::app) fn provider_auth_error(&self) -> Option<String> {
        match &self.primary_overlay {
            PrimaryOverlay::ProviderAuthPrompt(state) => state.error.clone(),
            _ => None,
        }
    }

    pub(in crate::app) fn provider_link_error(&self) -> Option<String> {
        match &self.primary_overlay {
            PrimaryOverlay::ProviderLinkPrompt(state) => state.error.clone(),
            _ => None,
        }
    }

    pub(in crate::app) fn provider_link_submitting(&self) -> bool {
        match &self.primary_overlay {
            PrimaryOverlay::ProviderLinkPrompt(state) => state.submitting,
            _ => false,
        }
    }

    pub(in crate::app) fn open_provider_auth_prompt(&mut self, provider_id: ProviderId) {
        self.dismiss_primary_overlay();
        self.primary_overlay = PrimaryOverlay::ProviderAuthPrompt(ProviderAuthPromptState {
            provider_id,
            error: None,
            submitting: false,
        });
    }

    pub(in crate::app) fn reset_provider_auth_prompt(&mut self) {
        if matches!(self.primary_overlay, PrimaryOverlay::ProviderAuthPrompt(_)) {
            self.primary_overlay = PrimaryOverlay::None;
        }
    }

    pub(in crate::app) fn open_provider_link_prompt(&mut self, mode: ProviderLinkPromptMode) {
        self.dismiss_primary_overlay();
        self.primary_overlay = PrimaryOverlay::ProviderLinkPrompt(ProviderLinkPromptState {
            mode,
            error: None,
            submitting: false,
        });
    }

    pub(in crate::app) fn reset_provider_link_prompt(&mut self) {
        if matches!(self.primary_overlay, PrimaryOverlay::ProviderLinkPrompt(_)) {
            self.primary_overlay = PrimaryOverlay::None;
        }
    }

    pub(in crate::app) fn set_provider_link_error(&mut self, error: Option<String>) {
        if let PrimaryOverlay::ProviderLinkPrompt(state) = &mut self.primary_overlay {
            state.error = error;
        }
    }

    pub(in crate::app) fn begin_provider_link_submit(&mut self) {
        if let PrimaryOverlay::ProviderLinkPrompt(state) = &mut self.primary_overlay {
            state.submitting = true;
            state.error = None;
        }
    }

    pub(in crate::app) fn finish_provider_link_submit(&mut self) {
        if let PrimaryOverlay::ProviderLinkPrompt(state) = &mut self.primary_overlay {
            state.submitting = false;
        }
    }

    pub(in crate::app) fn set_provider_auth_error(&mut self, error: Option<String>) {
        if let PrimaryOverlay::ProviderAuthPrompt(state) = &mut self.primary_overlay {
            state.error = error;
        }
    }

    pub(in crate::app) fn open_open_url_prompt(&mut self) {
        self.dismiss_contextual_surfaces();
        self.primary_overlay = PrimaryOverlay::OpenUrlPrompt { error: None };
    }

    pub(in crate::app) fn reset_open_url_prompt(&mut self) {
        if matches!(self.primary_overlay, PrimaryOverlay::OpenUrlPrompt { .. }) {
            self.primary_overlay = PrimaryOverlay::None;
        }
    }

    pub(in crate::app) fn set_open_url_error(&mut self, error: Option<String>) {
        if let PrimaryOverlay::OpenUrlPrompt {
            error: current_error,
        } = &mut self.primary_overlay
        {
            *current_error = error;
        }
    }

    pub(in crate::app) fn begin_provider_auth_submit(&mut self) {
        if let PrimaryOverlay::ProviderAuthPrompt(state) = &mut self.primary_overlay {
            state.submitting = true;
            state.error = None;
        }
    }

    pub(in crate::app) fn finish_provider_auth_submit(&mut self) {
        if let PrimaryOverlay::ProviderAuthPrompt(state) = &mut self.primary_overlay {
            state.submitting = false;
        }
    }

    pub(in crate::app) fn toggle_downloads_modal(&mut self) {
        self.dismiss_contextual_surfaces();
        self.downloads_modal_open = !self.downloads_modal_open;
    }

    pub(in crate::app) fn open_downloads_modal(&mut self) {
        self.dismiss_contextual_surfaces();
        self.downloads_modal_open = true;
    }

    pub(in crate::app) fn close_downloads_modal(&mut self) -> bool {
        if self.downloads_modal_open {
            self.downloads_modal_open = false;
            true
        } else {
            false
        }
    }

    pub(in crate::app) fn open_context_menu(
        &mut self,
        position: gpui::Point<Pixels>,
        target: ContextMenuTarget,
    ) {
        self.contextual_surface = Some(ContextualSurface::ContextMenu(ContextMenuState {
            position,
            target,
        }));
    }

    pub(in crate::app) fn close_context_menu(&mut self) -> bool {
        if matches!(
            self.contextual_surface,
            Some(ContextualSurface::ContextMenu(_))
        ) {
            self.contextual_surface = None;
            true
        } else {
            false
        }
    }

    pub(in crate::app) fn toggle_app_menu(&mut self) {
        self.contextual_surface = match self.contextual_surface {
            Some(ContextualSurface::AppMenu) => None,
            _ => Some(ContextualSurface::AppMenu),
        };
    }

    pub(in crate::app) fn close_app_menu(&mut self) -> bool {
        if matches!(self.contextual_surface, Some(ContextualSurface::AppMenu)) {
            self.contextual_surface = None;
            true
        } else {
            false
        }
    }

    pub(in crate::app) fn begin_import_review_analysis(&mut self) {
        self.dismiss_primary_overlay();
        self.import_review = ImportReviewState::Analyzing;
    }

    pub(in crate::app) fn begin_import_review_loading(&mut self) {
        if let ImportReviewState::Reviewing(review) = &self.import_review {
            self.import_review = ImportReviewState::Committing(review.clone());
        }
    }

    pub(in crate::app) fn finish_import_review_loading(&mut self) {
        self.import_review = match &self.import_review {
            ImportReviewState::Analyzing => ImportReviewState::Idle,
            ImportReviewState::Committing(review) => ImportReviewState::Reviewing(review.clone()),
            _ => self.import_review.clone(),
        };
    }

    pub(in crate::app) fn set_pending_import_review(&mut self, review: ImportReview) {
        self.import_review = ImportReviewState::Reviewing(review);
    }

    pub(in crate::app) fn clear_pending_import_review(&mut self) {
        if matches!(
            self.import_review,
            ImportReviewState::Reviewing(_) | ImportReviewState::Committing(_)
        ) {
            self.import_review = ImportReviewState::Idle;
        }
    }

    pub(in crate::app) fn take_pending_import_review(&mut self) -> Option<ImportReview> {
        match std::mem::replace(&mut self.import_review, ImportReviewState::Idle) {
            ImportReviewState::Reviewing(review) | ImportReviewState::Committing(review) => {
                Some(review)
            }
            state => {
                self.import_review = state;
                None
            }
        }
    }

    pub(in crate::app) fn update_pending_import_review(
        &mut self,
        update: impl FnOnce(&mut ImportReview),
    ) -> bool {
        match &mut self.import_review {
            ImportReviewState::Reviewing(review) | ImportReviewState::Committing(review) => {
                update(review);
                review.refresh_album_summaries();
                true
            }
            _ => false,
        }
    }
}
