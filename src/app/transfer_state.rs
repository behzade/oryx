use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui::{Context, EventEmitter};

use crate::progressive::ProgressiveDownload;
use crate::provider::TrackSummary;
use crate::transfer::{DownloadPurpose, TransferEvent};

const EXTERNAL_DOWNLOAD_HISTORY_LIMIT: usize = 24;

#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct ActiveTransfer {
    pub(super) track_id: String,
    pub(super) title: String,
    pub(super) purpose: DownloadPurpose,
    pub(super) progress: ProgressiveDownload,
}

#[derive(Clone)]
pub(super) struct DownloadItem {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) purpose: DownloadPurpose,
    pub(super) state: DownloadItemState,
}

#[derive(Clone)]
pub(super) enum DownloadItemState {
    Queued {
        source_url: String,
    },
    Active {
        progress: ProgressiveDownload,
        source_url: Option<String>,
        destination: Option<PathBuf>,
    },
    Completed {
        destination: PathBuf,
    },
    Failed {
        source_url: String,
        destination: Option<PathBuf>,
        error: String,
    },
}

#[derive(Clone)]
struct ExternalDownloadItem {
    id: String,
    title: String,
    state: DownloadItemState,
}

pub(super) struct TransferStateModel {
    downloads: HashMap<String, ActiveTransfer>,
    cancelled_downloads: HashSet<String>,
    external_downloads: Vec<ExternalDownloadItem>,
}

impl EventEmitter<TransferEvent> for TransferStateModel {}

impl TransferStateModel {
    pub(super) fn new() -> Self {
        Self {
            downloads: HashMap::new(),
            cancelled_downloads: HashSet::new(),
            external_downloads: Vec::new(),
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

    pub(super) fn active_download_count(&self) -> usize {
        self.downloads.len()
            + self
                .external_downloads
                .iter()
                .filter(|item| item.is_active())
                .count()
    }

    pub(super) fn download_items(&self) -> Vec<DownloadItem> {
        let mut items = self
            .external_downloads
            .iter()
            .cloned()
            .map(ExternalDownloadItem::into_download_item)
            .collect::<Vec<_>>();

        items.extend(
            self.downloads
                .values()
                .cloned()
                .map(|download| DownloadItem {
                    id: download.track_id.clone(),
                    title: download.title,
                    purpose: download.purpose,
                    state: DownloadItemState::Active {
                        progress: download.progress,
                        source_url: None,
                        destination: None,
                    },
                }),
        );

        items.sort_by(|left, right| {
            right
                .is_active()
                .cmp(&left.is_active())
                .then_with(|| left.title.cmp(&right.title))
        });
        items
    }

    pub(super) fn has_active_downloads(&self) -> bool {
        !self.downloads.is_empty()
            || self
                .external_downloads
                .iter()
                .any(ExternalDownloadItem::is_active)
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
            TransferEvent::ExternalDownloadQueued {
                download_id,
                title,
                source_url,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Queued {
                        source_url: source_url.clone(),
                    },
                });
                changed = true;
            }
            TransferEvent::ExternalDownloadStarted {
                download_id,
                title,
                source_url,
                destination,
                progress,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Active {
                        progress: progress.clone(),
                        source_url: Some(source_url.clone()),
                        destination: Some(destination.clone()),
                    },
                });
                changed = true;
            }
            TransferEvent::ExternalDownloadCompleted {
                download_id,
                title,
                destination,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Completed {
                        destination: destination.clone(),
                    },
                });
                changed = true;
            }
            TransferEvent::ExternalDownloadCancelled { download_id } => {
                self.external_downloads
                    .retain(|item| item.id != *download_id);
                changed = true;
            }
            TransferEvent::ExternalDownloadFailed {
                download_id,
                title,
                source_url,
                destination,
                error,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Failed {
                        source_url: source_url.clone(),
                        destination: destination.clone(),
                        error: error.clone(),
                    },
                });
                changed = true;
            }
            TransferEvent::PlaybackReady { .. } | TransferEvent::PlaybackFailed { .. } => {}
        }

        cx.emit(event);
        if changed {
            cx.notify();
        }
    }

    fn upsert_external_download(&mut self, item: ExternalDownloadItem) {
        self.external_downloads
            .retain(|existing| existing.id != item.id);
        self.external_downloads.insert(0, item);
        self.external_downloads
            .truncate(EXTERNAL_DOWNLOAD_HISTORY_LIMIT);
    }
}

impl DownloadItem {
    pub(super) fn is_active(&self) -> bool {
        matches!(
            self.state,
            DownloadItemState::Queued { .. } | DownloadItemState::Active { .. }
        )
    }
}

impl ExternalDownloadItem {
    fn into_download_item(self) -> DownloadItem {
        DownloadItem {
            id: self.id,
            title: self.title,
            purpose: DownloadPurpose::ExternalUrl,
            state: self.state,
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            self.state,
            DownloadItemState::Queued { .. } | DownloadItemState::Active { .. }
        )
    }
}

fn track_cache_key(track: &TrackSummary) -> String {
    format!(
        "{}:{}",
        track.reference.provider.as_str(),
        track.reference.id
    )
}
