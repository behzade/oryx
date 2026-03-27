use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AsyncApp, Context, FontWeight, InteractiveElement, MouseButton,
    MouseDownEvent, ParentElement, Styled, WeakEntity, Window, div, ease_out_quint, px, rgb, rgba,
};

use crate::theme;

const DEFAULT_NOTIFICATION_DURATION: Duration = Duration::from_millis(3400);
const ERROR_NOTIFICATION_DURATION: Duration = Duration::from_millis(5600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum NotificationLevel {
    Info,
    Success,
    Error,
}

#[derive(Clone)]
pub(in crate::app) struct NotificationEntry {
    pub(in crate::app) id: u64,
    pub(in crate::app) message: String,
    pub(in crate::app) level: NotificationLevel,
}

pub(in crate::app) struct NotificationCenter {
    next_id: u64,
    entries: Vec<NotificationEntry>,
}

impl NotificationCenter {
    pub(in crate::app) fn new() -> Self {
        Self {
            next_id: 0,
            entries: Vec::new(),
        }
    }

    pub(in crate::app) fn show(
        &mut self,
        message: impl Into<String>,
        level: NotificationLevel,
    ) -> NotificationEntry {
        self.next_id = self.next_id.wrapping_add(1);
        let entry = NotificationEntry {
            id: self.next_id,
            message: message.into(),
            level,
        };
        self.entries.insert(0, entry.clone());
        self.entries.truncate(4);
        entry
    }

    pub(in crate::app) fn dismiss(&mut self, id: u64) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        len_before != self.entries.len()
    }

    pub(in crate::app) fn items(&self) -> &[NotificationEntry] {
        &self.entries
    }
}

impl OryxApp {
    pub(in crate::app) fn show_notification(
        &mut self,
        message: impl Into<String>,
        level: NotificationLevel,
        cx: &mut Context<Self>,
    ) {
        let shown = self
            .notifications
            .update(cx, |notifications, _cx| notifications.show(message, level));

        let duration = match shown.level {
            NotificationLevel::Info | NotificationLevel::Success => DEFAULT_NOTIFICATION_DURATION,
            NotificationLevel::Error => ERROR_NOTIFICATION_DURATION,
        };
        let background = cx.background_executor().clone();

        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let background = background.clone();
            let mut async_cx = cx.clone();
            async move {
                background
                    .spawn(async move {
                        std::thread::sleep(duration);
                    })
                    .await;

                let _ = this.update(&mut async_cx, move |this, cx| {
                    this.dismiss_notification(shown.id, cx);
                });
            }
        })
        .detach();

        cx.notify();
    }

    pub(in crate::app) fn dismiss_notification(&mut self, id: u64, cx: &mut Context<Self>) {
        if self
            .notifications
            .update(cx, |notifications, _cx| notifications.dismiss(id))
        {
            cx.notify();
        }
    }

    pub(in crate::app) fn render_notifications(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let entries = self.notifications.read(cx).items().to_vec();
        if entries.is_empty() {
            return div()
                .absolute()
                .top(px(0.))
                .right(px(0.))
                .w(px(0.))
                .h(px(0.));
        }

        let mut stack = div()
            .absolute()
            .top(px(theme::SPACE_4))
            .right(px(theme::SPACE_4))
            .w(px(360.))
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_2))
            .items_end();

        for entry in entries {
            let (border, background, text) = notification_palette(entry.level);
            let card = div()
                .id(("notification", entry.id))
                .w_full()
                .rounded(px(12.))
                .border_1()
                .border_color(rgba(border))
                .bg(rgba(background))
                .px(px(theme::SPACE_3))
                .py(px(theme::SPACE_3))
                .shadow_lg()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(px(theme::SPACE_2))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(theme::SMALL_SIZE))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(text))
                                        .child(notification_label(entry.level).to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::META_SIZE))
                                        .text_color(rgb(theme::TEXT_PRIMARY))
                                        .child(entry.message.clone()),
                                ),
                        )
                        .child(
                            div()
                                .cursor_pointer()
                                .text_size(px(theme::SMALL_SIZE))
                                .text_color(rgb(theme::TEXT_DIM))
                                .hover(|style| style.text_color(rgb(theme::TEXT_PRIMARY)))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, _event: &MouseDownEvent, _window, cx| {
                                            this.dismiss_notification(entry.id, cx);
                                        },
                                    ),
                                )
                                .child("Dismiss".to_string()),
                        ),
                )
                .with_animation(
                    ("notification-fade", entry.id),
                    Animation::new(DEFAULT_NOTIFICATION_DURATION).with_easing(ease_out_quint()),
                    |element, delta| element.opacity(0.82 + (0.18 * (1.0 - delta))),
                );
            stack = stack.child(card);
        }

        stack
    }
}

fn notification_palette(level: NotificationLevel) -> (u32, u32, u32) {
    match level {
        NotificationLevel::Info => (0xAA5E5145, 0xF024272B, theme::TEXT_MUTED),
        NotificationLevel::Success => (0xAA467A4F, 0xF01E2D23, 0xFF90D2A0),
        NotificationLevel::Error => (0xAA964848, 0xF0301E1E, 0xFFF2A5A5),
    }
}

fn notification_label(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "Notice",
        NotificationLevel::Success => "Done",
        NotificationLevel::Error => "Error",
    }
}

use crate::app::OryxApp;
