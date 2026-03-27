use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use gpui::{Context, EventEmitter};

use crate::library::{PersistedExternalDownload, PersistedExternalDownloadState};
use crate::progressive::ProgressiveDownload;
use crate::provider::TrackSummary;
use crate::transfer::{DownloadPurpose, TransferEvent};

const EXTERNAL_DOWNLOAD_HISTORY_LIMIT: usize = 200;

#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct ActiveTransfer {
    pub(super) track_id: String,
    pub(super) title: String,
    pub(super) purpose: DownloadPurpose,
    pub(super) progress: ProgressiveDownload,
    pub(super) started_at: Instant,
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
        progress: ProgressiveDownload,
    },
    Active {
        progress: ProgressiveDownload,
        source_url: Option<String>,
        destination: Option<PathBuf>,
        started_at: Instant,
        duration_seconds: Option<u64>,
    },
    Completed {
        source_url: String,
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
                started_at: Instant::now(),
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

    pub(super) fn download_items(&self) -> Vec<DownloadItem> {
        let mut items = self.downloads.values().cloned().collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| left.title.cmp(&right.title))
        });
        let mut items = items
            .into_iter()
            .map(|download| DownloadItem {
                id: download.track_id.clone(),
                title: download.title,
                purpose: download.purpose,
                state: DownloadItemState::Active {
                    progress: download.progress,
                    source_url: None,
                    destination: None,
                    started_at: download.started_at,
                    duration_seconds: None,
                },
            })
            .collect::<Vec<_>>();

        items.extend(
            self.external_downloads
                .iter()
                .cloned()
                .map(ExternalDownloadItem::into_download_item),
        );
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
                progress,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Queued {
                        source_url: source_url.clone(),
                        progress: progress.clone(),
                    },
                });
                changed = true;
            }
            TransferEvent::ExternalDownloadStarted {
                download_id,
                title,
                source_url,
                destination,
                duration_seconds,
                progress,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Active {
                        progress: progress.clone(),
                        source_url: Some(source_url.clone()),
                        destination: Some(destination.clone()),
                        started_at: Instant::now(),
                        duration_seconds: *duration_seconds,
                    },
                });
                changed = true;
            }
            TransferEvent::ExternalDownloadCompleted {
                download_id,
                title,
                source_url,
                destination,
            } => {
                self.upsert_external_download(ExternalDownloadItem {
                    id: download_id.clone(),
                    title: title.clone(),
                    state: DownloadItemState::Completed {
                        source_url: source_url.clone(),
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

    pub(super) fn cancel_external_download(&mut self, download_id: &str) -> bool {
        let Some(index) = self
            .external_downloads
            .iter()
            .position(|item| item.id == download_id && item.is_active())
        else {
            return false;
        };

        match &self.external_downloads[index].state {
            DownloadItemState::Queued { progress, .. }
            | DownloadItemState::Active { progress, .. } => {
                progress.cancel();
            }
            DownloadItemState::Completed { .. } | DownloadItemState::Failed { .. } => {}
        }
        self.external_downloads.remove(index);
        true
    }

    pub(super) fn pause_external_download(&mut self, download_id: &str) -> bool {
        let Some(item) = self
            .external_downloads
            .iter()
            .find(|item| item.id == download_id && item.is_active())
        else {
            return false;
        };

        item.progress()
            .expect("active external download should have progress")
            .pause();
        true
    }

    pub(super) fn resume_external_download(&mut self, download_id: &str) -> bool {
        let Some(item) = self
            .external_downloads
            .iter()
            .find(|item| item.id == download_id && item.is_active())
        else {
            return false;
        };

        item.progress()
            .expect("active external download should have progress")
            .resume();
        true
    }

    pub(super) fn external_download_for_url(&self, source_url: &str) -> Option<DownloadItem> {
        self.external_downloads
            .iter()
            .find(|item| item.source_url() == Some(source_url))
            .cloned()
            .map(ExternalDownloadItem::into_download_item)
    }

    pub(super) fn restore_persisted_external_downloads(
        &mut self,
        downloads: Vec<PersistedExternalDownload>,
    ) {
        self.external_downloads = downloads
            .into_iter()
            .filter_map(|download| match download.state {
                PersistedExternalDownloadState::Completed { destination } => {
                    Some(ExternalDownloadItem {
                        id: download.id,
                        title: download.title,
                        state: DownloadItemState::Completed {
                            source_url: download.source_url,
                            destination,
                        },
                    })
                }
                PersistedExternalDownloadState::Failed { destination, error } => {
                    Some(ExternalDownloadItem {
                        id: download.id,
                        title: download.title,
                        state: DownloadItemState::Failed {
                            source_url: download.source_url,
                            destination,
                            error,
                        },
                    })
                }
                PersistedExternalDownloadState::Pending => None,
            })
            .collect();
        self.external_downloads
            .truncate(EXTERNAL_DOWNLOAD_HISTORY_LIMIT);
    }

    pub(super) fn persisted_external_downloads(&self) -> Vec<PersistedExternalDownload> {
        self.external_downloads
            .iter()
            .map(|item| {
                let snapshot = item.progress().map(ProgressiveDownload::snapshot);
                PersistedExternalDownload {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    source_url: item
                        .source_url()
                        .expect("external download should always retain its source URL")
                        .to_string(),
                    destination: item.destination().cloned(),
                    paused: snapshot.is_some_and(|snapshot| snapshot.paused),
                    downloaded_bytes: snapshot
                        .map(|snapshot| snapshot.downloaded_bytes)
                        .filter(|bytes| *bytes > 0),
                    total_bytes: snapshot.and_then(|snapshot| snapshot.total_bytes),
                    state: match &item.state {
                        DownloadItemState::Queued { .. } | DownloadItemState::Active { .. } => {
                            PersistedExternalDownloadState::Pending
                        }
                        DownloadItemState::Completed {
                            source_url: _,
                            destination,
                        } => PersistedExternalDownloadState::Completed {
                            destination: destination.clone(),
                        },
                        DownloadItemState::Failed {
                            destination, error, ..
                        } => PersistedExternalDownloadState::Failed {
                            destination: destination.clone(),
                            error: error.clone(),
                        },
                    },
                }
            })
            .collect()
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

    fn source_url(&self) -> Option<&str> {
        match &self.state {
            DownloadItemState::Queued { source_url, .. } => Some(source_url),
            DownloadItemState::Active { source_url, .. } => source_url.as_deref(),
            DownloadItemState::Completed { source_url, .. } => Some(source_url),
            DownloadItemState::Failed { source_url, .. } => Some(source_url),
        }
    }

    fn progress(&self) -> Option<&ProgressiveDownload> {
        match &self.state {
            DownloadItemState::Queued { progress, .. } => Some(progress),
            DownloadItemState::Active { progress, .. } => Some(progress),
            DownloadItemState::Completed { .. } | DownloadItemState::Failed { .. } => None,
        }
    }

    fn destination(&self) -> Option<&PathBuf> {
        match &self.state {
            DownloadItemState::Queued { .. } => None,
            DownloadItemState::Active { destination, .. } => destination.as_ref(),
            DownloadItemState::Completed { destination, .. } => Some(destination),
            DownloadItemState::Failed { destination, .. } => destination.as_ref(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_external_downloads_keep_pending_completed_and_failed_items() {
        let queued_progress = ProgressiveDownload::new();
        let active_progress = ProgressiveDownload::new();
        let state = TransferStateModel {
            downloads: HashMap::new(),
            cancelled_downloads: HashSet::new(),
            external_downloads: vec![
                ExternalDownloadItem {
                    id: "queued".to_string(),
                    title: "Queued".to_string(),
                    state: DownloadItemState::Queued {
                        source_url: "https://example.com/queued".to_string(),
                        progress: queued_progress,
                    },
                },
                ExternalDownloadItem {
                    id: "active".to_string(),
                    title: "Active".to_string(),
                    state: DownloadItemState::Active {
                        progress: active_progress,
                        source_url: Some("https://example.com/active".to_string()),
                        destination: Some(PathBuf::from("/tmp/active.mp4")),
                        started_at: Instant::now(),
                        duration_seconds: Some(120),
                    },
                },
                ExternalDownloadItem {
                    id: "done".to_string(),
                    title: "Done".to_string(),
                    state: DownloadItemState::Completed {
                        source_url: "https://example.com/done".to_string(),
                        destination: PathBuf::from("/tmp/done.mp4"),
                    },
                },
                ExternalDownloadItem {
                    id: "failed".to_string(),
                    title: "Failed".to_string(),
                    state: DownloadItemState::Failed {
                        source_url: "https://example.com/failed".to_string(),
                        destination: Some(PathBuf::from("/tmp/failed.mp4")),
                        error: "network".to_string(),
                    },
                },
            ],
        };

        let persisted = state.persisted_external_downloads();

        assert_eq!(persisted.len(), 4);
        assert!(matches!(
            persisted[0].state,
            PersistedExternalDownloadState::Pending
        ));
        assert_eq!(persisted[0].source_url, "https://example.com/queued");
        assert_eq!(persisted[0].destination, None);
        assert!(!persisted[0].paused);
        assert!(matches!(
            persisted[1].state,
            PersistedExternalDownloadState::Pending
        ));
        assert_eq!(persisted[1].source_url, "https://example.com/active");
        assert_eq!(
            persisted[1].destination.as_deref(),
            Some(PathBuf::from("/tmp/active.mp4").as_path())
        );
        assert!(!persisted[1].paused);
        assert!(matches!(
            persisted[2].state,
            PersistedExternalDownloadState::Completed { .. }
        ));
        assert!(matches!(
            persisted[3].state,
            PersistedExternalDownloadState::Failed { .. }
        ));
    }

    #[test]
    fn restore_persisted_external_downloads_skips_pending_items() {
        let mut state = TransferStateModel::new();
        state.restore_persisted_external_downloads(vec![
            PersistedExternalDownload {
                id: "pending".to_string(),
                title: "Pending".to_string(),
                source_url: "https://example.com/pending".to_string(),
                destination: Some(PathBuf::from("/tmp/pending.mp4")),
                paused: true,
                downloaded_bytes: Some(128),
                total_bytes: Some(512),
                state: PersistedExternalDownloadState::Pending,
            },
            PersistedExternalDownload {
                id: "done".to_string(),
                title: "Done".to_string(),
                source_url: "https://example.com/done".to_string(),
                destination: Some(PathBuf::from("/tmp/done.mp4")),
                paused: false,
                downloaded_bytes: None,
                total_bytes: None,
                state: PersistedExternalDownloadState::Completed {
                    destination: PathBuf::from("/tmp/done.mp4"),
                },
            },
        ]);

        let downloads = state.download_items();

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].id, "done");
        assert!(matches!(
            downloads[0].state,
            DownloadItemState::Completed { .. }
        ));
    }
}
