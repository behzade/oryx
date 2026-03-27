use std::path::PathBuf;

use gpui::prelude::FluentBuilder;
use gpui::{
    Context, FontWeight, InteractiveElement, MouseButton, MouseDownEvent, ParentElement, Styled,
    Window, div, px, rgb,
};

use crate::library::{PersistedExternalDownload, PersistedExternalDownloadState};
use crate::progressive::ProgressiveDownload;
use crate::theme;
use crate::url_media::{open_media_with_default_app, validate_open_url_input};

use super::OryxApp;
use super::text_input::{TextInputElement, TextInputId};
use super::transfer_state::DownloadItemState;
use super::ui::{self, NotificationLevel};

impl OryxApp {
    pub(super) fn restore_external_downloads(
        &mut self,
        downloads: Vec<PersistedExternalDownload>,
        cx: &mut Context<Self>,
    ) {
        let mut retained = Vec::new();
        let mut pending = Vec::new();
        for download in downloads {
            match download.state {
                PersistedExternalDownloadState::Pending => pending.push(download),
                PersistedExternalDownloadState::Completed { .. }
                | PersistedExternalDownloadState::Failed { .. } => retained.push(download),
            }
        }

        self.transfer_state.update(cx, |state, _cx| {
            state.restore_persisted_external_downloads(retained);
        });
        for download in pending {
            let progress = restored_external_download_progress(&download);
            self.transfer.queue_external_url_download_with_id(
                download.id,
                download.source_url,
                download.destination,
                Some(progress),
                Some(download.title),
            );
        }
    }

    pub(super) fn open_url_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discover.update(cx, |discover, _cx| {
            discover.close_source_picker();
        });
        self.open_url_input.reset(String::new());
        self.update_ui_state(cx, |state| state.open_open_url_prompt());
        self.focus_text_input(&TextInputId::OpenUrl, window);
        self.status_message = Some(
            "Paste a media URL to queue it in Downloads and open it with your default app."
                .to_string(),
        );
        cx.notify();
    }

    pub(super) fn close_open_url_prompt(&mut self, cx: &mut Context<Self>) {
        if !self.ui_state.read(cx).open_url_prompt_open() {
            return;
        }

        self.open_url_input.reset(String::new());
        self.update_ui_state(cx, |state| state.reset_open_url_prompt());
        self.status_message = Some("Open Media cancelled.".to_string());
        self.show_notification("Open Media cancelled.", NotificationLevel::Info, cx);
        cx.notify();
    }

    pub(super) fn submit_open_url_prompt(&mut self, cx: &mut Context<Self>) {
        if !self.ui_state.read(cx).open_url_prompt_open() {
            return;
        }

        let url = self.open_url_input.content().trim().to_string();
        if url.is_empty() {
            self.update_ui_state(cx, |state| {
                state.set_open_url_error(Some("A URL is required.".to_string()));
            });
            cx.notify();
            return;
        }
        let normalized_url = match validate_open_url_input(&url) {
            Ok(parsed) => parsed.to_string(),
            Err(error) => {
                self.update_ui_state(cx, |state| {
                    state.set_open_url_error(Some(format!("{error:#}")));
                });
                cx.notify();
                return;
            }
        };

        let existing = self
            .transfer_state
            .read(cx)
            .external_download_for_url(&normalized_url);
        if let Some(existing) = existing {
            self.open_url_input.reset(String::new());
            self.update_ui_state(cx, |state| {
                state.reset_open_url_prompt();
                state.open_downloads_modal();
            });
            match existing.state {
                DownloadItemState::Queued { .. } | DownloadItemState::Active { .. } => {
                    self.status_message =
                        Some(format!("'{}' is already downloading.", existing.title));
                    cx.notify();
                    return;
                }
                DownloadItemState::Completed { .. } => {
                    self.status_message =
                        Some(format!("'{}' is already in Downloads.", existing.title));
                    cx.notify();
                    return;
                }
                DownloadItemState::Failed { destination, .. } => {
                    self.retry_external_download(existing.id, normalized_url, destination, cx);
                    return;
                }
            }
        }

        self.transfer
            .queue_external_url_download(normalized_url.clone());
        self.open_url_input.reset(String::new());
        self.update_ui_state(cx, |state| {
            state.reset_open_url_prompt();
            state.open_downloads_modal();
        });
        self.status_message = Some(format!("Queued '{}' for download.", normalized_url));
        cx.notify();
    }

    pub(super) fn cancel_external_download(&mut self, download_id: String, cx: &mut Context<Self>) {
        let cancelled = self.transfer_state.update(cx, |state, _cx| {
            state.cancel_external_download(&download_id)
        });
        if !cancelled {
            return;
        }

        self.status_message = Some("Cancelled external download.".to_string());
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(super) fn pause_external_download(&mut self, download_id: String, cx: &mut Context<Self>) {
        let paused = self
            .transfer_state
            .update(cx, |state, _cx| state.pause_external_download(&download_id));
        if !paused {
            return;
        }

        self.status_message = Some("Paused external download.".to_string());
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(super) fn resume_external_download(&mut self, download_id: String, cx: &mut Context<Self>) {
        let resumed = self.transfer_state.update(cx, |state, _cx| {
            state.resume_external_download(&download_id)
        });
        if !resumed {
            return;
        }

        self.status_message = Some("Resumed external download.".to_string());
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(super) fn open_external_download(&mut self, destination: PathBuf, cx: &mut Context<Self>) {
        match open_media_with_default_app(&destination) {
            Ok(()) => {
                let message = format!("Opened '{}' with the default app.", destination.display());
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Info, cx);
            }
            Err(error) => {
                let message = format!(
                    "Failed to open '{}' with the default app: {error:#}",
                    destination.display()
                );
                self.status_message = Some(message.clone());
                self.show_notification(message, NotificationLevel::Error, cx);
            }
        }
    }

    pub(super) fn retry_external_download(
        &mut self,
        download_id: String,
        source_url: String,
        destination: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.transfer.queue_external_url_download_with_id(
            download_id,
            source_url.clone(),
            destination,
            None,
            None,
        );
        self.status_message = Some(format!("Retrying '{source_url}'."));
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(super) fn render_open_url_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let focused = self
            .text_input_focus_handle(&TextInputId::OpenUrl)
            .is_focused(window);
        let prompt_field = div()
            .w_full()
            .min_h(px(42.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(if focused {
                theme::ACCENT_PRIMARY
            } else {
                theme::BORDER_SUBTLE
            }))
            .bg(rgb(theme::SURFACE_FLOATING))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_2))
            .track_focus(self.text_input_focus_handle(&TextInputId::OpenUrl))
            .child(
                div()
                    .line_height(px(20.))
                    .text_size(px(theme::META_SIZE))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.on_text_input_mouse_down(TextInputId::OpenUrl, event, window, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(TextInputId::OpenUrl, event, window, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(TextInputId::OpenUrl, event, window, cx);
                        }),
                    )
                    .on_mouse_move(
                        cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                            this.on_text_input_mouse_move(TextInputId::OpenUrl, event, window, cx);
                        }),
                    )
                    .child(TextInputElement {
                        app: cx.entity(),
                        input_id: TextInputId::OpenUrl,
                        placeholder: "https://example.com/video",
                        masked: false,
                    }),
            );

        ui::render_modal_overlay(ui::render_modal_card(
            ui::ModalWidth::Narrow,
            ui::render_modal_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_3))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::TEXT_PRIMARY))
                                    .child("Open Media".to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child(
                                        "Paste a media URL. Oryx will download it into ~/Downloads and add it to Downloads with an Open button."
                                            .to_string(),
                                    ),
                            ),
                    )
                    .child(prompt_field)
                    .when_some(self.ui_state.read(cx).open_url_error(), |panel, error| {
                        panel.child(
                            div()
                                .text_size(px(theme::SMALL_SIZE))
                                .text_color(rgb(theme::ACCENT_PRIMARY))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(theme::SPACE_2))
                            .child(
                                div()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_2))
                                    .rounded(px(10.))
                                    .cursor_pointer()
                                    .bg(rgb(theme::SURFACE_BASE))
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.close_open_url_prompt(cx);
                                            },
                                        ),
                                    )
                                    .child("Cancel".to_string()),
                            )
                            .child(
                                div()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_2))
                                    .rounded(px(10.))
                                    .cursor_pointer()
                                    .bg(rgb(theme::ACCENT_PRIMARY))
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::BG_CANVAS))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.submit_open_url_prompt(cx);
                                            },
                                        ),
                                    )
                                    .child("Add To Downloads".to_string()),
                            ),
                    ),
                false,
            ),
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                this.close_open_url_prompt(cx);
            }),
        )
    }
}

fn restored_external_download_progress(
    download: &PersistedExternalDownload,
) -> ProgressiveDownload {
    let progress = ProgressiveDownload::new();
    if let Some(total_bytes) = download.total_bytes {
        progress.set_total_bytes(Some(total_bytes));
    }
    if let Some(downloaded_bytes) = download.downloaded_bytes.filter(|bytes| *bytes > 0) {
        progress.report_progress(downloaded_bytes);
    }
    if download.paused {
        progress.pause();
    }
    progress
}
