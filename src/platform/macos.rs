use std::ffi::c_void;
use std::path::PathBuf;

use anyhow::{Context, Result};
use gpui::{App, Menu, MenuItem, Modifiers, SystemMenuType, Window};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApp, NSApplication, NSImage, NSWindowStyleMask};
use objc2_foundation::{NSData, NSProcessInfo, NSSize, NSString};

use crate::keybindings::{
    ExportProviderLink, ImportFolder, ImportProviderLink, MinimizeWindow, OpenUrl, PlayNextTrack,
    PlayPreviousTrack, Quit, RefreshLocalArtwork, TogglePlayback,
};

use super::TextInputShortcut;

pub(crate) const IMPORT_FOLDER_KEYBINDING: &str = "cmd-o";
pub(crate) const OPEN_URL_KEYBINDING: &str = "cmd-shift-o";
pub(crate) const QUIT_KEYBINDING: &str = "cmd-q";
pub(crate) const MINIMIZE_KEYBINDING: &str = "cmd-m";
pub(crate) const REFRESH_LOCAL_ARTWORK_KEYBINDING: &str = "cmd-shift-r";

const APP_ICON_BYTES: &[u8] = include_bytes!("../../assets/icons/oryx.png");
const MACOS_RUNTIME_ICON_BYTES: [(&[u8], f64); 10] = [
    (
        include_bytes!("../../assets/icons/app/icon_512x512@2x.png"),
        512.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_512x512.png"),
        512.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_256x256@2x.png"),
        256.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_256x256.png"),
        256.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_128x128@2x.png"),
        128.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_128x128.png"),
        128.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_32x32@2x.png"),
        32.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_32x32.png"),
        32.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_16x16@2x.png"),
        16.0,
    ),
    (
        include_bytes!("../../assets/icons/app/icon_16x16.png"),
        16.0,
    ),
];

pub(super) fn setup_app(cx: &mut App) {
    set_native_app_icon();
    setup_native_main_menu(cx);
}

pub(super) fn configure_window(_: &mut Window) {
    enable_key_window_minimize();
}

pub(super) fn uses_native_window_controls() -> bool {
    true
}

pub(super) fn minimize_window(_: &mut Window) {
    minimize_key_window();
}

pub(super) fn app_root_dir() -> Result<PathBuf> {
    let root = dirs::audio_dir()
        .or_else(dirs::data_local_dir)
        .context("No application data directory is available")?;
    Ok(root.join("oryx"))
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
            "backspace" => Some(TextInputShortcut::BackspaceToStart),
            "left" => Some(TextInputShortcut::MoveToStart {
                select: modifiers.shift,
            }),
            "right" => Some(TextInputShortcut::MoveToEnd {
                select: modifiers.shift,
            }),
            _ => None,
        };
    }

    match key {
        "left" if is_word_navigation_shortcut(modifiers) => Some(TextInputShortcut::MoveLeft {
            select: modifiers.shift,
            by_word: true,
        }),
        "right" if is_word_navigation_shortcut(modifiers) => Some(TextInputShortcut::MoveRight {
            select: modifiers.shift,
            by_word: true,
        }),
        "backspace" if is_word_deletion_shortcut(modifiers) => {
            Some(TextInputShortcut::BackspaceWord)
        }
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

pub(super) fn media_controls_hwnd(_: &Window) -> Option<*mut c_void> {
    None
}

pub(super) fn is_primary_shortcut(modifiers: Modifiers) -> bool {
    modifiers.platform
        && !modifiers.control
        && !modifiers.alt
        && !modifiers.function
        && !modifiers.shift
}

fn is_word_navigation_shortcut(modifiers: Modifiers) -> bool {
    modifiers.alt && !modifiers.control && !modifiers.function && !modifiers.platform
}

fn is_word_deletion_shortcut(modifiers: Modifiers) -> bool {
    modifiers.alt
        && !modifiers.control
        && !modifiers.function
        && !modifiers.platform
        && !modifiers.shift
}

fn set_native_app_icon() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    let process_name = NSString::from_str("Oryx");
    NSProcessInfo::processInfo().setProcessName(&process_name);

    let mut image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(512.0, 512.0));

    for (icon_bytes, logical_size) in MACOS_RUNTIME_ICON_BYTES {
        let icon_data = NSData::with_bytes(icon_bytes);
        let Some(icon_image) = NSImage::initWithData(NSImage::alloc(), &icon_data) else {
            continue;
        };

        let logical_size = NSSize::new(logical_size, logical_size);
        for representation in icon_image.representations().iter() {
            representation.setSize(logical_size);
            image.addRepresentation(&representation);
        }
    }

    if image.representations().count() == 0 {
        let icon_data = NSData::with_bytes(APP_ICON_BYTES);
        let Some(fallback) = NSImage::initWithData(NSImage::alloc(), &icon_data) else {
            return;
        };
        image = fallback;
    }

    let app: Retained<NSApplication> = NSApp(mtm);
    unsafe {
        app.setApplicationIconImage(Some(&image));
    }
}

fn setup_native_main_menu(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "Oryx".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Oryx", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open URL...", OpenUrl),
                MenuItem::action("Import...", ImportFolder),
                MenuItem::action("Import Provider Link...", ImportProviderLink),
                MenuItem::action("Export Provider Link...", ExportProviderLink),
                MenuItem::action("Refresh Local Artwork", RefreshLocalArtwork),
            ],
        },
        Menu {
            name: "Playback".into(),
            items: vec![
                MenuItem::action("Play/Pause", TogglePlayback),
                MenuItem::action("Previous Track", PlayPreviousTrack),
                MenuItem::action("Next Track", PlayNextTrack),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Minimize", MinimizeWindow)],
        },
    ]);
}

fn enable_key_window_minimize() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    let app: Retained<NSApplication> = NSApp(mtm);
    let Some(window) = app.keyWindow().or_else(|| app.mainWindow()) else {
        return;
    };

    let mut style_mask = window.styleMask();
    style_mask |= NSWindowStyleMask::Resizable;
    style_mask |= NSWindowStyleMask::Miniaturizable;
    window.setStyleMask(style_mask);
}

fn minimize_key_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    let app: Retained<NSApplication> = NSApp(mtm);
    let Some(window) = app.keyWindow().or_else(|| app.mainWindow()) else {
        return;
    };

    window.miniaturize(None);
}
