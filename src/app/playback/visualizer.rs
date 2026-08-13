use std::f32::consts::{PI, TAU};
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    AnyElement, BorderStyle, Bounds, Context, Entity, EventEmitter, FontWeight, Hsla, IntoElement,
    MouseButton, MouseDownEvent, Path, PathBuilder, Pixels, Point, Render, Window, canvas, div,
    hsla, point, px, quad, rgb, size,
};

use crate::audio::{VISUALIZER_NOTES, VISUALIZER_OCTAVES, VisualizerSnapshot};
use crate::model::PlaybackStatus;
use crate::theme;

use super::super::ui::{ModalWidth, render_modal_body, render_modal_card, render_modal_overlay};
use super::PlaybackModule;

const DISPLAY_RESPONSE_SECONDS: f32 = 0.055;
const OCTAVE_FADE_RESPONSE_SECONDS: f32 = 0.14;
const POINTS_PER_NOTE: usize = 6;
const RADIAL_POINTS: usize = VISUALIZER_NOTES * POINTS_PER_NOTE;
const ARC_GATE: f32 = 0.16;
const MAX_VISIBLE_OCTAVES: usize = 3;
const COMPACT_HEIGHT_MIN: f32 = 92.0;
const COMPACT_HEIGHT_MAX: f32 = 132.0;
const COMPACT_GROWTH_START: f32 = 1180.0;
const COMPACT_GROWTH_END: f32 = 1440.0;
const COMPACT_MINIMUM_RADIUS_RATIO: f32 = 0.28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum VisualizerMode {
    Compact,
    Modal,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum VisualizerEvent {
    OpenRequested,
    CloseRequested,
}

pub(in crate::app) struct VisualizerView {
    playback: Entity<PlaybackModule>,
    mode: VisualizerMode,
    visible: bool,
    displayed: VisualizerSnapshot,
    octave_visibility: [f32; VISUALIZER_OCTAVES],
    last_frame: Option<Instant>,
}

impl EventEmitter<VisualizerEvent> for VisualizerView {}

impl VisualizerView {
    pub(in crate::app) fn new(playback: Entity<PlaybackModule>, mode: VisualizerMode) -> Self {
        Self {
            playback,
            mode,
            visible: matches!(mode, VisualizerMode::Compact),
            displayed: VisualizerSnapshot::default(),
            octave_visibility: [0.0; VISUALIZER_OCTAVES],
            last_frame: None,
        }
    }

    pub(in crate::app) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.last_frame = None;
        cx.notify();
    }

    fn animate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            self.last_frame = None;
            return;
        }

        let playback = self.playback.read(cx);
        match playback.playback_status() {
            PlaybackStatus::Playing => {
                let target = playback.visualizer_snapshot();
                let now = Instant::now();
                let elapsed = self
                    .last_frame
                    .map(|last| now.saturating_duration_since(last).as_secs_f32())
                    .unwrap_or(1.0 / 60.0)
                    .min(0.1);
                let amount = 1.0 - (-elapsed / DISPLAY_RESPONSE_SECONDS).exp();
                blend_snapshot(&mut self.displayed, target, amount);
                let visibility_target = octave_visibility_targets(&self.displayed.octaves);
                let visibility_amount = 1.0 - (-elapsed / OCTAVE_FADE_RESPONSE_SECONDS).exp();
                for (visibility, target) in self.octave_visibility.iter_mut().zip(visibility_target)
                {
                    *visibility += (target - *visibility) * visibility_amount;
                }
                self.last_frame = Some(now);
                window.request_animation_frame();
            }
            PlaybackStatus::Idle => {
                self.displayed = VisualizerSnapshot::default();
                self.octave_visibility = [0.0; VISUALIZER_OCTAVES];
                self.last_frame = None;
            }
            PlaybackStatus::Buffering | PlaybackStatus::Paused => self.last_frame = None,
        }
    }

    fn render_compact(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let playback = self.playback.read(cx);
        let disabled = playback.now_playing().is_none() || playback.play_loading();
        let viewport_width = window.viewport_size().width.to_f64() as f32;

        div()
            .w_full()
            .h(px(compact_visualizer_height(viewport_width)))
            .rounded(px(10.))
            .bg(rgb(theme::BG_CANVAS))
            .when(!disabled, |visualizer| visualizer.cursor_pointer())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event: &MouseDownEvent, _window, cx| {
                    if !disabled {
                        cx.emit(VisualizerEvent::OpenRequested);
                    }
                }),
            )
            .child(render_radial(self.displayed, self.octave_visibility, true).size_full())
            .into_any_element()
    }

    fn render_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let playback = self.playback.read(cx);
        let title = playback
            .now_playing()
            .map(|track| track.title)
            .unwrap_or_else(|| "Nothing selected".to_string());
        render_modal_overlay(render_modal_card(
            ModalWidth::Wide,
            render_modal_body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_4))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(theme::SPACE_1))
                                    .child(
                                        div()
                                            .text_size(px(18.))
                                            .font_weight(FontWeight::BOLD)
                                            .child("Visualizer"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(theme::META_SIZE))
                                            .text_color(rgb(theme::TEXT_MUTED))
                                            .child(title),
                                    ),
                            )
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
                                            |_this, _event: &MouseDownEvent, _window, cx| {
                                                cx.emit(VisualizerEvent::CloseRequested);
                                            },
                                        ),
                                    )
                                    .child("Close"),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(300.))
                            .p(px(theme::SPACE_4))
                            .rounded(px(12.))
                            .bg(rgb(theme::SURFACE_BASE))
                            .child(
                                render_radial(self.displayed, self.octave_visibility, false)
                                    .size_full(),
                            ),
                    ),
                false,
            ),
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
                cx.emit(VisualizerEvent::CloseRequested);
            }),
        )
        .into_any_element()
    }
}

impl Render for VisualizerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.animate(window, cx);
        match self.mode {
            VisualizerMode::Compact => self.render_compact(window, cx),
            VisualizerMode::Modal => self.render_modal(cx),
        }
    }
}

fn compact_visualizer_height(viewport_width: f32) -> f32 {
    let progress = ((viewport_width - COMPACT_GROWTH_START)
        / (COMPACT_GROWTH_END - COMPACT_GROWTH_START))
        .clamp(0.0, 1.0);
    COMPACT_HEIGHT_MIN + (COMPACT_HEIGHT_MAX - COMPACT_HEIGHT_MIN) * progress
}

fn render_radial(
    snapshot: VisualizerSnapshot,
    octave_visibility: [f32; VISUALIZER_OCTAVES],
    compact: bool,
) -> gpui::Canvas<()> {
    canvas(
        |_bounds, _window, _cx| {},
        move |bounds, (), window, _cx| {
            paint_radial(bounds, snapshot, octave_visibility, compact, window)
        },
    )
}

fn paint_radial(
    bounds: Bounds<Pixels>,
    snapshot: VisualizerSnapshot,
    octave_visibility: [f32; VISUALIZER_OCTAVES],
    compact: bool,
    window: &mut Window,
) {
    let width = bounds.size.width.to_f64() as f32;
    let height = bounds.size.height.to_f64() as f32;
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let center = point(
        bounds.origin.x + px(width * 0.5),
        bounds.origin.y + px(height * 0.5),
    );
    let maximum_radius = width.min(height) * 0.47;
    let minimum_radius = if compact {
        maximum_radius * COMPACT_MINIMUM_RADIUS_RATIO
    } else {
        0.75
    };
    let base_radius = amplitude_radius(minimum_radius, maximum_radius, snapshot.level);

    // Paint low octaves first so higher, redder bands win where fills overlap.
    for (octave, (notes, visibility)) in snapshot.octaves.iter().zip(octave_visibility).enumerate()
    {
        if visibility < 0.01 {
            continue;
        }
        let levels = interpolated_note_levels(smoothed_notes(*notes));
        let mut color = octave_color(octave);
        color.a = (visibility * 1.15).clamp(0.0, 0.92);
        for arc in active_arcs(levels) {
            if let Some(path) =
                arc_fill_path(center, base_radius, maximum_radius, snapshot.level, &arc)
            {
                window.paint_path(path, color);
            }
        }
    }

    let circle_stroke = if compact { px(1.0) } else { px(1.5) };
    let circle_bounds = Bounds::new(
        point(center.x - px(base_radius), center.y - px(base_radius)),
        size(px(base_radius * 2.0), px(base_radius * 2.0)),
    );
    window.paint_quad(quad(
        circle_bounds,
        px(base_radius),
        gpui::transparent_black(),
        circle_stroke,
        hsla(0.0, 0.0, 0.96, 0.92),
        BorderStyle::Solid,
    ));
}

fn amplitude_radius(minimum_radius: f32, maximum_radius: f32, level: f32) -> f32 {
    let level = level.clamp(0.0, 1.0);
    minimum_radius + (maximum_radius * 0.64 - minimum_radius) * level
}

fn octave_radius(base_radius: f32, maximum_radius: f32, level: f32, note_level: f32) -> f32 {
    let level = level.clamp(0.0, 1.0);
    let note_level = note_level.clamp(0.0, 1.0).powf(0.85);
    base_radius + maximum_radius * 0.32 * level * note_level
}

fn blend_snapshot(current: &mut VisualizerSnapshot, target: VisualizerSnapshot, amount: f32) {
    current.level += (target.level - current.level) * amount;
    for (current_octave, target_octave) in current.octaves.iter_mut().zip(target.octaves) {
        for (current, target) in current_octave.iter_mut().zip(target_octave) {
            *current += (target - *current) * amount;
        }
    }
}

fn smoothed_notes(notes: [f32; VISUALIZER_NOTES]) -> [f32; VISUALIZER_NOTES] {
    let mut levels = [0.0; VISUALIZER_NOTES];
    for (index, level) in levels.iter_mut().enumerate() {
        let left = (index + VISUALIZER_NOTES - 1) % VISUALIZER_NOTES;
        let right = (index + 1) % VISUALIZER_NOTES;
        *level = notes[left] * 0.12 + notes[index] * 0.76 + notes[right] * 0.12;
    }
    levels
}

fn octave_visibility_targets(
    octaves: &[[f32; VISUALIZER_NOTES]; VISUALIZER_OCTAVES],
) -> [f32; VISUALIZER_OCTAVES] {
    let mut ranked: [(usize, f32); VISUALIZER_OCTAVES] = std::array::from_fn(|octave| {
        let strength = octaves[octave].iter().copied().fold(0.0_f32, f32::max);
        (octave, strength)
    });
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));

    let mut targets = [0.0; VISUALIZER_OCTAVES];
    let strongest = ranked[0].1;
    if strongest <= ARC_GATE {
        return targets;
    }
    for (octave, strength) in ranked.into_iter().take(MAX_VISIBLE_OCTAVES) {
        if strength > ARC_GATE {
            targets[octave] = (strength / strongest).sqrt();
        }
    }
    targets
}

fn interpolated_note_levels(notes: [f32; VISUALIZER_NOTES]) -> [f32; RADIAL_POINTS] {
    std::array::from_fn(|point_index| {
        let note_position = point_index as f32 / POINTS_PER_NOTE as f32;
        let note = note_position.floor() as usize;
        let next_note = (note + 1) % VISUALIZER_NOTES;
        let progress = note_position - note as f32;
        let blend = progress * progress * (3.0 - 2.0 * progress);
        notes[note] + (notes[next_note] - notes[note]) * blend
    })
}

#[derive(Debug, PartialEq)]
struct ActiveArc {
    samples: Vec<(usize, f32)>,
}

fn active_arcs(levels: [f32; RADIAL_POINTS]) -> Vec<ActiveArc> {
    let activity = levels.map(|level| ((level - ARC_GATE) / (1.0 - ARC_GATE)).clamp(0.0, 1.0));
    if activity.iter().all(|level| *level > 0.0) {
        let mut samples = activity.into_iter().enumerate().collect::<Vec<_>>();
        samples.push((0, samples[0].1));
        return vec![ActiveArc { samples }];
    }

    let first_inactive = activity
        .iter()
        .position(|level| *level == 0.0)
        .expect("an inactive point should exist");
    let mut arcs = Vec::new();
    let mut current = Vec::new();
    for offset in 0..=RADIAL_POINTS {
        let point_index = (first_inactive + offset) % RADIAL_POINTS;
        let level = activity[point_index];
        if level > 0.0 {
            if current.is_empty() {
                current.push(((point_index + RADIAL_POINTS - 1) % RADIAL_POINTS, 0.0));
            }
            current.push((point_index, level));
        } else if !current.is_empty() {
            current.push((point_index, 0.0));
            arcs.push(ActiveArc {
                samples: std::mem::take(&mut current),
            });
        }
    }
    arcs
}

fn arc_fill_path(
    center: Point<Pixels>,
    base_radius: f32,
    maximum_radius: f32,
    level: f32,
    arc: &ActiveArc,
) -> Option<Path<Pixels>> {
    let mut builder = PathBuilder::fill();
    for (sample_index, (point_index, activity)) in arc.samples.iter().enumerate() {
        let radius = octave_radius(base_radius, maximum_radius, level, *activity);
        let point = radial_point(center, *point_index, radius);
        if sample_index == 0 {
            builder.move_to(point);
        } else {
            builder.line_to(point);
        }
    }
    for (point_index, _) in arc.samples.iter().rev() {
        builder.line_to(radial_point(center, *point_index, base_radius));
    }
    builder.close();
    builder.build().ok()
}

fn radial_point(center: Point<Pixels>, point_index: usize, radius: f32) -> Point<Pixels> {
    let angle = -PI * 0.5 + TAU * point_index as f32 / RADIAL_POINTS as f32;
    point(
        center.x + px(angle.cos() * radius),
        center.y + px(angle.sin() * radius),
    )
}

fn octave_color(octave: usize) -> Hsla {
    let position = octave.min(VISUALIZER_OCTAVES - 1) as f32 / (VISUALIZER_OCTAVES - 1) as f32;
    let hue = 0.61 + (0.99 - 0.61) * position;
    let lightness = 0.64 + (0.49 - 0.64) * position;
    hsla(hue, 0.90, lightness, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_visualizer_grows_on_wide_windows() {
        assert_eq!(compact_visualizer_height(960.0), COMPACT_HEIGHT_MIN);
        assert_eq!(compact_visualizer_height(1180.0), COMPACT_HEIGHT_MIN);
        assert_eq!(compact_visualizer_height(1310.0), 112.0);
        assert_eq!(compact_visualizer_height(1440.0), COMPACT_HEIGHT_MAX);
        assert_eq!(compact_visualizer_height(2048.0), COMPACT_HEIGHT_MAX);
    }

    #[test]
    fn compact_visualizer_keeps_a_visible_quiet_radius() {
        let maximum_radius = COMPACT_HEIGHT_MAX * 0.47;
        let minimum_radius = maximum_radius * COMPACT_MINIMUM_RADIUS_RATIO;

        assert!(minimum_radius > 17.0);
        assert_eq!(
            amplitude_radius(minimum_radius, maximum_radius, 0.0),
            minimum_radius
        );
        assert!(amplitude_radius(minimum_radius, maximum_radius, 1.0) > minimum_radius);
    }

    #[test]
    fn octave_colors_run_from_low_blue_to_high_red() {
        let low = octave_color(0);
        let high = octave_color(VISUALIZER_OCTAVES - 1);

        assert!(low.h < high.h);
        assert!(low.l > high.l);
        assert!((0.58..=0.64).contains(&low.h));
        assert!(high.h >= 0.98);
    }

    #[test]
    fn note_smoothing_wraps_around_the_circle() {
        let mut notes = [0.0; VISUALIZER_NOTES];
        notes[0] = 1.0;

        let levels = smoothed_notes(notes);

        assert_eq!(levels[VISUALIZER_NOTES - 1], 0.12);
        assert_eq!(levels[0], 0.76);
        assert_eq!(levels[1], 0.12);
    }

    #[test]
    fn only_the_three_strongest_octaves_are_targeted() {
        let mut octaves = [[0.0; VISUALIZER_NOTES]; VISUALIZER_OCTAVES];
        for (octave, notes) in octaves.iter_mut().enumerate() {
            notes[0] = (octave + 1) as f32 * 0.1;
        }

        let visibility = octave_visibility_targets(&octaves);

        assert_eq!(visibility.iter().filter(|value| **value > 0.0).count(), 3);
        assert_eq!(visibility[VISUALIZER_OCTAVES - 1], 1.0);
        assert_eq!(visibility[0], 0.0);
    }

    #[test]
    fn active_arcs_taper_to_the_circle_and_join_across_the_wrap() {
        let mut levels = [0.0; RADIAL_POINTS];
        levels[RADIAL_POINTS - 1] = 1.0;
        levels[0] = 1.0;

        let arcs = active_arcs(levels);

        assert_eq!(arcs.len(), 1);
        assert_eq!(arcs[0].samples.first().unwrap().1, 0.0);
        assert_eq!(arcs[0].samples.last().unwrap().1, 0.0);
        assert!(
            arcs[0]
                .samples
                .iter()
                .any(|(point, level)| *point == RADIAL_POINTS - 1 && *level > 0.0)
        );
        assert!(
            arcs[0]
                .samples
                .iter()
                .any(|(point, level)| *point == 0 && *level > 0.0)
        );
    }

    #[test]
    fn fully_active_band_repeats_the_top_point_to_close_the_fill() {
        let arcs = active_arcs([1.0; RADIAL_POINTS]);

        assert_eq!(arcs.len(), 1);
        assert_eq!(arcs[0].samples.len(), RADIAL_POINTS + 1);
        assert_eq!(arcs[0].samples.first().unwrap().0, 0);
        assert_eq!(arcs[0].samples.last().unwrap().0, 0);
        assert!(arcs[0].samples.first().unwrap().1 > 0.0);
    }

    #[test]
    fn amplitude_circle_and_octave_lines_have_separate_ranges() {
        let silent_circle = amplitude_radius(0.5, 100.0, 0.0);
        let loud_circle = amplitude_radius(0.5, 100.0, 1.0);
        let inactive_note = octave_radius(loud_circle, 100.0, 1.0, 0.0);
        let active_note = octave_radius(loud_circle, 100.0, 1.0, 1.0);

        assert_eq!(silent_circle, 0.5);
        assert_eq!(loud_circle, 64.0);
        assert_eq!(inactive_note, loud_circle);
        assert_eq!(active_note, 96.0);
    }
}
