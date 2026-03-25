use gpui::{
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Styled, Window,
    div, px, rgb, rgba,
};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModalWidth {
    Narrow,
    Medium,
    Wide,
}

impl ModalWidth {
    fn px(self) -> f32 {
        match self {
            Self::Narrow => 420.0,
            Self::Medium => 720.0,
            Self::Wide => 980.0,
        }
    }
}

pub(in crate::app) fn render_modal_overlay(content: impl IntoElement) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(rgba(0xAA000000))
        .flex()
        .items_center()
        .justify_center()
        .p(px(theme::SPACE_4))
        .child(content)
}

pub(in crate::app) fn render_modal_card(width: ModalWidth, content: impl IntoElement) -> gpui::Div {
    render_modal_card_sized(
        width,
        theme::WINDOW_WIDTH - 64.0,
        theme::WINDOW_HEIGHT - 96.0,
        content,
    )
}

pub(in crate::app) fn render_modal_card_sized(
    width: ModalWidth,
    max_width: f32,
    max_height: f32,
    content: impl IntoElement,
) -> gpui::Div {
    div()
        .w(px(width.px()))
        .max_w(px(max_width))
        .max_h(px(max_height))
        .min_h_0()
        .flex()
        .flex_col()
        .rounded(px(14.))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .bg(rgb(theme::SURFACE_FLOATING))
        .overflow_hidden()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            |_event: &MouseDownEvent, window: &mut Window, cx| {
                cx.stop_propagation();
                window.prevent_default();
            },
        )
        .child(content)
}

pub(in crate::app) fn render_modal_body(content: impl IntoElement) -> gpui::Div {
    div()
        .w_full()
        .flex_1()
        .min_h_0()
        .p(px(theme::SPACE_4))
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_3))
        .child(content)
}
