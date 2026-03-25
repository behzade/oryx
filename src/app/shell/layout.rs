use gpui::prelude::*;
use gpui::{Context, IntoElement, Render, Window, div, rgb};

use crate::keybindings::APP_KEY_CONTEXT;
use crate::theme;

use super::super::{OryxApp, browse};

impl OryxApp {
    pub(super) fn render_shell(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let shell = self.render_shell_base(window, cx);
        let shell = self.bind_shell_actions(shell, cx);
        self.render_shell_overlays(shell, window, cx)
    }

    fn render_shell_base(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .size_full()
            .relative()
            .key_context(APP_KEY_CONTEXT)
            .track_focus(&self.shell_focus_handle)
            .bg(rgb(theme::BG_CANVAS))
            .text_color(rgb(theme::TEXT_PRIMARY))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(self.render_search_box(cx))
            .child(browse::section_divider())
            .child(self.render_lists(window, cx))
            .child(browse::section_divider())
            .child(self.render_now_playing(cx))
    }
}

impl Render for OryxApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title("Oryx");
        self.render_shell(window, cx)
    }
}
