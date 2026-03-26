use gpui::prelude::FluentBuilder;
use gpui::{Context, Div, ParentElement, Window};

use super::super::OryxApp;

impl OryxApp {
    pub(super) fn render_shell_overlays(
        &self,
        shell: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        shell
            .child(self.render_notifications(window, cx))
            .when(self.ui_state.read(cx).downloads_modal_open(), |shell| {
                shell.child(self.render_downloads_modal(cx))
            })
            .when(self.ui_state.read(cx).open_url_prompt_open(), |shell| {
                shell.child(self.render_open_url_overlay(window, cx))
            })
            .when(
                self.ui_state.read(cx).provider_auth_prompt().is_some(),
                |shell| shell.child(self.render_provider_auth_overlay(window, cx)),
            )
            .when(
                self.ui_state.read(cx).provider_link_prompt().is_some(),
                |shell| shell.child(self.render_provider_link_overlay(window, cx)),
            )
            .when(self.ui_state.read(cx).import_review_loading(), |shell| {
                shell.child(self.render_import_review_loading_modal(cx))
            })
            .when_some(
                self.ui_state.read(cx).pending_import_review(),
                |shell, review| shell.child(self.render_import_review_modal(&review, window, cx)),
            )
            .when_some(self.ui_state.read(cx).context_menu(), |shell, menu| {
                shell.child(self.render_context_menu_overlay(&menu, cx))
            })
    }
}
