use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

pub(crate) struct EmbeddedAssetSource;

impl EmbeddedAssetSource {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl AssetSource for EmbeddedAssetSource {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(embedded_asset(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let names = match path {
            "" => vec!["icons".into()],
            "icons" => vec!["lucide".into()],
            "icons/lucide" => vec![
                "download.svg".into(),
                "heart.svg".into(),
                "heart-filled.svg".into(),
                "music-4.svg".into(),
                "pause.svg".into(),
                "play.svg".into(),
                "repeat-1.svg".into(),
                "repeat-2.svg".into(),
                "search.svg".into(),
                "shuffle.svg".into(),
                "skip-back.svg".into(),
                "skip-forward.svg".into(),
                "trash.svg".into(),
                "trash-2.svg".into(),
                "x.svg".into(),
            ],
            _ => Vec::new(),
        };
        Ok(names)
    }
}

fn embedded_asset(path: &str) -> Option<&'static [u8]> {
    match path {
        "icons/lucide/search.svg" => Some(include_bytes!("../assets/icons/lucide/search.svg")),
        "icons/lucide/play.svg" => Some(include_bytes!("../assets/icons/lucide/play.svg")),
        "icons/lucide/pause.svg" => Some(include_bytes!("../assets/icons/lucide/pause.svg")),
        "icons/lucide/skip-back.svg" => {
            Some(include_bytes!("../assets/icons/lucide/skip-back.svg"))
        }
        "icons/lucide/skip-forward.svg" => {
            Some(include_bytes!("../assets/icons/lucide/skip-forward.svg"))
        }
        "icons/lucide/shuffle.svg" => Some(include_bytes!("../assets/icons/lucide/shuffle.svg")),
        "icons/lucide/repeat-1.svg" => Some(include_bytes!("../assets/icons/lucide/repeat-1.svg")),
        "icons/lucide/repeat-2.svg" => Some(include_bytes!("../assets/icons/lucide/repeat-2.svg")),
        "icons/lucide/music-4.svg" => Some(include_bytes!("../assets/icons/lucide/music-4.svg")),
        "icons/lucide/heart.svg" => Some(include_bytes!("../assets/icons/lucide/heart.svg")),
        "icons/lucide/heart-filled.svg" => {
            Some(include_bytes!("../assets/icons/lucide/heart-filled.svg"))
        }
        "icons/lucide/download.svg" => Some(include_bytes!("../assets/icons/lucide/download.svg")),
        "icons/lucide/trash.svg" => Some(include_bytes!("../assets/icons/lucide/trash.svg")),
        "icons/lucide/trash-2.svg" => Some(include_bytes!("../assets/icons/lucide/trash-2.svg")),
        "icons/lucide/x.svg" => Some(include_bytes!("../assets/icons/lucide/x.svg")),
        _ => None,
    }
}
