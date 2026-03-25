use crate::model::{PlaybackStatus, RepeatMode, Track};
use crate::provider::TrackList;
use crate::theme;
use gpui::prelude::*;
use gpui::{
    App, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement,
    Styled, StyledImage, Window, div, img, px, relative, rgb,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::library::{AudioQuality, normalized_audio_quality_label};
use super::super::{
    AppIcon, OryxApp, PLAYER_CENTER_WIDTH, PREVIOUS_RESTART_THRESHOLD, format_clock,
    render_icon_with_color,
};
use super::{PlaybackIntent, PlaybackRuntimeEvent};

#[derive(Clone, Copy)]
enum PlaybackAdvanceReason {
    AutomaticFinish,
    ManualNext,
    ManualPrevious,
}

enum PlaybackAdvanceAction {
    Noop,
    RestartCurrent,
    StartIndex { track_list: TrackList, index: usize },
    Stop(&'static str),
}

impl OryxApp {
    pub(in crate::app) fn play_next(&mut self, cx: &mut Context<Self>) {
        self.advance_playback(PlaybackAdvanceReason::ManualNext, cx);
    }

    pub(in crate::app) fn handle_track_finished(&mut self, cx: &mut Context<Self>) {
        self.advance_playback(PlaybackAdvanceReason::AutomaticFinish, cx);
    }

    pub(in crate::app) fn handle_playback_runtime_event(
        &mut self,
        event: PlaybackRuntimeEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            PlaybackRuntimeEvent::UiRefreshRequested => cx.notify(),
            PlaybackRuntimeEvent::TrackFinished => self.handle_track_finished(cx),
            PlaybackRuntimeEvent::PlaybackFailed => self.handle_playback_failed(cx),
        }
    }

    pub(in crate::app) fn handle_playback_failed(&mut self, cx: &mut Context<Self>) {
        let _ = self.playback_state.read(cx).stop();
        self.update_playback_state(cx, |state| {
            state.set_play_loading(false);
            state.set_playback_status(PlaybackStatus::Paused);
        });
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(in crate::app) fn play_previous(&mut self, cx: &mut Context<Self>) {
        self.advance_playback(PlaybackAdvanceReason::ManualPrevious, cx);
    }

    pub(in crate::app) fn toggle_playback_from_ui(&mut self, cx: &mut Context<Self>) {
        self.dispatch_playback_intent(PlaybackIntent::Toggle, cx);
    }

    pub(in crate::app) fn cycle_repeat_mode(&mut self, cx: &mut Context<Self>) {
        let repeat_mode = self
            .playback_state
            .update(cx, |state, _cx| state.cycle_repeat_mode());
        self.status_message = Some(repeat_mode.label().to_string());
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(in crate::app) fn toggle_shuffle_enabled(&mut self, cx: &mut Context<Self>) {
        let (next_shuffle_enabled, next_shuffle_seed) = {
            let playback_state = self.playback_state.read(cx);
            let next_shuffle_enabled = !playback_state.shuffle_enabled();
            let next_shuffle_seed = if next_shuffle_enabled {
                next_shuffle_seed()
            } else {
                playback_state.shuffle_seed()
            };
            (next_shuffle_enabled, next_shuffle_seed)
        };
        self.update_playback_state(cx, |state| {
            state.set_shuffle_enabled(next_shuffle_enabled, next_shuffle_seed);
        });
        self.status_message = Some(if next_shuffle_enabled {
            "Shuffle On".to_string()
        } else {
            "Shuffle Off".to_string()
        });
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    pub(in crate::app) fn resume_current_track(&mut self, cx: &mut Context<Self>) {
        let resume = {
            let playback_state = self.playback_state.read(cx);
            playback_state.current_track_index().map(|index| {
                let position = if playback_state.resume_position().is_zero() {
                    None
                } else {
                    Some(playback_state.resume_position())
                };
                (index, position)
            })
        };
        if let Some((index, position)) = resume {
            self.play_track_at_position(index, position, cx);
        }
    }

    pub(in crate::app) fn seek_backward_by(&mut self, delta: Duration, cx: &mut Context<Self>) {
        let target = self
            .playback_state
            .read(cx)
            .current_playback_position()
            .saturating_sub(delta);
        self.seek_to(target, cx);
    }

    pub(in crate::app) fn seek_forward_by(&mut self, delta: Duration, cx: &mut Context<Self>) {
        let target = self.playback_state.read(cx).current_playback_position() + delta;
        self.seek_to(target, cx);
    }

    pub(in crate::app) fn seek_to(&mut self, target: Duration, cx: &mut Context<Self>) {
        let Some(duration) = self.playback_state.read(cx).current_track_duration() else {
            return;
        };

        let target = target.min(duration);
        let has_live_playback = self
            .playback_state
            .read(cx)
            .position()
            .ok()
            .flatten()
            .is_some();

        if has_live_playback {
            if let Err(error) = self.playback_state.read(cx).seek_to(target) {
                self.status_message = Some(format!("Seek failed: {error}"));
                cx.notify();
                return;
            }
        }

        self.update_playback_state(cx, |state| {
            state.set_resume_position(target);
        });
        self.persist_session_snapshot(cx);
        self.playback_state
            .read(cx)
            .publish_restored_media_session();
        cx.notify();
    }

    pub(in crate::app) fn seek_to_ratio(
        &mut self,
        ratio: f32,
        _window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(duration) = self.playback_state.read(cx).current_track_duration() else {
            return;
        };
        let seconds = duration.as_secs_f32() * ratio.clamp(0.0, 1.0);
        self.seek_to(Duration::from_secs_f32(seconds), cx);
    }

    pub(in crate::app) fn render_now_playing(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (now_playing, playback_status, repeat_mode, shuffle_enabled, play_loading) = {
            let playback_state = self.playback_state.read(cx);
            (
                playback_state.now_playing(),
                playback_state.playback_status(),
                playback_state.repeat_mode(),
                playback_state.shuffle_enabled(),
                playback_state.play_loading(),
            )
        };
        let progress = self.playback_state.read(cx).current_playback_position();
        let total = self.playback_state.read(cx).current_track_duration();
        let current_track_summary = self.current_playback_track_summary(cx);
        let current_track_liked = current_track_summary
            .as_ref()
            .is_some_and(|track| self.track_is_liked(track, cx));
        let progress_ratio = total
            .map(|total| {
                if total.is_zero() {
                    0.0
                } else {
                    (progress.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
                }
            })
            .unwrap_or(0.0);
        let title = now_playing
            .as_ref()
            .map(|track| track.title.clone())
            .unwrap_or_else(|| "Nothing selected".to_string());
        let detail = now_playing
            .as_ref()
            .map(|track| {
                let mut parts = vec![
                    track.artist.clone(),
                    track.album.clone(),
                    track.duration_label.clone(),
                ];
                if let Some(quality) = now_playing_quality_label(track) {
                    parts.push(track.provider.display_name().to_string());
                    parts.push(quality);
                } else {
                    parts.push(track.provider.display_name().to_string());
                }
                parts.join("  •  ")
            })
            .unwrap_or_else(|| "Choose a track to start playback.".to_string());
        let artwork_path = now_playing
            .as_ref()
            .and_then(|track| track.artwork_path.clone());
        let controls_disabled = now_playing.is_none() || play_loading;
        let play_icon = if matches!(
            playback_status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering
        ) {
            AppIcon::Pause
        } else {
            AppIcon::Play
        };
        let repeat_icon = match repeat_mode {
            RepeatMode::Off | RepeatMode::All => AppIcon::RepeatAll,
            RepeatMode::One => AppIcon::RepeatOne,
        };

        div()
            .w_full()
            .px(px(theme::SPACE_4))
            .py(px(theme::SPACE_3))
            .flex()
            .items_center()
            .gap(px(theme::SPACE_4))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_3))
                    .child(render_artwork(artwork_path, 92.))
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
                                    .child(format!(
                                        "Now Playing  •  {}{}",
                                        playback_status.label(),
                                        if play_loading { "..." } else { "" }
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(theme::META_SIZE))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child(detail),
                            ),
                    ),
            )
            .child(
                div()
                    .w(px(PLAYER_CENTER_WIDTH))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_2))
                    .child(
                        div().w_full().flex().justify_center().child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(theme::SPACE_2))
                                .child(
                                    icon_button(
                                        AppIcon::Shuffle,
                                        false,
                                        controls_disabled,
                                        shuffle_enabled,
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.toggle_shuffle_enabled(cx);
                                            },
                                        ),
                                    ),
                                )
                                .child(
                                    icon_button(AppIcon::SkipBack, false, controls_disabled, false)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(
                                                |this, _event: &MouseDownEvent, _window, cx| {
                                                    this.play_previous(cx);
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    icon_button(play_icon, true, controls_disabled, false)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(
                                                |this, _event: &MouseDownEvent, _window, cx| {
                                                    this.toggle_playback_from_ui(cx);
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    icon_button(
                                        AppIcon::SkipForward,
                                        false,
                                        controls_disabled,
                                        false,
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.play_next(cx);
                                            },
                                        ),
                                    ),
                                )
                                .child(
                                    icon_button(
                                        repeat_icon,
                                        false,
                                        controls_disabled,
                                        !matches!(repeat_mode, RepeatMode::Off),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            |this, _event: &MouseDownEvent, _window, cx| {
                                                this.cycle_repeat_mode(cx);
                                            },
                                        ),
                                    ),
                                )
                                .when_some(current_track_summary.clone(), |controls, track| {
                                    controls.child(
                                        like_icon_button(current_track_liked, controls_disabled)
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    move |this,
                                                          _event: &MouseDownEvent,
                                                          _window,
                                                          cx| {
                                                        this.toggle_track_like(track.clone(), cx);
                                                    },
                                                ),
                                            ),
                                    )
                                }),
                        ),
                    )
                    .child(
                        progress_bar(progress_ratio, controls_disabled).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                let ratio = progress_click_ratio(event, window);
                                this.seek_to_ratio(ratio, window, cx);
                            }),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(px(theme::SMALL_SIZE))
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(format_clock(progress))
                            .child(
                                total
                                    .map(format_clock)
                                    .unwrap_or_else(|| "--:--".to_string()),
                            ),
                    ),
            )
            .child(div().flex_1())
    }

    fn finish_playback_without_next(&mut self, message: &str, cx: &mut Context<Self>) {
        self.update_playback_state(cx, |state| {
            state.set_playback_status(PlaybackStatus::Idle);
            state.set_resume_position(Duration::ZERO);
        });
        self.status_message = Some(message.to_string());
        self.playback_state
            .read(cx)
            .publish_restored_media_session();
        self.persist_session_snapshot(cx);
        cx.notify();
    }

    fn advance_playback(&mut self, reason: PlaybackAdvanceReason, cx: &mut Context<Self>) {
        match self.playback_advance_action(reason, cx) {
            PlaybackAdvanceAction::Noop => {}
            PlaybackAdvanceAction::RestartCurrent => {
                match self.playback_state.read(cx).restart_current() {
                    Ok(()) => {
                        self.update_playback_state(cx, |state| {
                            state.restart_current_playback();
                        });
                        self.persist_session_snapshot(cx);
                        cx.notify();
                    }
                    Err(error) => {
                        self.status_message = Some(format!("Failed to restart playback: {error}"));
                        self.show_notification(
                            format!("Failed to restart playback: {error}"),
                            super::super::ui::NotificationLevel::Error,
                            cx,
                        );
                        self.handle_playback_failed(cx);
                    }
                }
            }
            PlaybackAdvanceAction::StartIndex { track_list, index } => {
                self.start_playback_for_context(track_list, index, None, cx);
            }
            PlaybackAdvanceAction::Stop(message) => self.finish_playback_without_next(message, cx),
        }
    }

    fn playback_advance_action(
        &self,
        reason: PlaybackAdvanceReason,
        cx: &App,
    ) -> PlaybackAdvanceAction {
        let (track_list, current_index, repeat_mode, elapsed) =
            {
                let playback_state = self.playback_state.read(cx);
                let Some(track_list) = playback_state.playback_context() else {
                    return match reason {
                        PlaybackAdvanceReason::AutomaticFinish => {
                            PlaybackAdvanceAction::Stop("Reached end of track list.")
                        }
                        PlaybackAdvanceReason::ManualNext
                        | PlaybackAdvanceReason::ManualPrevious => PlaybackAdvanceAction::Noop,
                    };
                };
                let Some(current_index) = playback_state.current_track_index() else {
                    return PlaybackAdvanceAction::Noop;
                };
                if track_list.tracks.is_empty() {
                    return match reason {
                        PlaybackAdvanceReason::AutomaticFinish => {
                            PlaybackAdvanceAction::Stop("Reached end of track list.")
                        }
                        PlaybackAdvanceReason::ManualNext
                        | PlaybackAdvanceReason::ManualPrevious => PlaybackAdvanceAction::Noop,
                    };
                }

                (
                    track_list,
                    current_index,
                    playback_state.repeat_mode(),
                    playback_state
                        .position()
                        .unwrap_or(None)
                        .unwrap_or_default(),
                )
            };

        let order = self.playback_order(&track_list, cx);
        let Some(order_position) = order.iter().position(|&index| index == current_index) else {
            return PlaybackAdvanceAction::Noop;
        };

        match reason {
            PlaybackAdvanceReason::AutomaticFinish if matches!(repeat_mode, RepeatMode::One) => {
                PlaybackAdvanceAction::RestartCurrent
            }
            PlaybackAdvanceReason::AutomaticFinish => {
                if let Some(next_index) = order.get(order_position + 1).copied() {
                    PlaybackAdvanceAction::StartIndex {
                        track_list,
                        index: next_index,
                    }
                } else if matches!(repeat_mode, RepeatMode::All) {
                    order
                        .first()
                        .copied()
                        .map(|index| PlaybackAdvanceAction::StartIndex { track_list, index })
                        .unwrap_or(PlaybackAdvanceAction::Stop("Reached end of track list."))
                } else {
                    PlaybackAdvanceAction::Stop("Reached end of track list.")
                }
            }
            PlaybackAdvanceReason::ManualNext => {
                if let Some(next_index) = order.get(order_position + 1).copied() {
                    PlaybackAdvanceAction::StartIndex {
                        track_list,
                        index: next_index,
                    }
                } else if matches!(repeat_mode, RepeatMode::All) {
                    order
                        .first()
                        .copied()
                        .map(|index| PlaybackAdvanceAction::StartIndex { track_list, index })
                        .unwrap_or(PlaybackAdvanceAction::Noop)
                } else {
                    PlaybackAdvanceAction::Noop
                }
            }
            PlaybackAdvanceReason::ManualPrevious if elapsed >= PREVIOUS_RESTART_THRESHOLD => {
                PlaybackAdvanceAction::RestartCurrent
            }
            PlaybackAdvanceReason::ManualPrevious => {
                if let Some(previous_index) = order.get(order_position.wrapping_sub(1)).copied()
                    && order_position > 0
                {
                    PlaybackAdvanceAction::StartIndex {
                        track_list,
                        index: previous_index,
                    }
                } else if matches!(repeat_mode, RepeatMode::All) {
                    order
                        .last()
                        .copied()
                        .map(|index| PlaybackAdvanceAction::StartIndex { track_list, index })
                        .unwrap_or(PlaybackAdvanceAction::RestartCurrent)
                } else {
                    PlaybackAdvanceAction::RestartCurrent
                }
            }
        }
    }

    fn playback_order(&self, track_list: &TrackList, cx: &App) -> Vec<usize> {
        let mut indices = (0..track_list.tracks.len()).collect::<Vec<_>>();
        let shuffle_seed = {
            let playback_state = self.playback_state.read(cx);
            if !playback_state.shuffle_enabled() {
                return indices;
            }
            playback_state.shuffle_seed()
        };

        indices.sort_by_key(|index| {
            let track = &track_list.tracks[*index];
            stable_shuffle_key(shuffle_seed, index, &track.reference.id)
        });
        indices
    }
}

fn now_playing_quality_label(track: &Track) -> Option<String> {
    normalized_audio_quality_label(&AudioQuality {
        audio_format: track.audio_format.clone(),
        bitrate_bps: track.bitrate_bps,
    })
}

fn render_artwork(artwork_path: Option<PathBuf>, size: f32) -> gpui::Div {
    let corner_radius = 10.;
    let frame = div()
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .rounded(px(corner_radius))
        .overflow_hidden()
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .bg(rgb(theme::SURFACE_FLOATING));

    match artwork_path {
        Some(path) => frame.child(
            img(path)
                .w_full()
                .h_full()
                .rounded(px(corner_radius))
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| artwork_fallback().into_any_element()),
        ),
        None => frame.child(artwork_fallback()),
    }
}

fn artwork_fallback() -> gpui::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(theme::SURFACE_FLOATING))
        .child(render_icon_with_color(AppIcon::Music, 26., theme::TEXT_DIM))
}

fn icon_button(icon: AppIcon, emphasized: bool, disabled: bool, active: bool) -> gpui::Div {
    let border = if emphasized || active {
        theme::ACCENT_PRIMARY
    } else {
        theme::BORDER_SUBTLE
    };
    let background = if emphasized {
        theme::ACCENT_PRIMARY_LIGHT
    } else if active {
        theme::SURFACE_FLOATING
    } else {
        theme::SURFACE_BASE
    };
    let icon_color = if emphasized || active {
        theme::ACCENT_PRIMARY
    } else {
        theme::TEXT_MUTED
    };

    div()
        .w(px(if emphasized { 42. } else { 34. }))
        .h(px(if emphasized { 42. } else { 34. }))
        .rounded(px(theme::RADIUS_FULL))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(background))
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.45))
        .flex()
        .items_center()
        .justify_center()
        .child(render_icon_with_color(
            icon,
            if emphasized { 20. } else { 16. },
            icon_color,
        ))
}

fn progress_bar(progress_ratio: f32, disabled: bool) -> gpui::Div {
    div()
        .w_full()
        .h(px(8.))
        .rounded(px(theme::RADIUS_FULL))
        .bg(rgb(theme::ACCENT_PRIMARY_LIGHT))
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.45))
        .child(
            div()
                .w(relative(progress_ratio.clamp(0.0, 1.0)))
                .h_full()
                .rounded(px(theme::RADIUS_FULL))
                .bg(rgb(theme::ACCENT_PRIMARY)),
        )
}

fn like_icon_button(active: bool, disabled: bool) -> gpui::Div {
    let icon_color = if active {
        theme::ACCENT_PRIMARY
    } else {
        theme::TEXT_MUTED
    };

    div()
        .w(px(34.))
        .h(px(34.))
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.45))
        .flex()
        .items_center()
        .justify_center()
        .child(render_icon_with_color(
            if active {
                AppIcon::HeartFilled
            } else {
                AppIcon::Heart
            },
            18.,
            icon_color,
        ))
}

fn progress_click_ratio(event: &MouseDownEvent, window: &Window) -> f32 {
    let window_width = f32::from(window.bounds().size.width);
    let horizontal_chrome = theme::SPACE_4 * 4.0;
    let bar_width = PLAYER_CENTER_WIDTH
        .min((window_width - horizontal_chrome).max(1.0))
        .max(1.0);
    let bar_origin_x = ((window_width - bar_width) * 0.5).max(0.0);
    ((f32::from(event.position.x) - bar_origin_x) / bar_width).clamp(0.0, 1.0)
}

fn stable_shuffle_key(seed: u64, index: &usize, track_id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    index.hash(&mut hasher);
    track_id.hash(&mut hasher);
    hasher.finish()
}

fn next_shuffle_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1)
}
