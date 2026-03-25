use gpui::{AsyncApp, Context, Window};

use crate::provider::ProviderId;

use super::super::OryxApp;
use super::super::text_input::TextInputId;
use super::NotificationLevel;

impl OryxApp {
    pub(in crate::app) fn open_provider_auth_prompt(
        &mut self,
        provider_id: ProviderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        self.provider_auth_username_input.reset(String::new());
        self.provider_auth_password_input.reset(String::new());
        self.update_ui_state(cx, |state| state.open_provider_auth_prompt(provider_id));
        self.focus_text_input(&TextInputId::ProviderAuthUsername, window);
        self.status_message = Some(format!(
            "Sign in to {} to use authenticated playback URLs.",
            provider_id
        ));
        cx.notify();
    }

    fn reset_provider_auth_prompt(&mut self, cx: &mut Context<Self>) {
        self.provider_auth_username_input.reset(String::new());
        self.provider_auth_password_input.reset(String::new());
        self.update_ui_state(cx, |state| state.reset_provider_auth_prompt());
    }

    pub(in crate::app) fn close_provider_auth_prompt(&mut self, cx: &mut Context<Self>) {
        if self.ui_state.read(cx).provider_auth_submitting() {
            return;
        }

        self.reset_provider_auth_prompt(cx);
        self.status_message = Some("Provider sign-in cancelled.".to_string());
        self.show_notification("Provider sign-in cancelled.", NotificationLevel::Info, cx);
        cx.notify();
    }

    pub(in crate::app) fn submit_provider_auth(&mut self, cx: &mut Context<Self>) {
        let Some(provider_id) = self.ui_state.read(cx).provider_auth_prompt() else {
            return;
        };
        let username = self
            .provider_auth_username_input
            .content()
            .trim()
            .to_string();
        let password = self.provider_auth_password_input.content().to_string();
        if username.is_empty() || password.is_empty() {
            self.update_ui_state(cx, |state| {
                state.set_provider_auth_error(Some(
                    "Username and password are required.".to_string(),
                ));
            });
            cx.notify();
            return;
        }

        let Some(provider) = self.provider_for_id(provider_id) else {
            self.update_ui_state(cx, |state| {
                state.set_provider_auth_error(Some(
                    "Selected provider is not available.".to_string(),
                ));
            });
            cx.notify();
            return;
        };

        let library = self.library.clone();
        self.update_ui_state(cx, |state| {
            state.begin_provider_auth_submit();
        });
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    provider.authenticate(&username, &password)?;
                    let serialized = provider.export_credentials().ok_or_else(|| {
                        anyhow::anyhow!("Provider did not return persisted credentials")
                    })?;
                    library.save_provider_auth(provider_id, &serialized)?;
                    Ok::<(), anyhow::Error>(())
                })
                .await;

            let _ = this.update(cx, move |this, cx| {
                this.update_ui_state(cx, |state| {
                    state.finish_provider_auth_submit();
                });
                match result {
                    Ok(()) => {
                        this.reset_provider_auth_prompt(cx);
                        this.discover.update(cx, |discover, _cx| {
                            discover.enable_provider(provider_id);
                        });
                        let provider_name = this
                            .provider_for_id(provider_id)
                            .map(|provider| provider.display_name().to_string())
                            .unwrap_or_else(|| provider_id.display_name().to_string());
                        this.reset_discover_scope(
                            format!("Enabled {provider_name} for search."),
                            cx,
                        );
                        this.show_notification(
                            format!("Signed in to {}.", provider_name),
                            NotificationLevel::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        let message = format!("Sign-in failed: {error}");
                        this.update_ui_state(cx, |state| {
                            state.set_provider_auth_error(Some(message.clone()));
                        });
                        this.show_notification(message, NotificationLevel::Error, cx);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}
