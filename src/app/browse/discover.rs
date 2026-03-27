use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Styled, Window, div, px, rgb, uniform_list,
};

use crate::platform;
use crate::theme;

use super::rows::{
    clickable_row, empty_state, sidebar_primary_metadata, sidebar_secondary_metadata,
    source_menu_row,
};
use super::{
    AppIcon, BrowseMode, CollectionKindLabel, OryxApp, SOURCE_MENU_TOP_OFFSET, SOURCE_MENU_WIDTH,
    TOPBAR_SIDE_SLOT_WIDTH, collection_entity_key, render_icon_with_color,
};
use crate::app::text_input::{TextInputElement, TextInputId};

impl OryxApp {
    pub(crate) fn render_search_box(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w_full()
            .px(px(theme::SPACE_4))
            .py(px(theme::SPACE_3))
            .flex()
            .child(
                div()
                    .w_full()
                    .h(px(42.))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .w(px(TOPBAR_SIDE_SLOT_WIDTH))
                            .flex()
                            .items_center()
                            .justify_start()
                            .child(self.render_window_controls(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .child(self.render_browse_mode_tabs(cx)),
                    )
                    .child(
                        div()
                            .w(px(TOPBAR_SIDE_SLOT_WIDTH))
                            .flex()
                            .items_center()
                            .justify_end()
                            .child(self.render_topbar_actions(cx)),
                    ),
            )
    }

    fn render_topbar_actions(&self, cx: &mut Context<Self>) -> gpui::Div {
        let app_menu_open = self.ui_state.read(cx).app_menu_open();
        let downloads_modal_open = self.ui_state.read(cx).downloads_modal_open();
        let active_download_count = self.transfer_state.read(cx).active_download_count();
        div()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .child(
                div()
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_2))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(if app_menu_open {
                        theme::ACCENT_PRIMARY
                    } else {
                        theme::BORDER_SUBTLE
                    }))
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(render_icon_with_color(
                        AppIcon::Menu,
                        16.,
                        if app_menu_open {
                            theme::ACCENT_PRIMARY
                        } else {
                            theme::TEXT_MUTED
                        },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                            this.toggle_app_menu(cx);
                        }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .px(px(theme::SPACE_2))
                    .py(px(theme::SPACE_2))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(if downloads_modal_open {
                        theme::ACCENT_PRIMARY
                    } else {
                        theme::BORDER_SUBTLE
                    }))
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                            this.toggle_downloads_modal(cx);
                        }),
                    )
                    .child(render_icon_with_color(
                        AppIcon::Download,
                        16.,
                        if downloads_modal_open {
                            theme::ACCENT_PRIMARY
                        } else {
                            theme::TEXT_MUTED
                        },
                    ))
                    .when(active_download_count > 0, |button| {
                        let label = active_download_count.min(9).to_string();
                        button.child(
                            div()
                                .absolute()
                                .top(px(1.))
                                .right(px(3.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::ACCENT_PRIMARY))
                                .child(label),
                        )
                    }),
            )
    }

    fn render_discover_search_cluster(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let query_focused = self.query_focus_handle.is_focused(window);

        div()
            .w_full()
            .h(px(52.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(if query_focused {
                theme::ACCENT_PRIMARY
            } else {
                theme::BORDER_SUBTLE
            }))
            .bg(rgb(theme::SURFACE_FLOATING))
            .px(px(theme::SPACE_3))
            .gap(px(theme::SPACE_2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(20.))
                    .child(render_icon_with_color(
                        AppIcon::Search,
                        16.,
                        theme::TEXT_MUTED,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .track_focus(&self.query_focus_handle)
                    .overflow_hidden()
                    .line_height(px(22.))
                    .text_size(px(18.))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.on_text_input_mouse_down(TextInputId::Query, event, window, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(TextInputId::Query, event, window, cx);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, window, cx| {
                            this.on_text_input_mouse_up(TextInputId::Query, event, window, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                        this.on_text_input_mouse_move(TextInputId::Query, event, window, cx);
                    }))
                    .child(TextInputElement {
                        app: cx.entity(),
                        input_id: TextInputId::Query,
                        placeholder: "Search soundtrack, artist, or album",
                        masked: false,
                    }),
            )
            .child(
                div()
                    .h_full()
                    .px(px(theme::SPACE_2))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_2))
                    .child(div().w(px(1.)).h(px(18.)).bg(rgb(theme::BORDER_SUBTLE)))
                    .child(self.render_source_picker(cx)),
            )
    }

    fn render_source_picker(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.toggle_source_picker(cx);
                }),
            )
            .child(
                div()
                    .text_size(px(theme::SMALL_SIZE))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child("Source".to_string()),
            )
            .child(
                div()
                    .text_size(px(theme::META_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(self.discover.read(cx).active_source_label(&self.providers)),
            )
    }

    fn render_discover_results_body(
        &self,
        window: &Window,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .w_full()
            .h_full()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_3))
            .child(self.render_discover_search_cluster(window, cx))
            .child(div().w_full().flex_1().min_h_0().child(body))
            .when(self.discover.read(cx).source_picker_open(), |panel| {
                panel.child(self.render_source_picker_menu(window, cx))
            })
    }

    fn render_source_picker_menu(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .absolute()
            .top(px(SOURCE_MENU_TOP_OFFSET))
            .left(px(self.source_menu_left_offset(window)))
            .w(px(SOURCE_MENU_WIDTH))
            .p(px(theme::SPACE_1))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .bg(rgb(theme::SURFACE_FLOATING))
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(
                self.searchable_provider_ids()
                    .into_iter()
                    .map(|provider_id| {
                        let is_active = self
                            .discover
                            .read(cx)
                            .enabled_provider_ids(&self.providers)
                            .contains(&provider_id);
                        let provider_name = self
                            .provider_for_id(provider_id)
                            .map(|provider| provider.display_name())
                            .unwrap_or(provider_id.display_name());
                        let requires_login = self
                            .provider_for_id(provider_id)
                            .map(|provider| {
                                provider.requires_credentials()
                                    && !provider.has_stored_credentials()
                            })
                            .unwrap_or(false);

                        source_menu_row(
                            provider_name,
                            is_active,
                            requires_login,
                            cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                                this.toggle_provider_from_menu(provider_id, window, cx);
                            }),
                        )
                    }),
            )
    }

    fn render_browse_mode_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .child(self.render_browse_mode_tab(BrowseMode::Discover, "Discover", cx))
            .child(self.render_tab_separator())
            .child(self.render_browse_mode_tab(BrowseMode::Artists, "Artists", cx))
            .child(self.render_tab_separator())
            .child(self.render_browse_mode_tab(BrowseMode::Albums, "Albums", cx))
            .child(self.render_tab_separator())
            .child(self.render_browse_mode_tab(BrowseMode::Playlists, "Playlists", cx))
    }

    fn render_browse_mode_tab(
        &self,
        mode: BrowseMode,
        label: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let active = self.browse_mode == mode;
        div()
            .px(px(2.))
            .py(px(2.))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                    this.set_browse_mode(mode, cx);
                }),
            )
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if active {
                        theme::ACCENT_PRIMARY
                    } else {
                        theme::TEXT_PRIMARY
                    }))
                    .child(label.to_string()),
            )
    }

    fn render_tab_separator(&self) -> gpui::Div {
        div()
            .text_size(px(15.))
            .text_color(rgb(theme::TEXT_DIM))
            .child("|")
    }

    fn render_window_controls(&self, cx: &mut Context<Self>) -> gpui::Div {
        if platform::uses_native_window_controls() {
            return div().w_full();
        }

        div()
            .flex()
            .items_center()
            .justify_start()
            .gap(px(theme::SPACE_3))
            .child(
                div()
                    .px(px(theme::SPACE_1))
                    .py(px(2.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                            this.quit_app(cx);
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::ACCENT_PRIMARY))
                            .child("x"),
                    ),
            )
            .child(
                div()
                    .px(px(theme::SPACE_1))
                    .py(px(2.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _event: &MouseDownEvent, window, _cx| {
                            platform::minimize_window(window);
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child("-"),
                    ),
            )
    }

    pub(super) fn render_results_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if self.discover.read(cx).search_loading() {
            let loading_message = format!(
                "Searching {}...",
                self.discover.read(cx).active_source_label(&self.providers)
            );
            return self
                .render_discover_results_body(window, empty_state(&loading_message), cx)
                .w(px(self.discovery_column_width(window)))
                .bg(rgb(theme::SURFACE_BASE))
                .px(px(theme::SPACE_3))
                .py(px(theme::SPACE_3));
        }

        let search_results = self.discover.read(cx).search_results();
        if search_results.is_empty() {
            return self
                .render_discover_results_body(window, empty_state("No results yet."), cx)
                .w(px(self.discovery_column_width(window)))
                .bg(rgb(theme::SURFACE_BASE))
                .px(px(theme::SPACE_3))
                .py(px(theme::SPACE_3));
        }

        let item_count = search_results.len();
        let body = uniform_list(
            "results-list",
            item_count,
            cx.processor(
                move |this: &mut OryxApp, range: Range<usize>, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let collection = this.discover.read(cx).search_results()[index].clone();
                        let collection_key = collection_entity_key(&collection.reference);
                        let active = this.discover.read(cx).selected_collection_id().as_deref()
                            == Some(collection_key.as_str());
                        let title = collection.title.clone();
                        let kind_label = collection.reference.kind.label();
                        let primary_metadata =
                            sidebar_primary_metadata(collection.subtitle.as_deref(), kind_label);
                        let secondary_metadata = sidebar_secondary_metadata(
                            kind_label,
                            collection.track_count,
                            &primary_metadata,
                        );
                        let metadata = this.collection_metadata(&collection, cx);
                        let reference = collection.reference.clone();
                        let artwork_url = collection.artwork_url.clone();
                        items.push(
                            clickable_row(
                                &title,
                                &primary_metadata,
                                secondary_metadata.as_deref(),
                                metadata,
                                super::rows::render_collection_artwork(artwork_url, 62.),
                                active,
                            )
                            .id(("discover", index))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                                    this.load_collection(reference.clone(), cx);
                                }),
                            ),
                        );
                    }
                    items
                },
            ),
        )
        .size_full();

        self.render_discover_results_body(window, body, cx)
            .w(px(self.discovery_column_width(window)))
            .bg(rgb(theme::SURFACE_BASE))
            .px(px(theme::SPACE_3))
            .py(px(theme::SPACE_3))
    }
}
