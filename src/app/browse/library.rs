use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    Context, FontWeight, MouseButton, MouseDownEvent, ParentElement, Styled, Window, div, px, rgb,
    uniform_list,
};

use crate::provider::{ProviderId, TrackList, TrackSummary};
use crate::theme;
use crate::transfer::DownloadPurpose;

use super::super::library::{summarize_collection_quality, summarize_track_list_quality};
use super::super::track_cache_key;
use super::super::ui::ContextMenuTarget;
use super::rows::{
    apply_previous_playing_row_style, artist_album_metadata, clickable_row, empty_state,
    panel_body, render_collection_artwork, render_download_progress_line, render_row_metadata,
    render_track_download_action, render_track_like_action, render_track_list_artwork, row_shell,
    sidebar_primary_metadata, sidebar_secondary_metadata,
};
use super::{
    AppIcon, BrowseMode, CollectionKindLabel, OryxApp, format_duration,
    local_collection_selection_key, provider_collection_ref_for_local_album,
    render_icon_with_color,
};

#[derive(Clone)]
enum ArtistTrackRow {
    AlbumHeader {
        provider: ProviderId,
        collection_id: String,
        album: String,
        artwork_url: Option<String>,
        track_count: usize,
    },
    Track {
        index: usize,
        track: TrackSummary,
    },
}

impl OryxApp {
    pub(super) fn render_local_albums_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.render_local_collection_panel(BrowseMode::Albums, window, cx)
    }

    pub(super) fn render_local_artists_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.render_local_collection_panel(BrowseMode::Artists, window, cx)
    }

    pub(super) fn render_local_playlists_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.render_local_collection_panel(BrowseMode::Playlists, window, cx)
    }

    fn render_local_collection_panel(
        &self,
        mode: BrowseMode,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let collections = self.local_collections(mode, cx);
        if collections.is_empty() {
            return panel_body(empty_state("No local items available."))
                .w(px(self.discovery_column_width(window)));
        }

        let item_count = collections.len();
        let list_id = match mode {
            BrowseMode::Albums => "albums-list",
            BrowseMode::Artists => "artists-list",
            BrowseMode::Playlists => "playlists-list",
            BrowseMode::Discover => "discover-list",
        };

        let body = uniform_list(
            list_id,
            item_count,
            cx.processor(
                move |this: &mut OryxApp, range: Range<usize>, _window, cx| {
                    let collections = this.local_collections(mode, cx);
                    let active_id = this.selected_local_collection_id(mode, cx);

                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let track_list = collections[index].clone();
                        let kind_label = match mode {
                            BrowseMode::Albums => "Album",
                            BrowseMode::Artists => "Artist",
                            BrowseMode::Playlists => "Playlist",
                            BrowseMode::Discover => "",
                        };
                        let primary_metadata = sidebar_primary_metadata(
                            track_list.collection.subtitle.as_deref(),
                            kind_label,
                        );
                        let secondary_metadata = sidebar_secondary_metadata(
                            (mode != BrowseMode::Artists).then_some(kind_label),
                            Some(track_list.tracks.len()),
                            &primary_metadata,
                        );
                        let metadata = this.track_list_metadata(&track_list, cx);

                        let track_list_for_left_click = track_list.clone();
                        let track_list_for_right_click = track_list.clone();
                        let is_active = active_id.as_deref()
                            == Some(
                                local_collection_selection_key(
                                    mode,
                                    &track_list.collection.reference,
                                )
                                .as_str(),
                            );
                        let row = clickable_row(
                            &track_list.collection.title,
                            &primary_metadata,
                            secondary_metadata.as_deref(),
                            metadata,
                            render_track_list_artwork(&track_list, 62.),
                            is_active,
                        )
                        .id((
                            match mode {
                                BrowseMode::Albums => "albums",
                                BrowseMode::Artists => "artists",
                                BrowseMode::Playlists => "playlists",
                                BrowseMode::Discover => "discover",
                            },
                            index,
                        ))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                                this.close_context_menu(cx);
                                match mode {
                                    BrowseMode::Albums => {
                                        this.select_local_collection(
                                            BrowseMode::Albums,
                                            track_list_for_left_click
                                                .collection
                                                .reference
                                                .id
                                                .clone(),
                                            cx,
                                        );
                                        this.status_message = Some(format!(
                                            "{} track(s) in '{}'.",
                                            track_list_for_left_click.tracks.len(),
                                            track_list_for_left_click.collection.title
                                        ));
                                    }
                                    BrowseMode::Artists => {
                                        this.select_local_collection(
                                            BrowseMode::Artists,
                                            track_list_for_left_click
                                                .collection
                                                .reference
                                                .id
                                                .clone(),
                                            cx,
                                        );
                                        this.status_message = Some(format!(
                                            "{} track(s) by '{}'.",
                                            track_list_for_left_click.tracks.len(),
                                            track_list_for_left_click.collection.title
                                        ));
                                    }
                                    BrowseMode::Playlists => {
                                        this.select_local_collection(
                                            BrowseMode::Playlists,
                                            track_list_for_left_click
                                                .collection
                                                .reference
                                                .id
                                                .clone(),
                                            cx,
                                        );
                                        this.status_message = Some(format!(
                                            "{} track(s) in playlist '{}'.",
                                            track_list_for_left_click.tracks.len(),
                                            track_list_for_left_click.collection.title
                                        ));
                                    }
                                    BrowseMode::Discover => {}
                                }
                                cx.notify();
                            }),
                        );

                        let row = if mode == BrowseMode::Albums {
                            row.on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    let Some(provider_collection) =
                                        provider_collection_ref_for_local_album(
                                            &track_list_for_right_click,
                                        )
                                    else {
                                        this.close_context_menu(cx);
                                        return;
                                    };
                                    this.open_context_menu(
                                        event.position,
                                        ContextMenuTarget::LocalAlbum {
                                            provider: provider_collection.provider,
                                            collection_id: provider_collection.id,
                                            title: track_list_for_right_click
                                                .collection
                                                .title
                                                .clone(),
                                        },
                                        cx,
                                    );
                                }),
                            )
                        } else {
                            row
                        };

                        items.push(row);
                    }

                    items
                },
            ),
        )
        .size_full();

        panel_body(body).w(px(self.discovery_column_width(window)))
    }

    pub(super) fn render_tracks_panel(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        if matches!(self.browse_mode, BrowseMode::Discover)
            && self.discover.read(cx).track_list_loading()
        {
            return panel_body(empty_state("Loading track list...")).flex_1();
        }

        let Some(track_list) = self.current_visible_track_list(cx) else {
            return panel_body(empty_state("No track list selected.")).flex_1();
        };

        if self.browse_mode == BrowseMode::Artists {
            return self.render_artist_tracks_panel(&track_list, window, cx);
        }

        let collection_meta = format!(
            "{}  •  {} track(s)",
            track_list.collection.reference.kind.label(),
            track_list
                .collection
                .track_count
                .unwrap_or(track_list.tracks.len())
        );
        let show_metadata = !self.should_show_compact_metadata(window);
        let show_playlist_track_artwork = self.browse_mode == BrowseMode::Playlists;
        let collection_metadata = if show_metadata {
            self.track_list_metadata(&track_list, cx)
        } else {
            None
        };
        let collection_quality = self
            .library_catalog
            .read(cx)
            .collection_quality(&track_list.collection.reference)
            .or_else(|| summarize_track_list_quality(&track_list));
        let item_count = track_list.tracks.len();
        let track_rows = uniform_list(
            "tracks-list",
            item_count,
            cx.processor(
                move |this: &mut OryxApp, range: Range<usize>, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    let Some(track_list) = this.current_visible_track_list(cx) else {
                        return items;
                    };
                    let playback_state = this.playback_state.read(cx);
                    let pending_track_key = playback_state
                        .pending_play_request()
                        .as_ref()
                        .and_then(|request| request.playback_context.tracks.get(request.index))
                        .map(track_cache_key);
                    let current_playing_track_key = playback_state
                        .playback_context()
                        .as_ref()
                        .and_then(|context| {
                            playback_state
                                .current_track_index()
                                .and_then(|index| context.tracks.get(index))
                        })
                        .map(track_cache_key);

                    for index in range {
                        let track = track_list.tracks[index].clone();
                        let track_for_context_menu = track.clone();
                        let track_key = track_cache_key(&track);
                        let is_pending =
                            pending_track_key.as_deref() == Some(track_key.as_str());
                        let is_current_playing =
                            current_playing_track_key.as_deref() == Some(track_key.as_str());
                        let is_previous_playing =
                            pending_track_key.is_some() && is_current_playing && !is_pending;
                        let active = pending_track_key
                            .as_deref()
                            .or(current_playing_track_key.as_deref())
                            == Some(track_key.as_str());
                        let is_cached = this.track_is_cached(&track, cx);
                        let is_liked = this.track_is_liked(&track, cx);
                        let active_download = this.active_download(&track, cx);
                        let explicit_download_active = matches!(
                            active_download.as_ref().map(|download| download.purpose),
                            Some(DownloadPurpose::Explicit)
                        );
                        let show_download_action = !is_cached
                            && !matches!(
                                active_download.as_ref().map(|download| download.purpose),
                                Some(DownloadPurpose::PlaybackPrefetch)
                            );
                        let download_progress = active_download
                            .as_ref()
                            .map(|download| download.progress.snapshot());
                        let subtitle = if show_playlist_track_artwork {
                            playlist_track_subtitle(&track)
                        } else {
                            format!(
                                "{}  •  {}",
                                track.artist.as_deref().unwrap_or("Unknown artist"),
                                format_duration(track.duration_seconds)
                            )
                        };
                        let metadata = this.track_metadata_for_collection(
                            &track,
                            track_list.collection.reference.provider,
                            collection_quality.as_ref(),
                            cx,
                        );
                        let row = row_shell(active, 82., 8.)
                            .when(is_previous_playing, apply_previous_playing_row_style)
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(
                                    move |this, _event: &MouseDownEvent, _window, cx| {
                                        this.play_track_at(index, cx);
                                    },
                                ),
                            )
                            .flex()
                            .flex_col()
                            .when_some(download_progress, |row, snapshot| {
                                row.child(render_download_progress_line(snapshot))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .px(px(theme::SPACE_3))
                                    .py(px(theme::SPACE_3))
                                    .flex()
                                    .items_center()
                                    .gap(px(theme::SPACE_3))
                                    .when(show_playlist_track_artwork, |row| {
                                        row.child(render_collection_artwork(
                                            track.artwork_url.clone(),
                                            48.,
                                        ))
                                    })
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
                                    .when_some(
                                        if show_metadata {
                                            metadata.as_ref()
                                        } else {
                                            None
                                        },
                                        |row, metadata| {
                                        row.child(render_row_metadata(metadata))
                                        },
                                    )
                                    .child(
                                        render_track_like_action(is_liked).on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this,
                                                      _event: &MouseDownEvent,
                                                      _window,
                                                      cx| {
                                                    cx.stop_propagation();
                                                    this.toggle_track_like(track.clone(), cx);
                                                },
                                            ),
                                        ),
                                    )
                                    .when(show_download_action, |row| {
                                        row.child(
                                            render_track_download_action(explicit_download_active)
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    move |this,
                                                          _event: &MouseDownEvent,
                                                          _window,
                                                          cx| {
                                                        cx.stop_propagation();
                                                        if explicit_download_active {
                                                            this.cancel_download_track_at(index, cx);
                                                        } else {
                                                            this.download_track_at(index, cx);
                                                        }
                                                    },
                                                ),
                                            ),
                                        )
                                    }),
                            )
                            .id(("track", index));
                        let row = if is_cached {
                            row.on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.open_context_menu(
                                        event.position,
                                        ContextMenuTarget::LocalTrack {
                                            provider: track_for_context_menu.reference.provider,
                                            track_id: track_for_context_menu.reference.id.clone(),
                                            title: track_for_context_menu.title.clone(),
                                        },
                                        cx,
                                    );
                                }),
                            )
                        } else {
                            row
                        };
                        items.push(row);
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
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_3))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_3))
                    .child(render_track_list_artwork(&track_list, 108.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .text_color(rgb(theme::ACCENT_PRIMARY))
                                    .truncate()
                                    .child(collection_meta),
                            )
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_clamp(2)
                                    .child(track_list.collection.title.clone()),
                            )
                            .when_some(
                                track_list.collection.subtitle.clone(),
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
                    .when_some(collection_metadata.as_ref(), |section, metadata| {
                        section.child(render_row_metadata(metadata))
                    }),
            )
            .child(div().w_full().flex_1().min_h_0().child(track_rows));

        panel_body(body).flex_1()
    }

    fn render_artist_tracks_panel(
        &self,
        track_list: &TrackList,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let show_metadata = !self.should_show_compact_metadata(window);
        let collection_metadata = if show_metadata {
            self.track_list_metadata(track_list, cx)
        } else {
            None
        };
        let collection_meta = format!(
            "Artist  •  {} track(s)",
            track_list
                .collection
                .track_count
                .unwrap_or(track_list.tracks.len())
        );
        let item_count = artist_group_rows(track_list).len();
        let track_rows = uniform_list(
            "artist-tracks-list",
            item_count,
            cx.processor(move |this: &mut OryxApp, range: Range<usize>, window, cx| {
                let mut items = Vec::with_capacity(range.len());
                let Some(track_list) = this.current_visible_track_list(cx) else {
                    return items;
                };
                let show_metadata = !this.should_show_compact_metadata(window);
                let grouped_rows = artist_group_rows(&track_list);
                let playback_state = this.playback_state.read(cx);
                let pending_track_key = playback_state
                    .pending_play_request()
                    .as_ref()
                    .and_then(|request| request.playback_context.tracks.get(request.index))
                    .map(track_cache_key);
                let current_playing_track_key = playback_state
                    .playback_context()
                    .as_ref()
                    .and_then(|context| {
                        playback_state
                            .current_track_index()
                            .and_then(|index| context.tracks.get(index))
                    })
                    .map(track_cache_key);

                for index in range {
                    match grouped_rows[index].clone() {
                        ArtistTrackRow::AlbumHeader {
                            provider,
                            collection_id,
                            album,
                            artwork_url,
                            track_count,
                        } => {
                            let artist = track_list.collection.title.clone();
                            let album_id = collection_id.clone();
                            let status_artist = artist.clone();
                            let status_album = album.clone();
                            items.push(
                                row_shell(false, 82., 8.)
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, _event: &MouseDownEvent, _window, cx| {
                                                this.set_browse_mode(BrowseMode::Albums, cx);
                                                this.select_local_collection(
                                                    BrowseMode::Albums,
                                                    album_id.clone(),
                                                    cx,
                                                );
                                                this.status_message = Some(format!(
                                                    "Showing '{}' from '{}'.",
                                                    status_album, status_artist
                                                ));
                                                cx.notify();
                                            },
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .px(px(theme::SPACE_3))
                                            .py(px(theme::SPACE_3))
                                            .flex()
                                            .items_center()
                                            .gap(px(theme::SPACE_3))
                                            .child(render_collection_artwork(artwork_url, 48.))
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
                                                            .child(album),
                                                    )
                                                    .child(
                                                        div()
                                                            .h(px(18.))
                                                            .truncate()
                                                            .text_size(px(theme::META_SIZE))
                                                            .text_color(rgb(theme::TEXT_MUTED))
                                                            .child(format!(
                                                                "Album  •  {track_count} track(s)  •  Open album"
                                                            )),
                                                    ),
                                            )
                                            .when_some(
                                                show_metadata
                                                    .then(|| {
                                                        artist_album_metadata(
                                                            provider,
                                                            &track_list.tracks,
                                                            &collection_id,
                                                        )
                                                    })
                                                    .flatten()
                                                    .as_ref(),
                                                |row, metadata| {
                                                    row.child(render_row_metadata(metadata))
                                                },
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .child(render_icon_with_color(
                                                        AppIcon::Play,
                                                        14.,
                                                        theme::TEXT_DIM,
                                                    )),
                                            ),
                                    )
                                    .id(("artist-album-header", index)),
                            );
                        }
                        ArtistTrackRow::Track { index, track } => {
                            let track_index = index;
                            let track_for_context_menu = track.clone();
                            let track_key = track_cache_key(&track);
                            let is_pending =
                                pending_track_key.as_deref() == Some(track_key.as_str());
                            let is_current_playing =
                                current_playing_track_key.as_deref() == Some(track_key.as_str());
                            let is_previous_playing =
                                pending_track_key.is_some() && is_current_playing && !is_pending;
                            let active = pending_track_key
                                .as_deref()
                                .or(current_playing_track_key.as_deref())
                                == Some(track_key.as_str());
                            let is_cached = this.track_is_cached(&track, cx);
                            let is_liked = this.track_is_liked(&track, cx);
                            let active_download = this.active_download(&track, cx);
                            let explicit_download_active = matches!(
                                active_download.as_ref().map(|download| download.purpose),
                                Some(DownloadPurpose::Explicit)
                            );
                            let show_download_action = !is_cached
                                && !matches!(
                                    active_download.as_ref().map(|download| download.purpose),
                                    Some(DownloadPurpose::PlaybackPrefetch)
                                );
                            let download_progress = active_download
                                .as_ref()
                                .map(|download| download.progress.snapshot());
                            let subtitle = format_duration(track.duration_seconds);
                            let collection_quality = summarize_collection_quality(
                                track_list
                                    .tracks
                                    .iter()
                                    .filter(|candidate| {
                                        candidate.collection_id.as_deref() == track.collection_id.as_deref()
                                    }),
                            );
                            let collection_provider = track.reference.provider;
                            let metadata = this.track_metadata_for_collection(
                                &track,
                                collection_provider,
                                collection_quality.as_ref(),
                                cx,
                            );
                            let row = row_shell(active, 82., 8.)
                                .when(is_previous_playing, apply_previous_playing_row_style)
                                .cursor_pointer()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, _event: &MouseDownEvent, _window, cx| {
                                            this.play_track_at(track_index, cx);
                                        },
                                    ),
                                )
                                .flex()
                                .flex_col()
                                .when_some(download_progress, |row, snapshot| {
                                    row.child(render_download_progress_line(snapshot))
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .px(px(theme::SPACE_4))
                                        .py(px(theme::SPACE_3))
                                        .flex()
                                        .items_center()
                                        .gap(px(theme::SPACE_3))
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
                                        .when_some(
                                            if show_metadata {
                                                metadata.as_ref()
                                            } else {
                                                None
                                            },
                                            |row, metadata| {
                                                row.child(render_row_metadata(metadata))
                                            },
                                        )
                                        .child(
                                            render_track_like_action(is_liked).on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    move |this,
                                                          _event: &MouseDownEvent,
                                                          _window,
                                                          cx| {
                                                        cx.stop_propagation();
                                                        this.toggle_track_like(track.clone(), cx);
                                                    },
                                                ),
                                            ),
                                        )
                                        .when(show_download_action, |row| {
                                            row.child(
                                                render_track_download_action(explicit_download_active)
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(
                                                        move |this,
                                                              _event: &MouseDownEvent,
                                                              _window,
                                                              cx| {
                                                            cx.stop_propagation();
                                                            if explicit_download_active {
                                                                this.cancel_download_track_at(track_index, cx);
                                                            } else {
                                                                this.download_track_at(track_index, cx);
                                                            }
                                                        },
                                                    ),
                                                ),
                                            )
                                        }),
                                )
                                .id(("artist-track", track_index));
                            let row = if is_cached {
                                row.on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.open_context_menu(
                                            event.position,
                                            ContextMenuTarget::LocalTrack {
                                                provider: track_for_context_menu.reference.provider,
                                                track_id: track_for_context_menu.reference.id.clone(),
                                                title: track_for_context_menu.title.clone(),
                                            },
                                            cx,
                                        );
                                    }),
                                )
                            } else {
                                row
                            };
                            items.push(row);
                        }
                    }
                }

                items
            }),
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
                    .bg(rgb(theme::SURFACE_FLOATING))
                    .px(px(theme::SPACE_3))
                    .py(px(theme::SPACE_3))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_3))
                    .child(render_track_list_artwork(track_list, 108.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(theme::SPACE_1))
                            .child(
                                div()
                                    .text_size(px(theme::SMALL_SIZE))
                                    .text_color(rgb(theme::ACCENT_PRIMARY))
                                    .truncate()
                                    .child(collection_meta),
                            )
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_clamp(2)
                                    .child(track_list.collection.title.clone()),
                            )
                            .when_some(
                                track_list.collection.subtitle.clone(),
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
                    .when_some(collection_metadata.as_ref(), |section, metadata| {
                        section.child(render_row_metadata(metadata))
                    }),
            )
            .child(div().w_full().flex_1().min_h_0().child(track_rows));

        panel_body(body).flex_1()
    }
}

fn artist_group_rows(track_list: &TrackList) -> Vec<ArtistTrackRow> {
    let mut rows = Vec::new();
    let mut current_album_key: Option<(ProviderId, String)> = None;
    let mut current_album_title: Option<String> = None;
    let mut current_album_artwork: Option<String> = None;
    let mut current_album_count = 0usize;
    let mut current_album_tracks = Vec::new();

    for (index, track) in track_list.tracks.iter().cloned().enumerate() {
        let collection_id = track
            .collection_id
            .clone()
            .unwrap_or_else(|| track.reference.id.clone());
        let album = track
            .collection_title
            .clone()
            .or_else(|| track.album.clone())
            .unwrap_or_else(|| "Unknown album".to_string());
        let album_key = (track.reference.provider, collection_id.clone());

        if current_album_key.as_ref() != Some(&album_key) {
            if let (Some((provider, collection_id)), Some(previous_album)) = (
                current_album_key.replace(album_key),
                current_album_title.replace(album.clone()),
            ) {
                rows.push(ArtistTrackRow::AlbumHeader {
                    provider,
                    collection_id,
                    album: previous_album,
                    artwork_url: current_album_artwork.take(),
                    track_count: current_album_count,
                });
                rows.append(&mut current_album_tracks);
            }
            current_album_artwork = track.artwork_url.clone();
            current_album_count = 0;
        }

        current_album_count += 1;
        current_album_tracks.push(ArtistTrackRow::Track { index, track });
    }

    if let (Some((provider, collection_id)), Some(last_album)) =
        (current_album_key, current_album_title)
    {
        rows.push(ArtistTrackRow::AlbumHeader {
            provider,
            collection_id,
            album: last_album,
            artwork_url: current_album_artwork,
            track_count: current_album_count,
        });
        rows.append(&mut current_album_tracks);
    }

    rows
}

fn playlist_track_subtitle(track: &TrackSummary) -> String {
    let mut parts = Vec::new();
    parts.push(
        track
            .artist
            .as_deref()
            .unwrap_or("Unknown artist")
            .to_string(),
    );
    if let Some(album) = track
        .album
        .as_deref()
        .filter(|album| !album.trim().is_empty())
    {
        parts.push(album.to_string());
    }
    parts.push(format_duration(track.duration_seconds));
    parts.join("  •  ")
}
