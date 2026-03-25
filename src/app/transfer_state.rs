use std::collections::{HashMap, HashSet};

use gpui::{Context, EventEmitter};

use crate::progressive::ProgressiveDownload;
use crate::provider::TrackSummary;
use crate::transfer::{DownloadPurpose, TransferEvent};

#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct ActiveTransfer {
    pub(super) track_id: String,
    pub(super) title: String,
    pub(super) purpose: DownloadPurpose,
    pub(super) progress: ProgressiveDownload,
}

pub(super) struct TransferStateModel {
    downloads: HashMap<String, ActiveTransfer>,
    cancelled_downloads: HashSet<String>,
}

impl EventEmitter<TransferEvent> for TransferStateModel {}

impl TransferStateModel {
    pub(super) fn new() -> Self {
        Self {
            downloads: HashMap::new(),
            cancelled_downloads: HashSet::new(),
        }
    }

    pub(super) fn start_download(
        &mut self,
        track_id: String,
        title: String,
        purpose: DownloadPurpose,
        progress: ProgressiveDownload,
    ) {
        self.cancelled_downloads.remove(&track_id);
        self.downloads.insert(
            track_id.clone(),
            ActiveTransfer {
                track_id,
                title,
                purpose,
                progress,
            },
        );
    }

    pub(super) fn finish_download(&mut self, track_id: &str) {
        self.downloads.remove(track_id);
        self.cancelled_downloads.remove(track_id);
    }

    pub(super) fn track_is_downloading(&self, track: &TrackSummary) -> bool {
        self.downloads.contains_key(&track_cache_key(track))
    }

    pub(super) fn active_download(&self, track: &TrackSummary) -> Option<ActiveTransfer> {
        self.downloads.get(&track_cache_key(track)).cloned()
    }

    pub(super) fn cancel_explicit_download(&mut self, track: &TrackSummary) -> bool {
        let key = track_cache_key(track);
        let Some(download) = self.downloads.get(&key) else {
            return false;
        };
        if !matches!(download.purpose, DownloadPurpose::Explicit) {
            return false;
        }

        download.progress.cancel();
        self.downloads.remove(&key);
        self.cancelled_downloads.insert(key);
        true
    }

    pub(super) fn was_cancelled(&self, track_id: &str) -> bool {
        self.cancelled_downloads.contains(track_id)
    }

    #[allow(dead_code)]
    pub(super) fn active_downloads(&self) -> Vec<ActiveTransfer> {
        self.downloads.values().cloned().collect()
    }

    pub(super) fn has_active_downloads(&self) -> bool {
        !self.downloads.is_empty()
    }

    pub(super) fn handle_worker_event(&mut self, event: TransferEvent, cx: &mut Context<Self>) {
        let mut changed = false;
        match &event {
            TransferEvent::DownloadStarted {
                track_id,
                title,
                purpose,
                progress,
            } => {
                self.start_download(track_id.clone(), title.clone(), *purpose, progress.clone());
                changed = true;
            }
            TransferEvent::DownloadCompleted { track_id, .. }
            | TransferEvent::DownloadCancelled { track_id } => {
                self.finish_download(track_id);
                changed = true;
            }
            TransferEvent::DownloadFailed { track_id, .. } => {
                self.finish_download(track_id);
                changed = true;
            }
            TransferEvent::PlaybackReady { .. } | TransferEvent::PlaybackFailed { .. } => {}
        }

        cx.emit(event);
        if changed {
            cx.notify();
        }
    }
}

fn track_cache_key(track: &TrackSummary) -> String {
    format!(
        "{}:{}",
        track.reference.provider.as_str(),
        track.reference.id
    )
}
