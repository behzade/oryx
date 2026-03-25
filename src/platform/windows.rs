use std::ffi::c_void;
use std::path::PathBuf;

use anyhow::{Context, Result};
use gpui::{Modifiers, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::TextInputShortcut;

pub(crate) const IMPORT_FOLDER_KEYBINDING: &str = "ctrl-o";
pub(crate) const QUIT_KEYBINDING: &str = "ctrl-q";
pub(crate) const MINIMIZE_KEYBINDING: &str = "ctrl-m";
pub(crate) const REFRESH_LOCAL_ARTWORK_KEYBINDING: &str = "ctrl-shift-r";

pub(super) fn uses_native_window_controls() -> bool {
    false
}

pub(super) fn app_root_dir() -> Result<PathBuf> {
    let root = dirs::data_local_dir()
        .or_else(dirs::audio_dir)
        .context("No local data directory is available")?;
    Ok(root.join("Oryx"))
}

pub(super) fn map_text_input_shortcut(
    key: &str,
    modifiers: Modifiers,
) -> Option<TextInputShortcut> {
    if is_primary_shortcut(modifiers) {
        return match key {
            "a" => Some(TextInputShortcut::SelectAll),
            "c" => Some(TextInputShortcut::Copy),
            "x" => Some(TextInputShortcut::Cut),
            "v" => Some(TextInputShortcut::Paste),
            "backspace" => Some(TextInputShortcut::BackspaceWord),
            _ => None,
        };
    }

    match key {
        "home"
            if !modifiers.control
                && !modifiers.alt
                && !modifiers.function
                && !modifiers.platform =>
        {
            Some(TextInputShortcut::MoveToStart {
                select: modifiers.shift,
            })
        }
        "end"
            if !modifiers.control
                && !modifiers.alt
                && !modifiers.function
                && !modifiers.platform =>
        {
            Some(TextInputShortcut::MoveToEnd {
                select: modifiers.shift,
            })
        }
        "left" if is_word_navigation_shortcut(modifiers) => Some(TextInputShortcut::MoveLeft {
            select: modifiers.shift,
            by_word: true,
        }),
        "right" if is_word_navigation_shortcut(modifiers) => Some(TextInputShortcut::MoveRight {
            select: modifiers.shift,
            by_word: true,
        }),
        "left"
            if !modifiers.control
                && !modifiers.alt
                && !modifiers.function
                && !modifiers.platform =>
        {
            Some(TextInputShortcut::MoveLeft {
                select: modifiers.shift,
                by_word: false,
            })
        }
        "right"
            if !modifiers.control
                && !modifiers.alt
                && !modifiers.function
                && !modifiers.platform =>
        {
            Some(TextInputShortcut::MoveRight {
                select: modifiers.shift,
                by_word: false,
            })
        }
        "backspace"
            if !modifiers.control
                && !modifiers.alt
                && !modifiers.function
                && !modifiers.platform
                && !modifiers.shift =>
        {
            Some(TextInputShortcut::Backspace)
        }
        "delete" if !modifiers.modified() || modifiers.shift => Some(TextInputShortcut::Delete),
        "escape" if !modifiers.modified() => Some(TextInputShortcut::Clear),
        _ => None,
    }
}

pub(super) fn media_controls_hwnd(window: &Window) -> Option<*mut c_void> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut c_void),
        _ => None,
    }
}

pub(super) fn is_primary_shortcut(modifiers: Modifiers) -> bool {
    modifiers.control
        && !modifiers.alt
        && !modifiers.function
        && !modifiers.platform
        && !modifiers.shift
}

fn is_word_navigation_shortcut(modifiers: Modifiers) -> bool {
    modifiers.control && !modifiers.alt && !modifiers.function && !modifiers.platform
}
