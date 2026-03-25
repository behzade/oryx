use gpui::{App, ClipboardItem, Context, KeyDownEvent, Window};
use std::time::Duration;

use crate::model::PlaybackStatus;
use crate::platform::{self, TextInputShortcut};

use super::{OryxApp, playback::PlaybackIntent, text_input::TextInputId};

enum AppIntent {
    StartSearch,
    TextInput(TextInputId, TextInputShortcut),
    Playback(PlaybackIntent),
}

const KEYBOARD_SEEK_STEP: Duration = Duration::from_secs(10);

impl OryxApp {
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused_input = self.focused_text_input(window);
        if self.ui_state.read(cx).provider_link_prompt().is_some() {
            self.handle_provider_link_key_down(event, focused_input, cx);
            return;
        }
        if self.ui_state.read(cx).provider_auth_prompt().is_some() {
            self.handle_provider_auth_key_down(event, focused_input, window, cx);
            return;
        }

        let Some(intent) = Self::key_down_intent(event, focused_input) else {
            return;
        };

        self.dispatch_app_intent(intent, cx);
    }

    fn handle_provider_auth_key_down(
        &mut self,
        event: &KeyDownEvent,
        focused_input: Option<TextInputId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if let Some(input_id) = focused_input
            .clone()
            .filter(|input_id| input_id.is_provider_auth())
            && let Some(shortcut) =
                platform::map_text_input_shortcut(event.keystroke.key.as_str(), modifiers)
        {
            self.dispatch_text_input_command(input_id, shortcut, cx);
            return;
        }

        if modifiers.function {
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" => self.close_provider_auth_prompt(cx),
            "tab" => {
                let target = match (focused_input, modifiers.shift) {
                    (Some(TextInputId::ProviderAuthPassword), true) => {
                        TextInputId::ProviderAuthUsername
                    }
                    (Some(TextInputId::ProviderAuthUsername), false) => {
                        TextInputId::ProviderAuthPassword
                    }
                    (_, true) => TextInputId::ProviderAuthPassword,
                    _ => TextInputId::ProviderAuthUsername,
                };
                self.focus_text_input(&target, window);
                cx.notify();
            }
            "enter" => {
                if focused_input == Some(TextInputId::ProviderAuthPassword) {
                    self.submit_provider_auth(cx);
                } else {
                    self.focus_text_input(&TextInputId::ProviderAuthPassword, window);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn handle_provider_link_key_down(
        &mut self,
        event: &KeyDownEvent,
        focused_input: Option<TextInputId>,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if focused_input == Some(TextInputId::ProviderLink)
            && let Some(shortcut) =
                platform::map_text_input_shortcut(event.keystroke.key.as_str(), modifiers)
        {
            self.dispatch_text_input_command(TextInputId::ProviderLink, shortcut, cx);
            return;
        }

        if modifiers.function {
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" => self.close_provider_link_prompt(cx),
            "enter" => self.submit_provider_link_prompt(cx),
            _ => {}
        }
    }

    pub(super) fn dispatch_playback_intent(
        &mut self,
        intent: PlaybackIntent,
        cx: &mut Context<Self>,
    ) {
        match intent {
            PlaybackIntent::Pause => self.pause_playback(cx),
            PlaybackIntent::Play => self.play_playback(cx),
            PlaybackIntent::Toggle => self.toggle_playback(cx),
            PlaybackIntent::Stop => self.stop_playback(cx),
            PlaybackIntent::Next => self.play_next(cx),
            PlaybackIntent::Previous => self.play_previous(cx),
            PlaybackIntent::SeekBackward(delta) => self.seek_backward_by(delta, cx),
            PlaybackIntent::SeekForward(delta) => self.seek_forward_by(delta, cx),
            PlaybackIntent::SeekTo(target) => self.seek_to(target, cx),
        }
    }

    fn dispatch_app_intent(&mut self, intent: AppIntent, cx: &mut Context<Self>) {
        match intent {
            AppIntent::StartSearch => self.start_search(cx),
            AppIntent::TextInput(input_id, command) => {
                self.dispatch_text_input_command(input_id, command, cx)
            }
            AppIntent::Playback(intent) => self.dispatch_playback_intent(intent, cx),
        }
    }

    fn dispatch_text_input_command(
        &mut self,
        input_id: TextInputId,
        command: TextInputShortcut,
        cx: &mut Context<Self>,
    ) {
        let previous_text = self.text_input(&input_id).content().to_string();
        let previous_cursor = self.text_input(&input_id).cursor();
        let previous_selection = self.text_input(&input_id).selection_anchor();

        match command {
            TextInputShortcut::Copy => {
                let Some(range) = self.text_input(&input_id).selection_range() else {
                    return;
                };
                cx.write_to_clipboard(ClipboardItem::new_string(
                    self.text_input(&input_id).content()[range].to_string(),
                ));
                return;
            }
            TextInputShortcut::Cut => {
                let Some(range) = self.text_input(&input_id).selection_range() else {
                    return;
                };
                let selected_text = self.text_input(&input_id).content()[range.clone()].to_string();
                cx.write_to_clipboard(ClipboardItem::new_string(selected_text));
                self.text_input_mut(&input_id).replace_range(range, "");
            }
            TextInputShortcut::Paste => {
                let Some(text) = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .map(|text| text.replace(['\r', '\n'], ""))
                else {
                    return;
                };
                self.text_input_mut(&input_id)
                    .replace_text_in_range_utf16(None, &text);
            }
            TextInputShortcut::Backspace => self.text_input_mut(&input_id).delete_backward(),
            TextInputShortcut::BackspaceWord => {
                self.text_input_mut(&input_id).delete_word_backward()
            }
            TextInputShortcut::BackspaceToStart => self.text_input_mut(&input_id).delete_to_start(),
            TextInputShortcut::Delete => self.text_input_mut(&input_id).delete_forward(),
            TextInputShortcut::Clear => self.text_input_mut(&input_id).clear(),
            TextInputShortcut::MoveLeft { select, by_word } => {
                self.text_input_mut(&input_id).move_left(select, by_word)
            }
            TextInputShortcut::MoveRight { select, by_word } => {
                self.text_input_mut(&input_id).move_right(select, by_word)
            }
            TextInputShortcut::MoveToStart { select } => {
                self.text_input_mut(&input_id).move_to_start(select)
            }
            TextInputShortcut::MoveToEnd { select } => {
                self.text_input_mut(&input_id).move_to_end(select)
            }
            TextInputShortcut::SelectAll => self.text_input_mut(&input_id).select_all(),
        }

        let text_changed = self.text_input(&input_id).content() != previous_text;
        let selection_changed = self.text_input(&input_id).cursor() != previous_cursor
            || self.text_input(&input_id).selection_anchor() != previous_selection;

        if text_changed {
            self.handle_text_input_edited(input_id, cx);
        }

        if text_changed || selection_changed {
            cx.notify();
        }
    }

    fn pause_playback(&mut self, cx: &mut Context<Self>) {
        match self.playback_state.read(cx).pause() {
            Ok(()) => {
                self.sync_paused_playback_state("Paused.", cx);
            }
            Err(error) => {
                self.status_message = Some(format!("Pause failed: {error}"));
            }
        }

        cx.notify();
    }

    fn play_playback(&mut self, cx: &mut Context<Self>) {
        if self.should_restart_current_track_on_play(cx) {
            self.resume_current_track(cx);
            return;
        }

        match self.playback_state.read(cx).resume() {
            Ok(()) => {
                self.update_playback_state(cx, |state| {
                    state.set_playback_status(PlaybackStatus::Playing);
                });
                self.status_message = Some("Playing.".to_string());
                self.persist_session_snapshot(cx);
                cx.notify();
            }
            Err(error) => {
                self.status_message = Some(format!("Resume failed: {error}"));
                cx.notify();
            }
        }
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.playback_state.read(cx).play_loading() {
            return;
        }

        match self.playback_state.read(cx).playback_status() {
            PlaybackStatus::Playing | PlaybackStatus::Buffering => self.pause_playback(cx),
            PlaybackStatus::Paused | PlaybackStatus::Idle => self.play_playback(cx),
        }
    }

    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        match self.playback_state.read(cx).stop() {
            Ok(()) => {
                self.capture_resume_position(cx);
                self.update_playback_state(cx, |state| {
                    state.set_playback_status(PlaybackStatus::Idle);
                });
                self.status_message = Some("Stopped.".to_string());
                self.persist_session_snapshot(cx);
            }
            Err(error) => {
                self.status_message = Some(format!("Stop failed: {error}"));
            }
        }

        cx.notify();
    }

    fn sync_paused_playback_state(&mut self, message: &str, cx: &mut Context<Self>) {
        self.capture_resume_position(cx);
        self.update_playback_state(cx, |state| {
            state.set_playback_status(PlaybackStatus::Paused);
        });
        self.status_message = Some(message.to_string());
        self.persist_session_snapshot(cx);
    }

    fn capture_resume_position(&mut self, cx: &mut Context<Self>) {
        let position = self
            .playback_state
            .read(cx)
            .position()
            .unwrap_or(None)
            .unwrap_or_default();
        self.update_playback_state(cx, |state| {
            state.set_resume_position(position);
        });
    }

    fn should_restart_current_track_on_play(&self, cx: &App) -> bool {
        let playback_state = self.playback_state.read(cx);
        matches!(playback_state.playback_status(), PlaybackStatus::Idle)
            || (matches!(playback_state.playback_status(), PlaybackStatus::Paused)
                && self
                    .playback_state
                    .read(cx)
                    .position()
                    .ok()
                    .flatten()
                    .is_none()
                && playback_state.current_track_index().is_some())
    }

    fn key_down_intent(
        event: &KeyDownEvent,
        focused_input: Option<TextInputId>,
    ) -> Option<AppIntent> {
        let modifiers = event.keystroke.modifiers;

        let Some(input_id) = focused_input else {
            return match event.keystroke.key.as_str() {
                "space"
                    if !modifiers.control
                        && !modifiers.function
                        && !modifiers.platform
                        && !modifiers.alt
                        && !modifiers.shift =>
                {
                    Some(AppIntent::Playback(PlaybackIntent::Toggle))
                }
                "left"
                    if !modifiers.control
                        && !modifiers.function
                        && !modifiers.platform
                        && !modifiers.alt
                        && !modifiers.shift =>
                {
                    Some(AppIntent::Playback(PlaybackIntent::SeekBackward(
                        KEYBOARD_SEEK_STEP,
                    )))
                }
                "right"
                    if !modifiers.control
                        && !modifiers.function
                        && !modifiers.platform
                        && !modifiers.alt
                        && !modifiers.shift =>
                {
                    Some(AppIntent::Playback(PlaybackIntent::SeekForward(
                        KEYBOARD_SEEK_STEP,
                    )))
                }
                _ => None,
            };
        };

        if input_id == TextInputId::Query
            && event.keystroke.key == "enter"
            && (!modifiers.modified()
                || modifiers.shift
                || platform::is_primary_shortcut(modifiers))
        {
            return Some(AppIntent::StartSearch);
        }

        platform::map_text_input_shortcut(event.keystroke.key.as_str(), modifiers)
            .map(|shortcut| AppIntent::TextInput(input_id, shortcut))
    }
}
