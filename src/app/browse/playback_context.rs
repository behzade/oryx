use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Context, FontWeight, MouseButton, MouseDownEvent, ParentElement, Styled, Window, div, px,
    rgb, uniform_list,
};

use crate::theme;

use super::super::track_cache_key;
use super::rows::{
    empty_state, panel_body, render_collection_artwork, render_row_metadata,
    summarize_track_list_quality,
};
use super::{CollectionKindLabel, OryxApp, format_duration};

impl OryxApp {
    pub(super) fn render_playback_context_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let Some(playback_context) = self.playback_state.read(cx).playback_context() else {
            return panel_body(empty_state("")).w(px(self.playback_context_width(window)));
        };

        let item_count = playback_context.tracks.len();
        let collection_reference = playback_context.collection.reference.clone();
        let playback_meta = format!(
            "{}  •  {} track(s)",
            playback_context.collection.reference.kind.label(),
            playback_context
                .collection
                .track_count
                .unwrap_or(playback_context.tracks.len())
        );
        let playback_collection_metadata = self.track_list_metadata(&playback_context, cx);
        let playback_collection_quality = self
            .library_catalog
            .read(cx)
            .collection_quality(&collection_reference)
            .or_else(|| summarize_track_list_quality(&playback_context));
        let track_rows = uniform_list(
            "playback-context-list",
            item_count,
            cx.processor(
                move |this: &mut OryxApp, range: Range<usize>, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    let Some(playback_context) = this.playback_state.read(cx).playback_context()
                    else {
                        return items;
                    };

                    for index in range {
                        let track = playback_context.tracks[index].clone();
                        let active =
                            this.playback_state.read(cx).current_track_index() == Some(index);
                        let subtitle = format!(
                            "{}  •  {}",
                            track.artist.as_deref().unwrap_or("Unknown artist"),
                            format_duration(track.duration_seconds)
                        );
                        let metadata = this.track_metadata_for_collection(
                            &track,
                            playback_context.collection.reference.provider,
                            playback_collection_quality.as_ref(),
                            cx,
                        );
                        items.push(
                            div()
                                .w_full()
                                .h(px(82.))
                                .overflow_hidden()
                                .rounded(px(10.))
                                .bg(rgb(if active {
                                    theme::ACCENT_PRIMARY_LIGHT
                                } else {
                                    theme::SURFACE_FLOATING
                                }))
                                .px(px(theme::SPACE_3))
                                .py(px(theme::SPACE_3))
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .gap(px(theme::SPACE_3))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, _event: &MouseDownEvent, _window, cx| {
                                            if let Some(playback_context) =
                                                this.playback_state.read(cx).playback_context()
                                            {
                                                this.start_playback_for_context(
                                                    playback_context,
                                                    index,
                                                    None,
                                                    cx,
                                                );
                                            }
                                        },
                                    ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .flex()
                                        .flex_col()
                                        .gap(px(theme::SPACE_1))
                                        .child(
                                            div()
                                                .h(px(20.))
                                                .truncate()
                                                .text_size(px(theme::BODY_SIZE))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(track.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .h(px(18.))
                                                .truncate()
                                                .text_size(px(theme::META_SIZE))
                                                .text_color(rgb(theme::TEXT_MUTED))
                                                .child(subtitle),
                                        ),
                                )
                                .when_some(metadata.as_ref(), |row, metadata| {
                                    row.child(render_row_metadata(metadata))
                                })
                                .id(("playback-track", index)),
                        );
                    }

                    items
                },
            ),
        )
        .size_full();
        let body = div()
            .w_full()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_3))
            .child(
                div()
                    .w_full()
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_3))
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_3))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                            this.load_collection(collection_reference.clone(), cx);
                        }),
                    )
                    .child(render_collection_artwork(
                        playback_context.collection.artwork_url.clone(),
                        220.,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .text_color(rgb(theme::ACCENT_PRIMARY))
                                    .truncate()
                                    .child(playback_meta),
                            )
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_clamp(2)
                                    .child(playback_context.collection.title.clone()),
                            )
                            .when_some(
                                playback_context.collection.subtitle.clone(),
                                |section, subtitle| {
                                    section.child(
                                        div()
                                            .text_size(px(theme::META_SIZE))
                                            .text_color(rgb(theme::TEXT_MUTED))
                                            .truncate()
                                            .child(subtitle),
                                    )
                                },
                            ),
                    )
                    .when_some(
                        playback_collection_metadata.as_ref(),
                        |section, metadata| section.child(render_row_metadata(metadata)),
                    ),
            )
            .child(div().w_full().flex_1().min_h_0().child(track_rows));

        panel_body(body).w(px(self.playback_context_width(window)))
    }

    pub(super) fn should_show_playback_context_panel(&self, cx: &App) -> bool {
        let Some(playback_context) = self.playback_state.read(cx).playback_context() else {
            return false;
        };
        let Some(current_track_key) = self
            .playback_state
            .read(cx)
            .current_track_index()
            .and_then(|index| playback_context.tracks.get(index))
            .map(track_cache_key)
        else {
            return true;
        };

        match self.current_visible_track_list(cx) {
            Some(track_list) => !track_list
                .tracks
                .iter()
                .any(|track| track_cache_key(track) == current_track_key),
            None => true,
        }
    }
}
