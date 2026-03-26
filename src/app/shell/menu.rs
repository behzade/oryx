use gpui::prelude::*;
use gpui::{
    App, Context, FontWeight, MouseButton, MouseDownEvent, ParentElement, Styled, Window, div, px,
    rgb,
};

use crate::app::ui::ProviderLinkPromptMode;
use crate::theme;

use super::super::OryxApp;

const APP_MENU_WIDTH: f32 = 232.0;
const APP_MENU_TOP_OFFSET: f32 = 62.0;

impl OryxApp {
    pub(super) fn render_app_menu_overlay(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let panel = div()
            .absolute()
            .top(px(APP_MENU_TOP_OFFSET))
            .right(px(theme::SPACE_4))
            .w(px(APP_MENU_WIDTH))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .bg(rgb(theme::SURFACE_FLOATING))
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                |_event: &MouseDownEvent, window: &mut Window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                },
            )
            .p(px(theme::SPACE_1))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(self.render_app_menu_section_label("Library"))
            .child(app_menu_row(
                "Import...",
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(cx);
                    this.prompt_for_import_folder(window, cx);
                }),
            ))
            .child(app_menu_row(
                "Open Media...",
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(cx);
                    this.open_url_prompt(window, cx);
                }),
            ))
            .child(app_menu_row(
                "Import Provider Link...",
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(cx);
                    this.open_provider_link_prompt(ProviderLinkPromptMode::Import, window, cx);
                }),
            ))
            .child(app_menu_row(
                "Export Provider Link...",
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(cx);
                    this.open_provider_link_prompt(ProviderLinkPromptMode::Export, window, cx);
                }),
            ))
            .child(app_menu_row(
                "Refresh Local Artwork",
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(cx);
                    this.refresh_local_artwork(window, cx);
                }),
            ))
            .child(menu_divider())
            .child(self.render_app_menu_section_label("Playback"))
            .child(app_menu_row(
                "Play/Pause",
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                    this.toggle_playback_from_ui(cx);
                }),
            ))
            .child(app_menu_row(
                "Previous Track",
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                    this.play_previous(cx);
                }),
            ))
            .child(app_menu_row(
                "Next Track",
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                    this.play_next(cx);
                }),
            ))
            .child(menu_divider())
            .child(self.render_app_menu_section_label("Window"))
            .child(app_menu_row(
                "Downloads",
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                    this.toggle_downloads_modal(cx);
                }),
            ))
            .child(app_menu_row(
                "Minimize",
                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(cx);
                    crate::platform::minimize_window(window);
                }),
            ))
            .child(app_menu_row(
                "Quit Oryx",
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                    this.quit_app(cx);
                }),
            ));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.close_app_menu(cx);
                }),
            )
            .child(panel)
    }

    fn render_app_menu_section_label(&self, label: &str) -> gpui::Div {
        div()
            .px(px(theme::SPACE_2))
            .pt(px(theme::SPACE_2))
            .pb(px(theme::SPACE_1))
            .text_size(px(theme::SMALL_SIZE))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::TEXT_DIM))
            .child(label.to_string())
    }
}

fn menu_divider() -> gpui::Div {
    div()
        .mx(px(theme::SPACE_2))
        .my(px(theme::SPACE_1))
        .h(px(1.))
        .bg(rgb(theme::BORDER_SUBTLE))
}

fn app_menu_row(
    label: &str,
    listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .px(px(theme::SPACE_2))
        .py(px(theme::SPACE_2))
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(theme::META_SIZE))
        .text_color(rgb(theme::TEXT_MUTED))
        .hover(|style| {
            style
                .bg(rgb(theme::ACCENT_PRIMARY_LIGHT))
                .text_color(rgb(theme::ACCENT_PRIMARY))
        })
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            listener(event, window, cx);
        })
        .child(label.to_string())
}
