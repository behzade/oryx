use std::future::Future;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread;
use std::time::Duration;

use crate::audio::PlaybackSource;
use crate::library::{Library, PreparedPlaybackTrack};
use crate::model::Track;
use crate::progressive::ProgressiveDownload;
use crate::provider::{SharedProvider, TrackSummary};
use crate::url_media::{
    download_video_to_path, fallback_title_for_url, next_download_destination, resolve_video_url,
};

#[derive(Clone)]
pub struct TransferManager {
    event_tx: Sender<TransferEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadPurpose {
    Explicit,
    PlaybackPrefetch,
    ExternalUrl,
}

#[derive(Clone)]
pub struct ReadyPlayback {
    pub current: Track,
    pub source: PlaybackSource,
    pub fully_cached: bool,
}

#[derive(Clone)]
pub enum TransferEvent {
    PlaybackReady {
        request_id: u64,
        playback: ReadyPlayback,
    },
    PlaybackFailed {
        request_id: u64,
        title: String,
        error: String,
    },
    DownloadStarted {
        track_id: String,
        title: String,
        purpose: DownloadPurpose,
        progress: ProgressiveDownload,
    },
    DownloadCompleted {
        track_id: String,
        title: String,
        purpose: DownloadPurpose,
    },
    DownloadCancelled {
        track_id: String,
    },
    DownloadFailed {
        track_id: String,
        title: String,
        purpose: DownloadPurpose,
        error: String,
    },
    ExternalDownloadQueued {
        download_id: String,
        title: String,
        source_url: String,
        progress: ProgressiveDownload,
    },
    ExternalDownloadStarted {
        download_id: String,
        title: String,
        source_url: String,
        destination: PathBuf,
        duration_seconds: Option<u64>,
        progress: ProgressiveDownload,
    },
    ExternalDownloadCompleted {
        download_id: String,
        title: String,
        source_url: String,
        destination: PathBuf,
    },
    ExternalDownloadCancelled {
        download_id: String,
    },
    ExternalDownloadFailed {
        download_id: String,
        title: String,
        source_url: String,
        destination: Option<PathBuf>,
        error: String,
    },
}

impl TransferManager {
    pub fn new() -> (Self, Receiver<TransferEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        (Self { event_tx }, event_rx)
    }

    pub fn queue_play_request(
        &self,
        request_id: u64,
        provider: Option<SharedProvider>,
        library: Library,
        selected_track: TrackSummary,
        track_position: Option<usize>,
        position: Option<Duration>,
    ) {
        let event_tx = self.event_tx.clone();
        thread::Builder::new()
            .name("transfer-play-request".to_string())
            .spawn(move || {
                let result = resolve_playback(
                    &event_tx,
                    provider,
                    library,
                    selected_track.clone(),
                    track_position,
                    position,
                );
                let event = match result {
                    Ok(playback) => TransferEvent::PlaybackReady {
                        request_id,
                        playback,
                    },
                    Err(error) => {
                        let error_message = format_anyhow_error(&error);
                        eprintln!(
                            "playback resolution failed for '{}': {error_message}",
                            selected_track.title
                        );
                        TransferEvent::PlaybackFailed {
                            request_id,
                            title: selected_track.title.clone(),
                            error: error_message,
                        }
                    }
                };
                let _ = event_tx.send(event);
            })
            .expect("failed to spawn transfer play request worker");
    }

    pub fn queue_download(
        &self,
        provider: SharedProvider,
        library: Library,
        selected_track: TrackSummary,
        track_position: Option<usize>,
    ) {
        let event_tx = self.event_tx.clone();
        let track_id = track_cache_key(&selected_track);
        let title = selected_track.title.clone();
        let progress = ProgressiveDownload::new();
        let _ = event_tx.send(TransferEvent::DownloadStarted {
            track_id: track_id.clone(),
            title: title.clone(),
            purpose: DownloadPurpose::Explicit,
            progress: progress.clone(),
        });

        thread::Builder::new()
            .name("transfer-download-request".to_string())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let song = block_on(provider.get_song_data(&selected_track.reference))?;
                    if progress.is_cancelled() {
                        anyhow::bail!("Download cancelled.");
                    }
                    library.ensure_track_cached_with_progress(
                        provider.as_ref(),
                        &selected_track,
                        track_position,
                        &song,
                        Some(&progress),
                    )?;
                    Ok(())
                })();

                let event = match result {
                    Ok(()) => TransferEvent::DownloadCompleted {
                        track_id,
                        title,
                        purpose: DownloadPurpose::Explicit,
                    },
                    Err(_error) if progress.is_cancelled() => {
                        TransferEvent::DownloadCancelled { track_id }
                    }
                    Err(error) => {
                        let error_message = format_anyhow_error(&error);
                        eprintln!("download failed for '{}': {error_message}", title);
                        progress.fail(error_message.clone());
                        TransferEvent::DownloadFailed {
                            track_id,
                            title,
                            purpose: DownloadPurpose::Explicit,
                            error: error_message,
                        }
                    }
                };
                let _ = event_tx.send(event);
            })
            .expect("failed to spawn transfer download worker");
    }

    pub fn queue_external_url_download(&self, source_url: String) {
        self.queue_external_url_download_with_id(
            next_external_download_id(),
            source_url,
            None,
            None,
            None,
        );
    }

    pub fn queue_external_url_download_with_id(
        &self,
        download_id: String,
        source_url: String,
        preferred_destination: Option<PathBuf>,
        progress: Option<ProgressiveDownload>,
        queued_title: Option<String>,
    ) {
        let event_tx = self.event_tx.clone();
        let queued_title = queued_title.unwrap_or_else(|| fallback_title_for_url(&source_url));
        let progress = progress.unwrap_or_else(ProgressiveDownload::new);
        let _ = event_tx.send(TransferEvent::ExternalDownloadQueued {
            download_id: download_id.clone(),
            title: queued_title,
            source_url: source_url.clone(),
            progress: progress.clone(),
        });

        thread::Builder::new()
            .name("transfer-open-url-download".to_string())
            .spawn(move || {
                let mut resolved_title = fallback_title_for_url(&source_url);
                let mut destination = None;

                let result = (|| -> anyhow::Result<()> {
                    progress.wait_if_paused()?;
                    let resolved = resolve_video_url(&source_url)?;
                    if let Some(title) = resolved.title.clone() {
                        resolved_title = title;
                    }
                    let resolved_destination = next_download_destination(
                        resolved.title.as_deref(),
                        resolved.extension.as_deref(),
                        &resolved.download_request.url,
                        preferred_destination.as_deref(),
                    )?;
                    destination = Some(resolved_destination.clone());
                    let _ = event_tx.send(TransferEvent::ExternalDownloadStarted {
                        download_id: download_id.clone(),
                        title: resolved_title.clone(),
                        source_url: source_url.clone(),
                        destination: resolved_destination.clone(),
                        duration_seconds: resolved.duration_seconds,
                        progress: progress.clone(),
                    });
                    download_video_to_path(
                        &resolved.download_request,
                        &resolved_destination,
                        Some(&progress),
                    )?;
                    Ok(())
                })();

                let event = match result {
                    Ok(()) => TransferEvent::ExternalDownloadCompleted {
                        download_id,
                        title: resolved_title,
                        source_url,
                        destination: destination.expect("external download destination missing"),
                    },
                    Err(_error) if progress.is_cancelled() => {
                        TransferEvent::ExternalDownloadCancelled { download_id }
                    }
                    Err(error) => {
                        let error_message = format_anyhow_error(&error);
                        eprintln!(
                            "open url download failed for '{}': {error_message}",
                            source_url
                        );
                        progress.fail(error_message.clone());
                        TransferEvent::ExternalDownloadFailed {
                            download_id,
                            title: resolved_title,
                            source_url,
                            destination,
                            error: error_message,
                        }
                    }
                };
                let _ = event_tx.send(event);
            })
            .expect("failed to spawn external download worker");
    }
}

fn resolve_playback(
    event_tx: &Sender<TransferEvent>,
    provider: Option<SharedProvider>,
    library: Library,
    selected_track: TrackSummary,
    track_position: Option<usize>,
    position: Option<Duration>,
) -> anyhow::Result<ReadyPlayback> {
    if let Some(prepared) =
        library.prepare_cached_track_for_playback(&selected_track, track_position)?
    {
        return Ok(ready_playback_from_prepared(selected_track, prepared));
    }

    let provider = provider.ok_or_else(|| {
        anyhow::anyhow!(
            "Provider '{}' is not available and no cached audio exists for '{}'.",
            selected_track.reference.provider,
            selected_track.title
        )
    })?;

    let song = block_on(provider.get_song_data(&selected_track.reference))?;
    let prepared = library.prepare_track_for_playback(
        provider.as_ref(),
        &selected_track,
        track_position,
        &song,
        position,
    )?;

    if let Some(cache_monitor) = prepared.cache_monitor.clone() {
        let track_id = track_cache_key(&selected_track);
        let title = selected_track.title.clone();
        let _ = event_tx.send(TransferEvent::DownloadStarted {
            track_id: track_id.clone(),
            title: title.clone(),
            purpose: DownloadPurpose::PlaybackPrefetch,
            progress: cache_monitor.clone(),
        });
        spawn_download_monitor(
            event_tx.clone(),
            track_id,
            title,
            cache_monitor,
            DownloadPurpose::PlaybackPrefetch,
        );
    }

    Ok(ready_playback_from_prepared(selected_track, prepared))
}

fn ready_playback_from_prepared(
    mut selected_track: TrackSummary,
    prepared: PreparedPlaybackTrack,
) -> ReadyPlayback {
    if selected_track.bitrate_bps.is_none() {
        selected_track.bitrate_bps = prepared.bitrate_bps;
    }
    if selected_track.audio_format.is_none() {
        selected_track.audio_format = prepared.audio_format.clone();
    }

    let current = Track::from_track_summary_with_source(
        selected_track,
        prepared.display_path.display().to_string(),
        prepared.artwork_path.clone(),
    );

    ReadyPlayback {
        current,
        source: prepared.source,
        fully_cached: prepared.fully_cached,
    }
}

fn spawn_download_monitor(
    event_tx: Sender<TransferEvent>,
    track_id: String,
    title: String,
    progress: ProgressiveDownload,
    purpose: DownloadPurpose,
) {
    thread::Builder::new()
        .name("transfer-download-monitor".to_string())
        .spawn(move || {
            let event = match progress.wait_for_completion() {
                Ok(()) => TransferEvent::DownloadCompleted {
                    track_id,
                    title,
                    purpose,
                },
                Err(error) if error.is_cancelled() => TransferEvent::DownloadCancelled { track_id },
                Err(error) => {
                    let error_message = error.to_string();
                    eprintln!("download monitor failed for '{}': {error_message}", title);
                    TransferEvent::DownloadFailed {
                        track_id,
                        title,
                        purpose,
                        error: error_message,
                    }
                }
            };
            let _ = event_tx.send(event);
        })
        .expect("failed to spawn transfer download monitor");
}

fn track_cache_key(track: &TrackSummary) -> String {
    format!(
        "{}:{}",
        track.reference.provider.as_str(),
        track.reference.id
    )
}

fn format_anyhow_error(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = noop_waker();
    let mut future = pin!(future);
    let mut context = Context::from_waker(&waker);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::yield_now(),
        }
    }
}

fn noop_waker() -> Waker {
    unsafe { Waker::from_raw(noop_raw_waker()) }
}

fn noop_raw_waker() -> RawWaker {
    RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE)
}

static NOOP_WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(|_| noop_raw_waker(), |_| {}, |_| {}, |_| {});

fn next_external_download_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    format!("external-url:{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::params;

    use super::*;
    use crate::library::Library;
    use crate::provider::{ProviderId, TrackRef};

    fn fixture_provider() -> ProviderId {
        ProviderId::parse("fixture_remote").expect("fixture provider id should parse")
    }

    #[test]
    fn resolves_cached_playback_without_a_provider() {
        let root = std::env::temp_dir().join(format!(
            "oryx-transfer-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ));
        let library = Library::new_in(root.clone()).expect("test library should initialize");
        let audio_path = root.join("library").join("cached.mp3");
        fs::create_dir_all(
            audio_path
                .parent()
                .expect("audio path should have a parent"),
        )
        .expect("audio parent dir should exist");
        fs::write(&audio_path, b"cached audio").expect("audio fixture should be written");

        let selected_track = TrackSummary {
            reference: TrackRef::new(
                fixture_provider(),
                "track-1",
                Some("https://example.com/tracks/1".to_string()),
                Some("Track One".to_string()),
            ),
            title: "Track One".to_string(),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            collection_id: Some("album-1".to_string()),
            collection_title: Some("Album".to_string()),
            collection_subtitle: Some("Artist".to_string()),
            duration_seconds: None,
            bitrate_bps: None,
            audio_format: None,
            artwork_url: None,
        };

        let connection = rusqlite::Connection::open(root.join("oryx.db"))
            .expect("test database should be openable");
        connection
            .execute(
                r#"
                INSERT INTO cached_tracks (
                    provider,
                    track_id,
                    canonical_url,
                    artist,
                    album,
                    title,
                    collection_id,
                    collection_title,
                    collection_subtitle,
                    track_position,
                    duration_seconds,
                    bitrate_bps,
                    audio_format,
                    stream_url,
                    audio_path,
                    artwork_path
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, NULL, ?11, ?12, NULL)
                "#,
                params![
                    selected_track.reference.provider.as_str(),
                    selected_track.reference.id.as_str(),
                    selected_track.reference.canonical_url.as_deref(),
                    selected_track.artist.as_deref(),
                    selected_track.album.as_deref(),
                    selected_track.title.as_str(),
                    selected_track.collection_id.as_deref(),
                    selected_track.collection_title.as_deref(),
                    selected_track.collection_subtitle.as_deref(),
                    0usize,
                    "https://cdn.example.com/track-1.mp3",
                    audio_path.to_string_lossy().to_string(),
                ],
            )
            .expect("cached track row should be inserted");

        let (event_tx, _event_rx) = mpsc::channel();
        let playback = resolve_playback(&event_tx, None, library, selected_track, Some(0), None)
            .expect("cached playback should resolve without a provider");

        assert!(playback.fully_cached);
        match playback.source {
            PlaybackSource::LocalFile(path) => assert_eq!(path, audio_path),
            PlaybackSource::GrowingFile { .. } => panic!("expected local cached playback"),
        }

        fs::remove_dir_all(&root).expect("temp dir should be removed");
    }
}
