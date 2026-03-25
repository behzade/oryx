use gpui::{App, AsyncApp, ClipboardItem, Context, Window};

use crate::provider::{
    ConfiguredProviderImportStatus, ProviderId, ProviderRegistry, export_provider_link,
    import_provider_link,
};

use super::super::OryxApp;
use super::super::text_input::TextInputId;
use super::{NotificationLevel, ProviderLinkPromptMode};

impl OryxApp {
    pub(in crate::app) fn open_provider_link_prompt(
        &mut self,
        mode: ProviderLinkPromptMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        let initial_value = match mode {
            ProviderLinkPromptMode::Import => String::new(),
            ProviderLinkPromptMode::Export => self
                .default_provider_link_export_target(cx)
                .map(|provider_id| provider_id.as_str().to_string())
                .unwrap_or_default(),
        };
        self.provider_link_input.reset(initial_value);
        self.update_ui_state(cx, |state| state.open_provider_link_prompt(mode));
        self.focus_text_input(&TextInputId::ProviderLink, window);
        self.status_message = Some(match mode {
            ProviderLinkPromptMode::Import => {
                "Paste a provider link or provider TOML to import it.".to_string()
            }
            ProviderLinkPromptMode::Export => {
                "Enter a provider id to export its active config as a compact link.".to_string()
            }
        });
        cx.notify();
    }

    pub(in crate::app) fn close_provider_link_prompt(&mut self, cx: &mut Context<Self>) {
        if self.ui_state.read(cx).provider_link_submitting() {
            return;
        }

        self.provider_link_input.reset(String::new());
        self.update_ui_state(cx, |state| state.reset_provider_link_prompt());
        self.status_message = Some("Provider link prompt cancelled.".to_string());
        self.show_notification(
            "Provider link prompt cancelled.",
            NotificationLevel::Info,
            cx,
        );
        cx.notify();
    }

    pub(in crate::app) fn submit_provider_link_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(mode) = self.ui_state.read(cx).provider_link_prompt() else {
            return;
        };

        match mode {
            ProviderLinkPromptMode::Import => self.submit_provider_link_import(cx),
            ProviderLinkPromptMode::Export => self.submit_provider_link_export(cx),
        }
    }

    fn submit_provider_link_import(&mut self, cx: &mut Context<Self>) {
        let encoded = self.provider_link_input.content().trim().to_string();
        if encoded.is_empty() {
            self.update_ui_state(cx, |state| {
                state.set_provider_link_error(Some(
                    "A provider link or TOML payload is required.".to_string(),
                ));
            });
            cx.notify();
            return;
        }

        let library = self.library.clone();
        self.update_ui_state(cx, |state| {
            state.begin_provider_link_submit();
        });
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { import_provider_link(&library, &encoded) })
                .await;

            let _ = this.update(cx, move |this, cx| {
                this.update_ui_state(cx, |state| state.finish_provider_link_submit());
                match result {
                    Ok(result) => {
                        this.reload_provider_registry(result.provider_id, cx);
                        this.provider_link_input.reset(String::new());
                        this.update_ui_state(cx, |state| state.reset_provider_link_prompt());
                        let message = match result.status {
                            ConfiguredProviderImportStatus::Activated => format!(
                                "Imported provider '{}' and activated the new config.",
                                result.provider_id
                            ),
                            ConfiguredProviderImportStatus::PendingAuth => format!(
                                "Imported provider '{}', but validation is waiting for sign-in. The last validated config is still active.",
                                result.provider_id
                            ),
                            ConfiguredProviderImportStatus::RevertedToLastValidated => format!(
                                "Imported provider '{}', but validation failed. The last validated config is still active.",
                                result.provider_id
                            ),
                        };
                        this.status_message = Some(message.clone());
                        let level = match result.status {
                            ConfiguredProviderImportStatus::Activated => NotificationLevel::Success,
                            ConfiguredProviderImportStatus::PendingAuth => NotificationLevel::Info,
                            ConfiguredProviderImportStatus::RevertedToLastValidated => {
                                NotificationLevel::Error
                            }
                        };
                        this.show_notification(message, level, cx);
                        this.persist_session_snapshot(cx);
                        cx.notify();
                    }
                    Err(error) => {
                        let message = format!("Provider import failed: {error:#}");
                        this.update_ui_state(cx, |state| {
                            state.set_provider_link_error(Some(message.clone()));
                        });
                        this.show_notification(message, NotificationLevel::Error, cx);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn submit_provider_link_export(&mut self, cx: &mut Context<Self>) {
        let provider_id_text = self.provider_link_input.content().trim();
        let Some(provider_id) = ProviderId::parse(provider_id_text) else {
            self.update_ui_state(cx, |state| {
                state.set_provider_link_error(Some("Enter a valid provider id.".to_string()));
            });
            cx.notify();
            return;
        };

        match export_provider_link(&self.library, provider_id) {
            Ok(link) => {
                cx.write_to_clipboard(ClipboardItem::new_string(link));
                self.provider_link_input.reset(String::new());
                self.update_ui_state(cx, |state| state.reset_provider_link_prompt());
                let message = format!(
                    "Copied provider link for '{}' to the clipboard.",
                    provider_id
                );
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Success, cx);
                cx.notify();
            }
            Err(error) => {
                let message = format!("Provider export failed: {error:#}");
                self.update_ui_state(cx, |state| {
                    state.set_provider_link_error(Some(message.clone()));
                });
                self.show_notification(message, NotificationLevel::Error, cx);
                cx.notify();
            }
        }
    }

    fn reload_provider_registry(&mut self, imported_provider: ProviderId, cx: &mut Context<Self>) {
        let registry = ProviderRegistry::with_defaults(Some(&self.library));
        self.providers = registry.all().to_vec();
        self.discover.update(cx, |discover, _cx| {
            discover.retain_available_providers(&self.providers);
            if let Some(provider) = self
                .providers
                .iter()
                .find(|provider| provider.id() == imported_provider)
                && (!provider.requires_credentials() || provider.has_stored_credentials())
            {
                discover.enable_provider(provider.id());
            }
        });
    }

    fn default_provider_link_export_target(&self, cx: &App) -> Option<ProviderId> {
        self.discover
            .read(cx)
            .track_list()
            .map(|track_list| track_list.collection.reference.provider)
            .or_else(|| {
                self.discover
                    .read(cx)
                    .search_results()
                    .first()
                    .map(|collection| collection.reference.provider)
            })
            .or_else(|| {
                self.searchable_provider_ids()
                    .into_iter()
                    .find(|provider_id| *provider_id != ProviderId::Local)
            })
    }
}
