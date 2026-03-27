use std::collections::HashSet;
use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    App, IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Styled, Window, div,
    img, px, relative, rgb, rgba,
};

use crate::app::library::{AudioQualityGrade, summarize_collection_quality};
use crate::progressive::ProgressiveSnapshot;
use crate::provider::{CollectionKind, ProviderId, TrackList, TrackSummary};
use crate::theme;

use super::{AppIcon, CollectionQualitySummary, render_icon_with_color};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QualityMetadata {
    pub(super) label: String,
    pub(super) color: u32,
}

#[derive(Clone, Debug)]
pub(super) struct RowMetadata {
    pub(super) provider_label: Option<String>,
    pub(super) quality: Option<QualityMetadata>,
}

pub(super) fn sidebar_primary_metadata(subtitle: Option<&str>, kind_label: &str) -> String {
    subtitle
        .map(str::trim)
        .filter(|subtitle| !subtitle.is_empty() && *subtitle != kind_label)
        .map(str::to_string)
        .unwrap_or_else(|| kind_label.to_string())
}

pub(super) fn sidebar_secondary_metadata(
    kind_label: Option<&str>,
    track_count: Option<usize>,
    primary_metadata: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(kind_label) = kind_label
        && primary_metadata != kind_label
    {
        parts.push(kind_label.to_string());
    }
    if let Some(track_count) = track_count {
        parts.push(format!("{track_count} tracks"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("  •  "))
    }
}

pub(super) fn section_divider() -> gpui::Div {
    div().w_full().h(px(1.)).bg(rgb(theme::BORDER_SUBTLE))
}

pub(super) fn vertical_divider() -> gpui::Div {
    div().w(px(1.)).h_full().bg(rgb(theme::BORDER_SUBTLE))
}

pub(super) fn download_progress_ratio(snapshot: ProgressiveSnapshot) -> Option<f32> {
    if snapshot.complete {
        return Some(1.0);
    }

    snapshot
        .total_bytes
        .filter(|total| *total > 0)
        .map(|total| (snapshot.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0))
}

pub(super) fn render_download_progress_line(snapshot: ProgressiveSnapshot) -> gpui::Div {
    let ratio = download_progress_ratio(snapshot).unwrap_or(0.0);

    div()
        .absolute()
        .bottom(px(theme::SPACE_2))
        .left(px(theme::SPACE_3))
        .right(px(theme::SPACE_3))
        .h(px(2.))
        .rounded(px(999.))
        .overflow_hidden()
        .bg(rgb(theme::DOWNLOAD_PROGRESS_LIGHT))
        .child(
            div()
                .w(relative(ratio.clamp(0.0, 1.0)))
                .h_full()
                .rounded(px(999.))
                .bg(rgb(theme::DOWNLOAD_PROGRESS)),
        )
}

pub(super) fn render_track_download_action(is_cancel: bool) -> gpui::Div {
    let icon_color = if is_cancel {
        0xFFF06A59
    } else {
        theme::TEXT_DIM
    };

    div()
        .flex_shrink_0()
        .px(px(theme::SPACE_2))
        .py(px(theme::SPACE_2))
        .cursor_pointer()
        .child(
            div().w(px(16.)).h(px(16.)).overflow_hidden().child(
                gpui::svg()
                    .path(if is_cancel {
                        AppIcon::X.asset_path()
                    } else {
                        AppIcon::Download.asset_path()
                    })
                    .w_full()
                    .h_full()
                    .text_color(rgb(icon_color)),
            ),
        )
}

pub(super) fn render_track_like_action(is_liked: bool) -> gpui::Div {
    let icon_color = if is_liked {
        theme::ACCENT_PRIMARY
    } else {
        theme::TEXT_DIM
    };

    div()
        .flex_shrink_0()
        .px(px(theme::SPACE_2))
        .py(px(theme::SPACE_2))
        .cursor_pointer()
        .child(
            div().w(px(16.)).h(px(16.)).overflow_hidden().child(
                gpui::svg()
                    .path(
                        if is_liked {
                            AppIcon::HeartFilled
                        } else {
                            AppIcon::Heart
                        }
                        .asset_path(),
                    )
                    .w_full()
                    .h_full()
                    .text_color(rgb(icon_color)),
            ),
        )
}

pub(super) fn apply_previous_playing_row_style(row: gpui::Div) -> gpui::Div {
    row.border_color(rgb(theme::ROW_TRANSITION_BORDER))
        .bg(rgb(theme::ROW_TRANSITION_BG))
}

pub(super) fn panel_body(body: impl IntoElement) -> gpui::Div {
    div()
        .h_full()
        .min_h_0()
        .overflow_hidden()
        .bg(rgb(theme::SURFACE_BASE))
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_3))
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_3))
        .child(div().w_full().flex_1().min_h_0().child(body))
}

pub(super) fn row_shell(active: bool, height: f32, radius: f32) -> gpui::Div {
    div()
        .w_full()
        .h(px(height))
        .relative()
        .overflow_hidden()
        .rounded(px(radius))
        .border_1()
        .border_color(rgb(if active {
            theme::ROW_ACTIVE_BORDER
        } else {
            theme::ROW_IDLE_BORDER
        }))
        .bg(if active {
            rgba(theme::ROW_ACTIVE_BG)
        } else {
            rgb(theme::ROW_IDLE_BG)
        })
        .hover(move |style| {
            if active {
                style
            } else {
                style
                    .bg(rgb(theme::ROW_HOVER_BG))
                    .border_color(rgb(theme::ROW_HOVER_BORDER))
            }
        })
        .when(active, |row| row.border_l_4())
}

pub(super) fn clickable_row(
    title: &str,
    primary_metadata: &str,
    secondary_metadata: Option<&str>,
    metadata: Option<RowMetadata>,
    artwork: impl IntoElement,
    active: bool,
) -> gpui::Div {
    row_shell(active, 86., 8.)
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_3))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_3))
        .child(artwork)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .h(px(20.))
                        .truncate()
                        .text_size(px(theme::BODY_SIZE))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .h(px(18.))
                        .truncate()
                        .text_size(px(theme::META_SIZE))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(primary_metadata.to_string()),
                )
                .when(
                    secondary_metadata.is_some() || metadata.is_some(),
                    |column| {
                        column.child(render_clickable_row_secondary_metadata(
                            secondary_metadata,
                            metadata.as_ref(),
                        ))
                    },
                ),
        )
}

fn render_clickable_row_secondary_metadata(
    secondary_metadata: Option<&str>,
    metadata: Option<&RowMetadata>,
) -> gpui::Div {
    let mut dim_parts = Vec::new();
    if let Some(secondary_metadata) = secondary_metadata
        && !secondary_metadata.is_empty()
    {
        dim_parts.push(secondary_metadata.to_string());
    }
    if let Some(metadata) = metadata {
        if let Some(provider_label) = metadata.provider_label.as_ref() {
            dim_parts.push(provider_label.clone());
        }
    }
    let dim_text = if let Some(metadata) = metadata {
        if metadata.quality.is_some() && !dim_parts.is_empty() {
            format!("{}  • ", dim_parts.join("  •  "))
        } else {
            dim_parts.join("  •  ")
        }
    } else {
        dim_parts.join("  •  ")
    };

    div()
        .h(px(16.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(theme::SMALL_SIZE))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .when(!dim_text.is_empty(), |row| {
                    row.child(
                        div()
                            .text_color(rgb(theme::TEXT_DIM))
                            .truncate()
                            .child(dim_text),
                    )
                })
                .when_some(
                    metadata.and_then(|metadata| metadata.quality.as_ref()),
                    |row, quality| {
                        row.child(
                            div()
                                .text_color(rgb(quality.color))
                                .truncate()
                                .child(quality.label.clone()),
                        )
                    },
                ),
        )
}

pub(super) fn source_menu_row(
    label: impl Into<String>,
    active: bool,
    requires_login: bool,
    listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .px(px(theme::SPACE_2))
        .py(px(theme::SPACE_2))
        .rounded(px(8.))
        .cursor_pointer()
        .bg(rgb(if active {
            theme::ACCENT_PRIMARY_LIGHT
        } else {
            theme::SURFACE_FLOATING
        }))
        .text_size(px(theme::META_SIZE))
        .text_color(rgb(if active {
            theme::ACCENT_PRIMARY
        } else {
            theme::TEXT_PRIMARY
        }))
        .flex()
        .items_center()
        .gap(px(theme::SPACE_2))
        .child(
            div()
                .w(px(12.))
                .text_size(px(theme::META_SIZE))
                .text_color(rgb(if active {
                    theme::ACCENT_PRIMARY
                } else {
                    theme::TEXT_DIM
                }))
                .child(if active { "✓" } else { "" }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(theme::META_SIZE))
                .child(label.into()),
        )
        .when(requires_login && !active, |row| {
            row.child(
                div()
                    .text_size(px(theme::SMALL_SIZE))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child("Sign in".to_string()),
            )
        })
        .on_mouse_down(MouseButton::Left, listener)
}

pub(super) fn render_row_metadata(metadata: &RowMetadata) -> gpui::Div {
    let has_provider = metadata.provider_label.is_some();
    let has_quality = metadata.quality.is_some();

    div()
        .min_w(px(0.))
        .max_w(px(150.))
        .flex()
        .flex_shrink()
        .justify_end()
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(theme::SPACE_1))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(theme::SMALL_SIZE))
                .when_some(metadata.provider_label.as_ref(), |row, provider_label| {
                    row.child(
                        div()
                            .text_color(rgb(theme::TEXT_DIM))
                            .truncate()
                            .child(provider_label.clone()),
                    )
                })
                .when(has_provider && has_quality, |row| {
                    row.child(
                        div()
                            .text_color(rgb(theme::TEXT_DIM))
                            .child("•".to_string()),
                    )
                })
                .when_some(metadata.quality.as_ref(), |row, quality| {
                    row.child(
                        div()
                            .text_color(rgb(quality.color))
                            .truncate()
                            .child(quality.label.clone()),
                    )
                }),
        )
}

pub(super) fn artist_album_metadata(
    provider: ProviderId,
    tracks: &[TrackSummary],
    collection_id: &str,
) -> Option<RowMetadata> {
    let quality = summarize_collection_quality(
        tracks
            .iter()
            .filter(|track| track.collection_id.as_deref() == Some(collection_id)),
    )
    .and_then(|summary| collection_quality_metadata(&summary));
    metadata_label(provider, quality, provider != ProviderId::Local)
}

pub(super) fn metadata_label(
    provider: ProviderId,
    quality: Option<QualityMetadata>,
    show_provider: bool,
) -> Option<RowMetadata> {
    let provider_label = show_provider.then(|| provider.short_display_name().to_string());
    if provider_label.is_none() && quality.is_none() {
        None
    } else {
        Some(RowMetadata {
            provider_label,
            quality,
        })
    }
}

pub(super) fn quality_metadata_for_grade(grade: AudioQualityGrade) -> QualityMetadata {
    let color = match grade {
        AudioQualityGrade::Lossless => theme::QUALITY_LOSSLESS,
        AudioQualityGrade::High => theme::QUALITY_HIGH,
        AudioQualityGrade::Standard => theme::QUALITY_STANDARD,
        AudioQualityGrade::Low => theme::QUALITY_LOW,
    };

    QualityMetadata {
        label: grade.label().to_string(),
        color,
    }
}

pub(super) fn collection_quality_metadata(
    summary: &CollectionQualitySummary,
) -> Option<QualityMetadata> {
    match summary {
        CollectionQualitySummary::Uniform(quality) => Some(quality_metadata_for_grade(*quality)),
        CollectionQualitySummary::Range { lowest, highest }
            if *lowest == AudioQualityGrade::High && *highest == AudioQualityGrade::Lossless =>
        {
            Some(QualityMetadata {
                label: "High / Lossless".to_string(),
                color: theme::QUALITY_HIGH,
            })
        }
        CollectionQualitySummary::Range { .. } => Some(QualityMetadata {
            label: "Mixed".to_string(),
            color: theme::TEXT_DIM,
        }),
    }
}

pub(super) fn action_button(
    label: &str,
    primary: bool,
    listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .rounded(px(999.))
        .border_1()
        .border_color(rgb(if primary {
            theme::ACCENT_PRIMARY
        } else {
            theme::BORDER_SUBTLE
        }))
        .bg(rgb(if primary {
            theme::ACCENT_PRIMARY
        } else {
            theme::SURFACE_BASE
        }))
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_2))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, listener)
        .child(
            div()
                .text_size(px(theme::SMALL_SIZE))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(if primary {
                    theme::SURFACE_BASE
                } else {
                    theme::TEXT_PRIMARY
                }))
                .child(label.to_string()),
        )
}

pub(super) fn render_collection_artwork(artwork_url: Option<String>, size: f32) -> gpui::Div {
    let corner_radius = 10.;
    let frame = div()
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .rounded(px(corner_radius))
        .overflow_hidden()
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .bg(rgb(theme::SURFACE_BASE));

    match artwork_url {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => frame.child(
            img(url)
                .w_full()
                .h_full()
                .rounded(px(corner_radius))
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| collection_artwork_fallback().into_any_element()),
        ),
        Some(path) => frame.child(
            img(PathBuf::from(path))
                .w_full()
                .h_full()
                .rounded(px(corner_radius))
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| collection_artwork_fallback().into_any_element()),
        ),
        None => frame.child(collection_artwork_fallback()),
    }
}

pub(super) fn render_track_list_artwork(track_list: &TrackList, size: f32) -> gpui::Div {
    let fallback_artwork = track_list.collection.artwork_url.clone().or_else(|| {
        track_list
            .tracks
            .iter()
            .find_map(|track| track.artwork_url.clone())
    });

    if supports_artwork_collage(track_list) {
        let artwork_urls = artwork_collage_urls(track_list);

        if artwork_urls.len() >= 2 {
            return render_artwork_collage(&artwork_urls, size);
        }

        if let Some(artwork) = artwork_urls.into_iter().next() {
            return render_collection_artwork(Some(artwork), size);
        }
    }

    render_collection_artwork(fallback_artwork, size)
}

fn artwork_collage_urls(track_list: &TrackList) -> Vec<String> {
    if is_local_artist(track_list) {
        return artist_album_collage_urls(track_list);
    }

    let mut seen = HashSet::new();

    track_list
        .tracks
        .iter()
        .filter_map(|track| track.artwork_url.as_deref())
        .map(str::trim)
        .filter(|artwork| !artwork.is_empty())
        .filter(|artwork| seen.insert((*artwork).to_string()))
        .take(4)
        .map(str::to_string)
        .collect()
}

fn artist_album_collage_urls(track_list: &TrackList) -> Vec<String> {
    let mut artwork_urls = Vec::new();
    let mut current_album_key: Option<String> = None;
    let mut current_album_artwork: Option<String> = None;

    for track in &track_list.tracks {
        let album_key = track
            .collection_id
            .clone()
            .or_else(|| track.collection_title.clone())
            .unwrap_or_else(|| track.reference.id.clone());
        let track_artwork = track
            .artwork_url
            .as_deref()
            .map(str::trim)
            .filter(|artwork| !artwork.is_empty())
            .map(str::to_string);

        if current_album_key.as_deref() != Some(album_key.as_str()) {
            if let Some(artwork_url) = current_album_artwork.take() {
                artwork_urls.push(artwork_url);
                if artwork_urls.len() == 4 {
                    return artwork_urls;
                }
            }
            current_album_key = Some(album_key);
            current_album_artwork = track_artwork;
            continue;
        }

        if current_album_artwork.is_none() {
            current_album_artwork = track_artwork;
        }
    }

    if let Some(artwork_url) = current_album_artwork {
        artwork_urls.push(artwork_url);
    }

    artwork_urls.truncate(4);
    artwork_urls
}

fn render_artwork_collage(artwork_urls: &[String], size: f32) -> gpui::Div {
    let corner_radius = 10.;
    let gap = 1.;
    let frame = div()
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .rounded(px(corner_radius))
        .overflow_hidden()
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .bg(rgb(theme::SURFACE_BASE))
        .relative();

    let mut collage = frame;
    for (tile, artwork_url) in artwork_collage_layout(artwork_urls.len(), size, gap)
        .into_iter()
        .zip(artwork_urls.iter().cloned())
    {
        collage = collage.child(
            div()
                .absolute()
                .left(px(tile.left))
                .top(px(tile.top))
                .w(px(tile.width))
                .h(px(tile.height))
                .bg(rgb(theme::SURFACE_BASE))
                .child(render_collage_tile(Some(artwork_url))),
        );
    }

    collage
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CollageTile {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

fn artwork_collage_layout(tile_count: usize, size: f32, gap: f32) -> Vec<CollageTile> {
    match tile_count.min(4) {
        0 => Vec::new(),
        1 => vec![CollageTile {
            left: 0.,
            top: 0.,
            width: size.max(1.0),
            height: size.max(1.0),
        }],
        2 => {
            let tile_width = ((size - gap) / 2.0).max(1.0);
            vec![
                CollageTile {
                    left: 0.,
                    top: 0.,
                    width: tile_width,
                    height: size.max(1.0),
                },
                CollageTile {
                    left: tile_width + gap,
                    top: 0.,
                    width: tile_width,
                    height: size.max(1.0),
                },
            ]
        }
        3 => {
            let column_width = ((size - gap) / 2.0).max(1.0);
            let stacked_height = ((size - gap) / 2.0).max(1.0);
            vec![
                CollageTile {
                    left: 0.,
                    top: 0.,
                    width: column_width,
                    height: size.max(1.0),
                },
                CollageTile {
                    left: column_width + gap,
                    top: 0.,
                    width: column_width,
                    height: stacked_height,
                },
                CollageTile {
                    left: column_width + gap,
                    top: stacked_height + gap,
                    width: column_width,
                    height: stacked_height,
                },
            ]
        }
        _ => {
            let tile_size = ((size - gap) / 2.0).max(1.0);
            let second_offset = tile_size + gap;
            vec![
                CollageTile {
                    left: 0.,
                    top: 0.,
                    width: tile_size,
                    height: tile_size,
                },
                CollageTile {
                    left: second_offset,
                    top: 0.,
                    width: tile_size,
                    height: tile_size,
                },
                CollageTile {
                    left: 0.,
                    top: second_offset,
                    width: tile_size,
                    height: tile_size,
                },
                CollageTile {
                    left: second_offset,
                    top: second_offset,
                    width: tile_size,
                    height: tile_size,
                },
            ]
        }
    }
}

fn render_collage_tile(artwork_url: Option<String>) -> gpui::Div {
    let tile = div()
        .size_full()
        .overflow_hidden()
        .bg(rgb(theme::SURFACE_BASE));

    match artwork_url {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => tile.child(
            img(url)
                .w_full()
                .h_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| collection_artwork_fallback().into_any_element()),
        ),
        Some(path) => tile.child(
            img(PathBuf::from(path))
                .w_full()
                .h_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| collection_artwork_fallback().into_any_element()),
        ),
        None => tile.child(collection_artwork_fallback()),
    }
}

fn supports_artwork_collage(track_list: &TrackList) -> bool {
    is_local_playlist(track_list) || is_local_artist(track_list)
}

fn is_local_playlist(track_list: &TrackList) -> bool {
    track_list.collection.reference.kind == CollectionKind::Playlist
        && track_list.collection.reference.provider == ProviderId::Local
}

fn is_local_artist(track_list: &TrackList) -> bool {
    track_list.collection.reference.kind == CollectionKind::Album
        && track_list.collection.reference.provider == ProviderId::Local
        && track_list
            .collection
            .reference
            .id
            .starts_with("local-artist:")
}

fn collection_artwork_fallback() -> gpui::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(theme::SURFACE_BASE))
        .child(render_icon_with_color(AppIcon::Music, 20., theme::TEXT_DIM))
}

pub(super) fn empty_state(message: &str) -> gpui::Div {
    div()
        .w_full()
        .flex_1()
        .min_h_0()
        .bg(rgb(theme::SURFACE_FLOATING))
        .rounded(px(10.))
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_3))
        .text_size(px(theme::BODY_SIZE))
        .text_color(rgb(theme::TEXT_DIM))
        .child(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        CollageTile, artist_album_collage_urls, artwork_collage_layout,
        collection_quality_metadata, sidebar_secondary_metadata,
    };
    use crate::app::library::{
        AudioQualityGrade, CollectionQualitySummary, summarize_collection_quality,
    };
    use crate::provider::{
        AudioFormat, CollectionKind, CollectionRef, CollectionSummary, ProviderId, TrackList,
        TrackRef, TrackSummary,
    };
    use crate::theme;

    #[test]
    fn artwork_collage_layout_uses_two_panel_split_for_two_images() {
        assert_eq!(
            artwork_collage_layout(2, 62., 1.),
            vec![
                CollageTile {
                    left: 0.,
                    top: 0.,
                    width: 30.5,
                    height: 62.,
                },
                CollageTile {
                    left: 31.5,
                    top: 0.,
                    width: 30.5,
                    height: 62.,
                },
            ]
        );
    }

    #[test]
    fn artwork_collage_layout_uses_three_tile_composition_for_three_images() {
        assert_eq!(
            artwork_collage_layout(3, 62., 1.),
            vec![
                CollageTile {
                    left: 0.,
                    top: 0.,
                    width: 30.5,
                    height: 62.,
                },
                CollageTile {
                    left: 31.5,
                    top: 0.,
                    width: 30.5,
                    height: 30.5,
                },
                CollageTile {
                    left: 31.5,
                    top: 31.5,
                    width: 30.5,
                    height: 30.5,
                },
            ]
        );
    }

    #[test]
    fn local_artist_collage_uses_one_image_per_album() {
        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    ProviderId::Local,
                    "local-artist:Artist One",
                    CollectionKind::Album,
                    None,
                ),
                title: "Artist One".to_string(),
                subtitle: Some("2 albums".to_string()),
                artwork_url: None,
                track_count: Some(4),
            },
            tracks: vec![
                track_summary(
                    "track-1",
                    Some("album-a"),
                    Some("Album A"),
                    Some("https://cdn.example/track-a-variant.jpg"),
                ),
                track_summary(
                    "track-2",
                    Some("album-a"),
                    Some("Album A"),
                    Some("https://cdn.example/album-a.jpg"),
                ),
                track_summary(
                    "track-3",
                    Some("album-b"),
                    Some("Album B"),
                    Some("https://cdn.example/album-b.jpg"),
                ),
                track_summary(
                    "track-4",
                    Some("album-b"),
                    Some("Album B"),
                    Some("https://cdn.example/track-b-variant.jpg"),
                ),
            ],
        };

        assert_eq!(
            artist_album_collage_urls(&track_list),
            vec![
                "https://cdn.example/track-a-variant.jpg".to_string(),
                "https://cdn.example/album-b.jpg".to_string(),
            ]
        );
    }

    #[test]
    fn sidebar_secondary_metadata_omits_kind_label_when_not_requested() {
        assert_eq!(
            sidebar_secondary_metadata(None, Some(12), "2 albums"),
            Some("12 tracks".to_string())
        );
    }

    #[test]
    fn summarizes_high_and_lossless_collections_as_a_range() {
        let track_list = TrackList {
            collection: CollectionSummary {
                reference: CollectionRef::new(
                    ProviderId::Local,
                    "album-a",
                    CollectionKind::Album,
                    None,
                ),
                title: "Album A".to_string(),
                subtitle: Some("Artist One".to_string()),
                artwork_url: None,
                track_count: Some(2),
            },
            tracks: vec![
                track_summary_with_quality(
                    "track-1",
                    Some("album-a"),
                    Some("Album A"),
                    Some(AudioFormat::Flac),
                    None,
                ),
                track_summary_with_quality(
                    "track-2",
                    Some("album-a"),
                    Some("Album A"),
                    Some(AudioFormat::Mp3),
                    Some(320_000),
                ),
            ],
        };

        assert_eq!(
            summarize_collection_quality(track_list.tracks.iter()),
            Some(CollectionQualitySummary::Range {
                lowest: AudioQualityGrade::High,
                highest: AudioQualityGrade::Lossless,
            })
        );
    }

    #[test]
    fn quality_metadata_uses_explicit_label_for_high_and_lossless_ranges() {
        assert_eq!(
            collection_quality_metadata(&CollectionQualitySummary::Range {
                lowest: AudioQualityGrade::High,
                highest: AudioQualityGrade::Lossless,
            }),
            Some(super::QualityMetadata {
                label: "High / Lossless".to_string(),
                color: theme::QUALITY_HIGH,
            })
        );
    }

    fn track_summary(
        id: &str,
        collection_id: Option<&str>,
        collection_title: Option<&str>,
        artwork_url: Option<&str>,
    ) -> TrackSummary {
        track_summary_with_quality(id, collection_id, collection_title, None, None)
            .with_artwork(artwork_url)
    }

    fn track_summary_with_quality(
        id: &str,
        collection_id: Option<&str>,
        collection_title: Option<&str>,
        audio_format: Option<AudioFormat>,
        bitrate_bps: Option<u32>,
    ) -> TrackSummary {
        TrackSummary {
            reference: TrackRef::new(ProviderId::Local, id, None, Some(id.to_string())),
            title: id.to_string(),
            artist: Some("Artist One".to_string()),
            album: collection_title.map(str::to_string),
            collection_id: collection_id.map(str::to_string),
            collection_title: collection_title.map(str::to_string),
            collection_subtitle: Some("Artist One".to_string()),
            duration_seconds: None,
            bitrate_bps,
            audio_format,
            artwork_url: None,
        }
    }

    trait TrackSummaryTestExt {
        fn with_artwork(self, artwork_url: Option<&str>) -> Self;
    }

    impl TrackSummaryTestExt for TrackSummary {
        fn with_artwork(mut self, artwork_url: Option<&str>) -> Self {
            self.artwork_url = artwork_url.map(str::to_string);
            self
        }
    }
}
