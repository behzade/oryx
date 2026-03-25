use gpui::{App, KeyBinding, actions};

use crate::platform;

actions!(
    oryx_actions,
    [
        Quit,
        ImportFolder,
        ImportProviderLink,
        ExportProviderLink,
        RefreshLocalArtwork,
        TogglePlayback,
        PlayNextTrack,
        PlayPreviousTrack,
        MinimizeWindow
    ]
);

pub const APP_KEY_CONTEXT: &str = "OryxApp";

pub fn bind(cx: &mut App) {
    cx.bind_keys(vec![
        KeyBinding::new(
            platform::IMPORT_FOLDER_KEYBINDING,
            ImportFolder,
            Some(APP_KEY_CONTEXT),
        ),
        KeyBinding::new(platform::QUIT_KEYBINDING, Quit, Some(APP_KEY_CONTEXT)),
        KeyBinding::new(
            platform::MINIMIZE_KEYBINDING,
            MinimizeWindow,
            Some(APP_KEY_CONTEXT),
        ),
        KeyBinding::new(
            platform::REFRESH_LOCAL_ARTWORK_KEYBINDING,
            RefreshLocalArtwork,
            Some(APP_KEY_CONTEXT),
        ),
    ]);
}
