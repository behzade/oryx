use std::collections::HashSet;
use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    App, IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Styled, Window, div,
    img, px, relative, rgb,
};

use crate::app::library::AudioQualityGrade;
use crate::progressive::ProgressiveSnapshot;
use crate::provider::{ProviderId, TrackList, TrackSummary};
use crate::theme;

use super::{
    AppIcon, AudioQuality, CollectionQualitySummary, normalized_audio_quality_grade,
    render_icon_with_color,
};

#[derive(Clone, Debug)]
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
    kind_label: &str,
    track_count: Option<usize>,
    primary_metadata: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    if primary_metadata != kind_label {
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

pub(super) fn download_progress_label(snapshot: Option<ProgressiveSnapshot>) -> String {
    match snapshot.and_then(download_progress_ratio) {
        Some(ratio) => format!("{}%", (ratio * 100.0).round() as u32),
        None => "Downloading".to_string(),
    }
}

pub(super) fn render_download_progress_line(snapshot: ProgressiveSnapshot) -> gpui::Div {
    let ratio = download_progress_ratio(snapshot).unwrap_or(0.0);

    div()
        .absolute()
        .top_0()
        .left_0()
        .right(px(0.))
        .h(px(3.))
        .bg(rgb(theme::DOWNLOAD_PROGRESS_LIGHT))
        .child(
            div()
                .w(relative(ratio.clamp(0.0, 1.0)))
                .h_full()
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
        .rounded(px(8.))
        .bg(rgb(0x00000000))
        .px(px(theme::SPACE_2))
        .py(px(theme::SPACE_2))
        .cursor_pointer()
        .child(
            div().w(px(14.)).h(px(14.)).overflow_hidden().child(
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
                    .path(AppIcon::Heart.asset_path())
                    .w_full()
                    .h_full()
                    .text_color(rgb(icon_color)),
            ),
        )
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
        .bg(rgb(if active {
            theme::ROW_ACTIVE_BG
        } else {
            theme::ROW_IDLE_BG
        }))
        .hover(move |style| {
            if active {
                style
            } else {
                style
                    .bg(rgb(theme::ROW_HOVER_BG))
                    .border_color(rgb(theme::ROW_HOVER_BORDER))
            }
        })
        .when(active, |row| {
            row.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(div().w(px(3.)).h_full().bg(rgb(theme::ACCENT_PRIMARY))),
            )
        })
}

pub(super) fn clickable_row(
    title: &str,
    primary_metadata: &str,
    secondary_metadata: Option<&str>,
    metadata: Option<RowMetadata>,
    artwork_url: Option<String>,
    active: bool,
) -> gpui::Div {
    row_shell(active, 86., 8.)
        .px(px(theme::SPACE_3))
        .py(px(theme::SPACE_3))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_3))
        .child(render_collection_artwork(artwork_url, 62.))
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

pub(super) fn summarize_track_list_quality(
    track_list: &TrackList,
) -> Option<CollectionQualitySummary> {
    summarize_collection_quality(track_list.tracks.iter())
}

pub(super) fn summarize_collection_quality<'a>(
    tracks: impl Iterator<Item = &'a TrackSummary>,
) -> Option<CollectionQualitySummary> {
    let mut qualities = HashSet::new();
    for track in tracks {
        if let Some(quality) = audio_quality_from_track_summary(track) {
            if let Some(grade) = normalized_audio_quality_grade(&quality) {
                qualities.insert(grade);
            }
        }
    }

    match qualities.len() {
        0 => None,
        1 => Some(CollectionQualitySummary::Uniform(
            qualities
                .into_iter()
                .next()
                .expect("single quality should exist"),
        )),
        _ => Some(CollectionQualitySummary::Mixed),
    }
}

pub(super) fn audio_quality_from_track_summary(track: &TrackSummary) -> Option<AudioQuality> {
    if track.audio_format.is_none() && track.bitrate_bps.is_none() {
        return None;
    }

    Some(AudioQuality {
        audio_format: track.audio_format.clone(),
        bitrate_bps: track.bitrate_bps,
    })
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
        CollectionQualitySummary::Mixed => Some(QualityMetadata {
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
