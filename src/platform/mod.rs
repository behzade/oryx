use std::env;
use std::ffi::c_void;
use std::path::PathBuf;

use anyhow::Result;
use gpui::{App, Modifiers, Window};

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
use linux as native;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(target_os = "windows")]
use windows as native;

pub(crate) use native::{
    IMPORT_FOLDER_KEYBINDING, MINIMIZE_KEYBINDING, QUIT_KEYBINDING,
    REFRESH_LOCAL_ARTWORK_KEYBINDING,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextInputShortcut {
    Copy,
    Cut,
    Paste,
    Backspace,
    BackspaceWord,
    BackspaceToStart,
    Delete,
    Clear,
    MoveLeft { select: bool, by_word: bool },
    MoveRight { select: bool, by_word: bool },
    MoveToStart { select: bool },
    MoveToEnd { select: bool },
    SelectAll,
}

pub(crate) fn setup_app(cx: &mut App) {
    #[cfg(target_os = "macos")]
    native::setup_app(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

pub(crate) fn configure_window(window: &mut Window) {
    #[cfg(target_os = "macos")]
    native::configure_window(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

pub(crate) fn uses_native_window_controls() -> bool {
    native::uses_native_window_controls()
}

pub(crate) fn minimize_window(window: &mut Window) {
    window.minimize_window();
    #[cfg(target_os = "macos")]
    native::minimize_window(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

pub(crate) fn app_root_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("ORYX_ROOT_DIR") {
        return Ok(PathBuf::from(path));
    }
    native::app_root_dir()
}

pub(crate) fn map_text_input_shortcut(
    key: &str,
    modifiers: Modifiers,
) -> Option<TextInputShortcut> {
    native::map_text_input_shortcut(key, modifiers)
}

pub(crate) fn is_primary_shortcut(modifiers: Modifiers) -> bool {
    native::is_primary_shortcut(modifiers)
}

pub(crate) fn media_controls_hwnd(window: &Window) -> Option<*mut c_void> {
    native::media_controls_hwnd(window)
}
