use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use gpui::{Context, EventEmitter};
use souvlaki::{MediaControlEvent, SeekDirection};
use url::Url;

use crate::audio::{
    MediaSessionTrack, PlaybackController, PlaybackRuntimeStatus, PlaybackSource, PlaybackState,
};
use crate::model::{PlaybackStatus, RepeatMode, Track};
use crate::provider::{ProviderId, TrackList};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum PlaybackContextTrackRemoval {
    Unchanged,
    ContextUpdated,
    CurrentTrackRemoved,
}

#[derive(Clone)]
pub(in crate::app) struct PendingPlayRequest {
    pub(in crate::app) request_id: u64,
    pub(in crate::app) playback_context: TrackList,
    pub(in crate::app) index: usize,
    pub(in crate::app) position: Option<Duration>,
    pub(in crate::app) browser_track_list: Option<TrackList>,
    pub(in crate::app) browser_collection_id: Option<String>,
    pub(in crate::app) browser_contains_track: bool,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum PlaybackIntent {
    Pause,
    Play,
    Toggle,
    Stop,
    Next,
    Previous,
    SeekBackward(Duration),
    SeekForward(Duration),
    SeekTo(Duration),
}

#[derive(Clone, Debug)]
pub(in crate::app) enum PlaybackRuntimeEvent {
    UiRefreshRequested,
    TrackFinished,
    PlaybackFailed,
}

pub(in crate::app) struct PlaybackModule {
    controller: PlaybackController,
    playback_context: Option<TrackList>,
    current_track_index: Option<usize>,
    now_playing: Option<Track>,
    resume_position: Duration,
    playback_status: PlaybackStatus,
    repeat_mode: RepeatMode,
    shuffle_enabled: bool,
    shuffle_seed: u64,
    play_loading: bool,
    playback_status_before_loading: Option<PlaybackStatus>,
    play_nonce: u64,
    pending_play_request: Option<PendingPlayRequest>,
}

impl EventEmitter<PlaybackIntent> for PlaybackModule {}
impl EventEmitter<PlaybackRuntimeEvent> for PlaybackModule {}

impl PlaybackModule {
    const MEDIA_CONTROL_SEEK_STEP: Duration = Duration::from_secs(10);

    pub(in crate::app) fn new(
        controller: PlaybackController,
        playback_context: Option<TrackList>,
        current_track_index: Option<usize>,
        now_playing: Option<Track>,
        resume_position: Duration,
        playback_status: PlaybackStatus,
        repeat_mode: RepeatMode,
        shuffle_enabled: bool,
        shuffle_seed: u64,
    ) -> Self {
        Self {
            controller,
            playback_context,
            current_track_index,
            now_playing,
            resume_position,
            playback_status,
            repeat_mode,
            shuffle_enabled,
            shuffle_seed,
            play_loading: false,
            playback_status_before_loading: None,
            play_nonce: 0,
            pending_play_request: None,
        }
    }

    pub(in crate::app) fn play_source_at(
        &self,
        track: MediaSessionTrack,
        source: PlaybackSource,
        position: Option<Duration>,
    ) -> Result<()> {
        self.controller.play_source_at(track, source, position)
    }

    pub(in crate::app) fn pause(&self) -> Result<()> {
        self.controller.pause()
    }

    pub(in crate::app) fn resume(&self) -> Result<()> {
        self.controller.resume()
    }

    pub(in crate::app) fn stop(&self) -> Result<()> {
        self.controller.stop()
    }

    pub(in crate::app) fn position(&self) -> Result<Option<Duration>> {
        self.controller.position()
    }

    pub(in crate::app) fn current_playback_position(&self) -> Duration {
        self.position()
            .ok()
            .flatten()
            .unwrap_or(self.resume_position)
    }

    pub(in crate::app) fn current_track_duration(&self) -> Option<Duration> {
        self.now_playing
            .as_ref()
            .and_then(|track| track.duration_seconds)
            .map(|seconds| Duration::from_secs(seconds as u64))
    }

    pub(in crate::app) fn runtime_status(&self) -> Result<PlaybackRuntimeStatus> {
        self.controller.runtime_status()
    }

    pub(in crate::app) fn seek_to(&self, position: Duration) -> Result<()> {
        self.controller.seek_to(position)
    }

    pub(in crate::app) fn restart_current(&self) -> Result<()> {
        self.controller.restart_current()
    }

    pub(in crate::app) fn publish_session(
        &self,
        track: MediaSessionTrack,
        playback: PlaybackState,
        position: Option<Duration>,
        prime: bool,
    ) -> Result<()> {
        self.controller
            .publish_session(track, playback, position, prime)
    }

    pub(in crate::app) fn update_media_position(
        &self,
        playback: PlaybackState,
        position: Duration,
    ) -> Result<()> {
        self.controller.update_media_position(playback, position)
    }

    pub(in crate::app) fn handle_media_control_event(
        &mut self,
        event: MediaControlEvent,
        cx: &mut Context<Self>,
    ) {
        if let Some(intent) = Self::media_control_intent(event) {
            cx.emit(intent);
        }
    }

    pub(in crate::app) fn publish_restored_media_session(&self) {
        let Some(track) = self.now_playing.as_ref() else {
            return;
        };

        let playback = match self.playback_status {
            PlaybackStatus::Playing => PlaybackState::Playing,
            PlaybackStatus::Paused | PlaybackStatus::Buffering => PlaybackState::Paused,
            PlaybackStatus::Idle => PlaybackState::Idle,
        };

        if let Err(error) = self.publish_session(
            media_session_track(track),
            playback,
            Some(self.current_playback_position()),
            true,
        ) {
            eprintln!("failed to publish restored media session: {error}");
        }
    }

    pub(in crate::app) fn handle_refresh_tick(
        &mut self,
        has_active_downloads: bool,
        cx: &mut Context<Self>,
    ) {
        let mut should_notify = has_active_downloads;

        if self.should_poll_runtime()
            && let Ok(runtime_status) = self.runtime_status()
        {
            match self.handle_runtime_status(runtime_status) {
                RuntimeTickOutcome::NoChange => {}
                RuntimeTickOutcome::PositionUpdated(position) => {
                    let _ = self.update_media_position(PlaybackState::Playing, position);
                    should_notify = true;
                }
                RuntimeTickOutcome::TrackFinished => {
                    cx.emit(PlaybackRuntimeEvent::TrackFinished);
                }
                RuntimeTickOutcome::UiRefreshRequested => {
                    should_notify = true;
                }
                RuntimeTickOutcome::PlaybackFailed => {
                    cx.emit(PlaybackRuntimeEvent::PlaybackFailed);
                }
            }
        }

        if should_notify {
            cx.emit(PlaybackRuntimeEvent::UiRefreshRequested);
        }
    }

    pub(in crate::app) fn playback_context(&self) -> Option<TrackList> {
        self.playback_context.clone()
    }

    pub(in crate::app) fn current_track_index(&self) -> Option<usize> {
        self.current_track_index
    }

    pub(in crate::app) fn now_playing(&self) -> Option<Track> {
        self.now_playing.clone()
    }

    pub(in crate::app) fn resume_position(&self) -> Duration {
        self.resume_position
    }

    pub(in crate::app) fn playback_status(&self) -> PlaybackStatus {
        self.playback_status.clone()
    }

    pub(in crate::app) fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode.clone()
    }

    pub(in crate::app) fn shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    pub(in crate::app) fn shuffle_seed(&self) -> u64 {
        self.shuffle_seed
    }

    pub(in crate::app) fn play_loading(&self) -> bool {
        self.play_loading
    }

    pub(in crate::app) fn should_poll_runtime(&self) -> bool {
        if self.play_loading || self.pending_play_request.is_some() {
            return false;
        }

        matches!(
            self.playback_status,
            PlaybackStatus::Playing | PlaybackStatus::Buffering
        ) && self.now_playing.is_some()
    }

    pub(in crate::app) fn cycle_repeat_mode(&mut self) -> RepeatMode {
        let repeat_mode = self.repeat_mode.cycle();
        self.repeat_mode = repeat_mode.clone();
        repeat_mode
    }

    pub(in crate::app) fn set_shuffle_enabled(&mut self, enabled: bool, seed: u64) {
        self.shuffle_enabled = enabled;
        self.shuffle_seed = seed;
    }

    pub(in crate::app) fn set_resume_position(&mut self, position: Duration) {
        self.resume_position = position;
    }

    pub(in crate::app) fn set_playback_status(&mut self, status: PlaybackStatus) {
        self.playback_status = status;
    }

    pub(in crate::app) fn set_play_loading(&mut self, loading: bool) {
        self.play_loading = loading;
    }

    pub(in crate::app) fn begin_playback_resolution(&mut self, resume_position: Duration) {
        if self.playback_status_before_loading.is_none() && self.now_playing.is_some() {
            self.playback_status_before_loading = Some(self.playback_status.clone());
        }
        self.resume_position = resume_position;
        self.play_loading = true;
        self.playback_status = PlaybackStatus::Buffering;
    }

    pub(in crate::app) fn begin_play_request(
        &mut self,
        mut request: PendingPlayRequest,
        resume_position: Duration,
    ) -> u64 {
        self.play_nonce += 1;
        request.request_id = self.play_nonce;
        self.pending_play_request = Some(request);
        self.begin_playback_resolution(resume_position);
        self.play_nonce
    }

    pub(in crate::app) fn take_pending_request_if_matches(
        &mut self,
        request_id: u64,
    ) -> Option<PendingPlayRequest> {
        let matches = self
            .pending_play_request
            .as_ref()
            .map(|request| request.request_id == request_id)
            .unwrap_or(false);
        if matches {
            self.pending_play_request.take()
        } else {
            None
        }
    }

    pub(in crate::app) fn apply_ready_playback(
        &mut self,
        playback_context: TrackList,
        current_track_index: usize,
        now_playing: Track,
    ) {
        self.play_loading = false;
        self.playback_status_before_loading = None;
        self.playback_context = Some(playback_context);
        self.current_track_index = Some(current_track_index);
        self.now_playing = Some(now_playing);
        self.resume_position = Duration::ZERO;
        self.playback_status = PlaybackStatus::Playing;
    }

    pub(in crate::app) fn restart_current_playback(&mut self) {
        self.play_loading = false;
        self.playback_status_before_loading = None;
        self.pending_play_request = None;
        self.resume_position = Duration::ZERO;
        self.playback_status = PlaybackStatus::Playing;
    }

    pub(in crate::app) fn mark_playback_failed(&mut self) {
        self.play_loading = false;
        self.playback_status = self
            .playback_status_before_loading
            .take()
            .unwrap_or(PlaybackStatus::Idle);
    }

    pub(in crate::app) fn replace_playback_context(&mut self, playback_context: TrackList) {
        self.playback_context = Some(playback_context);
    }

    fn handle_runtime_status(
        &mut self,
        runtime_status: PlaybackRuntimeStatus,
    ) -> RuntimeTickOutcome {
        match runtime_status {
            PlaybackRuntimeStatus {
                position,
                buffering: false,
                finished: true,
                failed: false,
            } => {
                self.resume_position = position.unwrap_or_default();
                RuntimeTickOutcome::TrackFinished
            }
            PlaybackRuntimeStatus {
                position: Some(position),
                buffering: false,
                finished: false,
                failed: false,
            } => {
                self.resume_position = position;
                self.playback_status = PlaybackStatus::Playing;
                RuntimeTickOutcome::PositionUpdated(position)
            }
            PlaybackRuntimeStatus {
                position,
                buffering: true,
                ..
            } => {
                self.resume_position = position.unwrap_or(self.resume_position);
                if !matches!(self.playback_status, PlaybackStatus::Buffering) {
                    self.playback_status = PlaybackStatus::Buffering;
                    return RuntimeTickOutcome::UiRefreshRequested;
                }
                RuntimeTickOutcome::NoChange
            }
            PlaybackRuntimeStatus {
                position,
                buffering: false,
                finished: false,
                failed: true,
            } => {
                self.resume_position = position.unwrap_or(self.resume_position);
                RuntimeTickOutcome::PlaybackFailed
            }
            PlaybackRuntimeStatus {
                position: None,
                buffering: false,
                finished: false,
                failed: false,
            } => RuntimeTickOutcome::NoChange,
            PlaybackRuntimeStatus {
                position: None,
                buffering: false,
                finished: true,
                failed: true,
            } => RuntimeTickOutcome::PlaybackFailed,
            PlaybackRuntimeStatus {
                position: Some(position),
                buffering: false,
                finished: true,
                failed: true,
            } => {
                self.resume_position = position;
                RuntimeTickOutcome::PlaybackFailed
            }
        }
    }

    pub(in crate::app) fn remove_track(
        &mut self,
        provider: ProviderId,
        track_id: &str,
    ) -> PlaybackContextTrackRemoval {
        let Some(mut playback_context) = self.playback_context.clone() else {
            return PlaybackContextTrackRemoval::Unchanged;
        };
        let current_track_index = self.current_track_index;
        let removed_before_current = current_track_index
            .map(|current_track_index| {
                playback_context
                    .tracks
                    .iter()
                    .take(current_track_index)
                    .filter(|track| {
                        track.reference.provider == provider && track.reference.id == track_id
                    })
                    .count()
            })
            .unwrap_or(0);
        let current_track_removed = current_track_index
            .and_then(|current_track_index| playback_context.tracks.get(current_track_index))
            .map(|track| track.reference.provider == provider && track.reference.id == track_id)
            .unwrap_or(false);
        let original_len = playback_context.tracks.len();

        playback_context.tracks.retain(|track| {
            !(track.reference.provider == provider && track.reference.id == track_id)
        });

        if playback_context.tracks.len() == original_len {
            return PlaybackContextTrackRemoval::Unchanged;
        }

        playback_context.collection.track_count = Some(playback_context.tracks.len());

        if current_track_removed || playback_context.tracks.is_empty() {
            self.play_loading = false;
            self.playback_context = Some(playback_context);
            self.current_track_index = None;
            self.now_playing = None;
            self.resume_position = Duration::ZERO;
            self.playback_status = PlaybackStatus::Idle;
            return PlaybackContextTrackRemoval::CurrentTrackRemoved;
        }

        self.playback_context = Some(playback_context);
        self.current_track_index = current_track_index
            .map(|current_track_index| current_track_index.saturating_sub(removed_before_current));
        PlaybackContextTrackRemoval::ContextUpdated
    }

    fn media_control_intent(event: MediaControlEvent) -> Option<PlaybackIntent> {
        Some(match event {
            MediaControlEvent::Pause => PlaybackIntent::Pause,
            MediaControlEvent::Play => PlaybackIntent::Play,
            MediaControlEvent::Toggle => PlaybackIntent::Toggle,
            MediaControlEvent::Stop => PlaybackIntent::Stop,
            MediaControlEvent::Next => PlaybackIntent::Next,
            MediaControlEvent::Previous => PlaybackIntent::Previous,
            MediaControlEvent::Seek(SeekDirection::Forward) => {
                PlaybackIntent::SeekForward(Self::MEDIA_CONTROL_SEEK_STEP)
            }
            MediaControlEvent::Seek(SeekDirection::Backward) => {
                PlaybackIntent::SeekBackward(Self::MEDIA_CONTROL_SEEK_STEP)
            }
            MediaControlEvent::SeekBy(SeekDirection::Forward, amount) => {
                PlaybackIntent::SeekForward(amount)
            }
            MediaControlEvent::SeekBy(SeekDirection::Backward, amount) => {
                PlaybackIntent::SeekBackward(amount)
            }
            MediaControlEvent::SetPosition(position) => PlaybackIntent::SeekTo(position.0),
            MediaControlEvent::SetVolume(_)
            | MediaControlEvent::OpenUri(_)
            | MediaControlEvent::Raise
            | MediaControlEvent::Quit => return None,
        })
    }
}

enum RuntimeTickOutcome {
    NoChange,
    UiRefreshRequested,
    PositionUpdated(Duration),
    TrackFinished,
    PlaybackFailed,
}

pub(in crate::app) fn media_session_track(track: &Track) -> MediaSessionTrack {
    MediaSessionTrack {
        title: track.title.clone(),
        album: Some(track.album.clone()),
        artist: Some(track.artist.clone()),
        artwork_url: track.artwork_path.as_deref().and_then(local_file_url),
        duration_seconds: track.duration_seconds,
    }
}

fn local_file_url(path: &Path) -> Option<String> {
    Url::from_file_path(path).ok().map(Into::into)
}
