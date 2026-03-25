use gpui::{Context, Div, InteractiveElement};

use crate::keybindings::{
    ExportProviderLink, ImportFolder, ImportProviderLink, MinimizeWindow, PlayNextTrack,
    PlayPreviousTrack, Quit, RefreshLocalArtwork, TogglePlayback,
};
use crate::platform;

use super::super::OryxApp;

impl OryxApp {
    pub(super) fn bind_shell_actions(&self, shell: Div, cx: &mut Context<Self>) -> Div {
        shell
            .on_action(cx.listener(|this, _action: &Quit, _window, cx| {
                this.quit_app(cx);
            }))
            .on_action(cx.listener(|this, _action: &TogglePlayback, _window, cx| {
                this.toggle_playback_from_ui(cx);
            }))
            .on_action(cx.listener(|this, _action: &PlayNextTrack, _window, cx| {
                this.play_next(cx);
            }))
            .on_action(
                cx.listener(|this, _action: &PlayPreviousTrack, _window, cx| {
                    this.play_previous(cx);
                }),
            )
            .on_action(cx.listener(|_this, _action: &MinimizeWindow, window, _cx| {
                platform::minimize_window(window);
            }))
            .on_action(cx.listener(|this, _action: &ImportFolder, window, cx| {
                this.prompt_for_import_folder(window, cx);
            }))
            .on_action(
                cx.listener(|this, _action: &ImportProviderLink, window, cx| {
                    this.open_provider_link_prompt(
                        super::super::ui::ProviderLinkPromptMode::Import,
                        window,
                        cx,
                    );
                }),
            )
            .on_action(
                cx.listener(|this, _action: &ExportProviderLink, window, cx| {
                    this.open_provider_link_prompt(
                        super::super::ui::ProviderLinkPromptMode::Export,
                        window,
                        cx,
                    );
                }),
            )
            .on_action(
                cx.listener(|this, _action: &RefreshLocalArtwork, window, cx| {
                    this.refresh_local_artwork(window, cx);
                }),
            )
            .on_key_down(cx.listener(Self::handle_key_down))
    }
}
