mod clock;
mod media;
mod worker;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use souvlaki::MediaControlEvent;

use crate::progressive::ProgressiveDownload;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Idle,
    Paused,
    Playing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackRuntimeStatus {
    pub position: Option<Duration>,
    pub buffering: bool,
    pub finished: bool,
    pub failed: bool,
}

#[derive(Clone)]
pub struct PlaybackController {
    tx: Sender<PlaybackCommand>,
}

#[derive(Clone, Debug)]
pub struct MediaSessionTrack {
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_seconds: Option<u32>,
}

#[derive(Clone, Debug)]
pub enum PlaybackSource {
    LocalFile(PathBuf),
    GrowingFile {
        path: PathBuf,
        final_path: PathBuf,
        download: ProgressiveDownload,
    },
}

impl PlaybackController {
    pub fn new(media_controls_hwnd: Option<*mut c_void>) -> (Self, Receiver<MediaControlEvent>) {
        let (tx, rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let media_controls = media::init_media_controls(event_tx, media_controls_hwnd);

        thread::Builder::new()
            .name("audio-playback".to_string())
            .spawn(move || worker::playback_worker(rx, media_controls))
            .expect("failed to spawn audio playback worker");

        (Self { tx }, event_rx)
    }

    pub fn play_source_at(
        &self,
        track: MediaSessionTrack,
        source: PlaybackSource,
        position: Option<Duration>,
    ) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::Play {
                track,
                source,
                position,
                done: done_tx,
            })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn pause(&self) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::Pause { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn resume(&self) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::Resume { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn stop(&self) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::Stop { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn position(&self) -> Result<Option<Duration>> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::Position { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn runtime_status(&self) -> Result<PlaybackRuntimeStatus> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::RuntimeStatus { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn warm(&self) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::Warm { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn seek_to(&self, position: Duration) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::SeekTo {
                position,
                done: done_tx,
            })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn restart_current(&self) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::RestartCurrent { done: done_tx })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn publish_session(
        &self,
        track: MediaSessionTrack,
        playback: PlaybackState,
        position: Option<Duration>,
        prime: bool,
    ) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::PublishSession {
                track,
                playback,
                position,
                prime,
                done: done_tx,
            })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }

    pub fn update_media_position(&self, playback: PlaybackState, position: Duration) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send(PlaybackCommand::UpdateMediaPosition {
                playback,
                position,
                done: done_tx,
            })
            .context("Playback worker is unavailable")?;

        done_rx
            .recv()
            .context("Playback worker disconnected before responding")?
    }
}

enum PlaybackCommand {
    Play {
        track: MediaSessionTrack,
        source: PlaybackSource,
        position: Option<Duration>,
        done: Sender<Result<()>>,
    },
    Pause {
        done: Sender<Result<()>>,
    },
    Resume {
        done: Sender<Result<()>>,
    },
    Stop {
        done: Sender<Result<()>>,
    },
    Position {
        done: Sender<Result<Option<Duration>>>,
    },
    RuntimeStatus {
        done: Sender<Result<PlaybackRuntimeStatus>>,
    },
    Warm {
        done: Sender<Result<()>>,
    },
    SeekTo {
        position: Duration,
        done: Sender<Result<()>>,
    },
    RestartCurrent {
        done: Sender<Result<()>>,
    },
    PublishSession {
        track: MediaSessionTrack,
        playback: PlaybackState,
        position: Option<Duration>,
        prime: bool,
        done: Sender<Result<()>>,
    },
    UpdateMediaPosition {
        playback: PlaybackState,
        position: Duration,
        done: Sender<Result<()>>,
    },
}
