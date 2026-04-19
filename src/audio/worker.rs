use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::cpal::StreamError;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use souvlaki::{MediaControls, MediaPlayback, MediaPosition};

use super::clock::PlaybackClock;
use super::media;
use super::{
    MediaSessionTrack, PlaybackCommand, PlaybackRuntimeStatus, PlaybackSource, PlaybackState,
};

struct PlaybackBackend {
    sink: MixerDeviceSink,
    player: Option<Player>,
    media_controls: Option<MediaControls>,
    clock: PlaybackClock,
    stream_errors: Receiver<StreamError>,
    needs_rebuild: bool,
    current: Option<CurrentPlayback>,
    last_default_output: Option<OutputDeviceSnapshot>,
    route_changed_since_rebuild: bool,
}

#[derive(Clone)]
struct CurrentPlayback {
    track: MediaSessionTrack,
    source: PlaybackSource,
    playback: PlaybackState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OutputDeviceSnapshot {
    id: String,
    description: String,
}

const TRACK_END_TOLERANCE: Duration = Duration::from_millis(250);
const OUTPUT_ROUTE_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub(super) fn playback_worker(
    rx: Receiver<PlaybackCommand>,
    media_controls: Option<MediaControls>,
) {
    let mut backend: Option<PlaybackBackend> = None;
    let mut media_controls = media_controls;

    loop {
        let command = match rx.recv_timeout(OUTPUT_ROUTE_POLL_INTERVAL) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                if let Some(backend) = backend
                    .as_mut()
                    .filter(|backend| should_poll_output_route(backend.current.as_ref()))
                {
                    observe_output_route(backend);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };

        match command {
            PlaybackCommand::Play {
                track,
                source,
                position,
                done,
            } => {
                let result = play_song(&mut backend, &mut media_controls, track, source, position);
                let _ = done.send(result);
            }
            PlaybackCommand::Pause { done } => {
                let result = pause_playback(&mut backend, &mut media_controls);
                let _ = done.send(result);
            }
            PlaybackCommand::Resume { done } => {
                let result = resume_playback(&mut backend, &mut media_controls);
                let _ = done.send(result);
            }
            PlaybackCommand::Stop { done } => {
                let result = stop_playback(&mut backend, &mut media_controls);
                let _ = done.send(result);
            }
            PlaybackCommand::Position { done } => {
                let _ = done.send(Ok(current_position(&backend)));
            }
            PlaybackCommand::RuntimeStatus { done } => {
                let _ = done.send(runtime_status(&mut backend));
            }
            PlaybackCommand::Warm { done } => {
                let result = warm_output(&mut backend, &mut media_controls);
                let _ = done.send(result);
            }
            PlaybackCommand::SeekTo { position, done } => {
                let result = seek_to_position(&mut backend, &mut media_controls, position);
                let _ = done.send(result);
            }
            PlaybackCommand::RestartCurrent { done } => {
                let result = restart_current_playback(&mut backend, &mut media_controls);
                let _ = done.send(result);
            }
            PlaybackCommand::PublishSession {
                track,
                playback,
                position,
                prime,
                done,
            } => {
                let result = publish_session(
                    &mut backend,
                    &mut media_controls,
                    &track,
                    playback,
                    position,
                    prime,
                );
                let _ = done.send(result);
            }
            PlaybackCommand::UpdateMediaPosition {
                playback,
                position,
                done,
            } => {
                let result =
                    update_media_position(&mut backend, &mut media_controls, playback, position);
                let _ = done.send(result);
            }
        }
    }
}

fn play_song(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
    track: MediaSessionTrack,
    source: PlaybackSource,
    position: Option<Duration>,
) -> Result<()> {
    let backend = ensure_backend(backend, media_controls.take())?;
    observe_output_route(backend);
    // macOS/CoreAudio route changes can leave a rodio/CPAL output stream silently stale without
    // emitting StreamInvalidated/DeviceNotAvailable. In the reproduced Bluetooth cases:
    // 1. The default output changed away from the headphones and sometimes back again.
    // 2. CPAL often kept reporting the same logical device id after reconnect.
    // 3. The existing sink/player kept advancing transport state but produced no audible output.
    //
    // Because the stream can be dead even when the current default device "matches" again, we
    // cannot rely on sink-vs-default equality or on CPAL error callbacks alone. The reliable app-
    // level signal we do have is that the OS output route changed at some point after the sink was
    // built. When that happens, the next explicit playback action must rebuild the sink before it
    // tries to use the stale stream.
    if backend.route_changed_since_rebuild {
        backend.needs_rebuild = true;
    }
    rebuild_output_if_needed(backend)?;

    if let Some(current_player) = backend.player.take() {
        current_player.stop();
    }

    let player = build_player(&backend.sink, &source, position)?;
    backend.player = Some(player);
    let position = position.unwrap_or(Duration::ZERO);
    backend.clock.start(position);
    backend.needs_rebuild = false;
    backend.current = Some(CurrentPlayback {
        track: track.clone(),
        source,
        playback: PlaybackState::Playing,
    });
    media::publish_now_playing(&mut backend.media_controls, &track);
    media::set_media_playback_state(
        &mut backend.media_controls,
        MediaPlayback::Playing {
            progress: Some(MediaPosition(position)),
        },
    );

    Ok(())
}

fn pause_playback(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
) -> Result<()> {
    let Some(backend) = backend.as_mut() else {
        return Ok(());
    };
    rebuild_output_if_needed(backend)?;

    if backend.media_controls.is_none() {
        backend.media_controls = media_controls.take();
    }

    if let Some(player) = backend.player.as_ref() {
        player.pause();
        backend.clock.pause();
        if let Some(current) = backend.current.as_mut() {
            current.playback = PlaybackState::Paused;
        }
        let progress = backend.clock.elapsed().map(MediaPosition);
        media::set_media_playback_state(
            &mut backend.media_controls,
            MediaPlayback::Paused { progress },
        );
    }

    Ok(())
}

fn resume_playback(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
) -> Result<()> {
    let Some(backend) = backend.as_mut() else {
        return Ok(());
    };
    if backend.route_changed_since_rebuild && backend.current.is_some() {
        backend.needs_rebuild = true;
    }
    if debug_force_rebuild_on_resume_enabled() && backend.current.is_some() {
        eprintln!("audio debug: forcing output rebuild on resume");
        backend.needs_rebuild = true;
    }
    rebuild_output_if_needed(backend)?;

    if backend.media_controls.is_none() {
        backend.media_controls = media_controls.take();
    }

    if let Some(player) = backend.player.as_ref() {
        player.play();
        backend.clock.resume();
        if let Some(current) = backend.current.as_mut() {
            current.playback = PlaybackState::Playing;
        }
        let progress = backend.clock.elapsed().map(MediaPosition);
        media::set_media_playback_state(
            &mut backend.media_controls,
            MediaPlayback::Playing { progress },
        );
    }

    Ok(())
}

fn stop_playback(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
) -> Result<()> {
    let Some(backend) = backend.as_mut() else {
        return Ok(());
    };

    if backend.media_controls.is_none() {
        backend.media_controls = media_controls.take();
    }

    if let Some(player) = backend.player.take() {
        player.stop();
    }
    backend.clock.stop();
    backend.current = None;
    backend.needs_rebuild = false;
    backend.route_changed_since_rebuild = false;
    media::set_media_playback_state(&mut backend.media_controls, MediaPlayback::Stopped);

    Ok(())
}

fn ensure_backend(
    backend: &mut Option<PlaybackBackend>,
    media_controls: Option<MediaControls>,
) -> Result<&mut PlaybackBackend> {
    if backend.is_none() {
        let (mut sink, stream_errors, _sink_output) = open_output_sink()?;
        sink.log_on_drop(false);
        *backend = Some(PlaybackBackend {
            sink,
            player: None,
            media_controls,
            clock: PlaybackClock::default(),
            stream_errors,
            needs_rebuild: false,
            current: None,
            last_default_output: current_default_output_device(),
            route_changed_since_rebuild: false,
        });
    } else if let Some(media_controls) = media_controls {
        backend
            .as_mut()
            .expect("playback backend should exist after initialization")
            .media_controls = Some(media_controls);
    }

    Ok(backend
        .as_mut()
        .expect("playback backend should exist after initialization"))
}

fn current_position(backend: &Option<PlaybackBackend>) -> Option<Duration> {
    backend.as_ref().and_then(playback_position)
}

fn runtime_status(backend: &mut Option<PlaybackBackend>) -> Result<PlaybackRuntimeStatus> {
    let Some(backend) = backend.as_mut() else {
        return Ok(PlaybackRuntimeStatus {
            position: None,
            buffering: false,
            finished: false,
            failed: false,
        });
    };
    drain_stream_errors(backend);
    observe_output_route(backend);
    rebuild_output_if_needed(backend)?;

    let Some(player) = backend.player.as_ref() else {
        return Ok(PlaybackRuntimeStatus {
            position: backend.clock.elapsed(),
            buffering: false,
            finished: false,
            failed: false,
        });
    };

    let progressive_buffering = player.empty()
        && backend
            .current
            .as_ref()
            .map(current_progressive_retrying)
            .unwrap_or(false);
    let progressive_failed = player.empty()
        && backend
            .current
            .as_ref()
            .and_then(current_progressive_failure)
            .is_some();
    let position = playback_position(backend).unwrap_or_else(|| player.get_pos());
    let at_track_end = backend
        .current
        .as_ref()
        .and_then(|current| current.track.duration_seconds)
        .map(|seconds| {
            position.saturating_add(TRACK_END_TOLERANCE) >= Duration::from_secs(seconds as u64)
        })
        .unwrap_or(false);

    Ok(PlaybackRuntimeStatus {
        position: Some(position),
        buffering: progressive_buffering,
        finished: (player.empty() || at_track_end) && !progressive_buffering && !progressive_failed,
        failed: progressive_failed,
    })
}

fn warm_output(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
) -> Result<()> {
    let backend = ensure_backend(backend, media_controls.take())?;
    rebuild_output_if_needed(backend)?;
    Ok(())
}

fn seek_to_position(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
    position: Duration,
) -> Result<()> {
    let Some(backend) = backend.as_mut() else {
        return Ok(());
    };
    if backend.route_changed_since_rebuild && backend.current.is_some() {
        backend.needs_rebuild = true;
    }
    rebuild_output_if_needed(backend)?;

    if backend.media_controls.is_none() {
        backend.media_controls = media_controls.take();
    }

    let Some(current) = backend.current.clone() else {
        return Ok(());
    };

    if let Some(player) = backend.player.take() {
        player.stop();
    }

    let player = build_player(&backend.sink, &current.source, Some(position))?;
    backend.player = Some(player);
    backend.clock.start(position);
    backend.needs_rebuild = false;
    match current.playback {
        PlaybackState::Playing => {}
        PlaybackState::Paused => {
            if let Some(player) = backend.player.as_ref() {
                player.pause();
            }
            backend.clock.pause();
        }
        PlaybackState::Idle => {
            if let Some(player) = backend.player.take() {
                player.stop();
            }
            backend.clock.stop();
            backend.current = None;
            media::set_media_playback_state(&mut backend.media_controls, MediaPlayback::Stopped);
            return Ok(());
        }
    }

    media::set_media_playback_state(
        &mut backend.media_controls,
        media_playback(current.playback, Some(MediaPosition(position))),
    );

    Ok(())
}

fn restart_current_playback(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
) -> Result<()> {
    let Some(backend) = backend.as_mut() else {
        return Ok(());
    };
    if backend.route_changed_since_rebuild && backend.current.is_some() {
        backend.needs_rebuild = true;
    }
    rebuild_output_if_needed(backend)?;

    if backend.media_controls.is_none() {
        backend.media_controls = media_controls.take();
    }

    let Some(current) = backend.current.clone() else {
        return Ok(());
    };

    if let Some(player) = backend.player.take() {
        player.stop();
    }

    let player = build_player(&backend.sink, &current.source, Some(Duration::ZERO))?;
    backend.player = Some(player);
    backend.clock.start(Duration::ZERO);
    backend.needs_rebuild = false;
    if let Some(current_playback) = backend.current.as_mut() {
        current_playback.playback = PlaybackState::Playing;
    }

    media::publish_now_playing(&mut backend.media_controls, &current.track);
    media::set_media_playback_state(
        &mut backend.media_controls,
        MediaPlayback::Playing {
            progress: Some(MediaPosition(Duration::ZERO)),
        },
    );

    Ok(())
}

fn open_output_sink() -> Result<(
    MixerDeviceSink,
    Receiver<StreamError>,
    Option<OutputDeviceSnapshot>,
)> {
    let (error_tx, error_rx) = mpsc::channel();
    let host = rodio::cpal::default_host();
    let default_device = host.default_output_device();
    let default_device_id = default_device.as_ref().and_then(output_device_id);

    if let Some(device) = default_device {
        match open_output_sink_for_device(device, error_tx.clone()) {
            Ok((sink, sink_output)) => {
                log_selected_output("opened default audio output", sink_output.as_ref());
                return Ok((sink, error_rx, sink_output));
            }
            Err(error) => {
                eprintln!("failed to open default audio output, trying fallbacks: {error}");
            }
        }
    } else {
        eprintln!("failed to resolve the default audio output, trying enumerated fallbacks");
    }

    let mut fallback_error = None;
    let devices = host
        .output_devices()
        .context("Failed to enumerate audio outputs after default output resolution failed")?
        .filter(usable_output_device)
        .collect::<Vec<_>>();

    let mut sorted_devices = devices
        .into_iter()
        .filter(|device| {
            !(output_device_id(device).is_some() && output_device_id(device) == default_device_id)
        })
        .collect::<Vec<_>>();
    sorted_devices.sort_by_key(output_device_preference);

    for device in sorted_devices {
        let attempted_output = snapshot_output_device(&device);
        match open_output_sink_for_device(device, error_tx.clone()) {
            Ok((sink, sink_output)) => {
                log_selected_output("opened fallback audio output", sink_output.as_ref());
                return Ok((sink, error_rx, sink_output));
            }
            Err(error) => {
                log_output_open_failure(
                    "fallback audio output failed",
                    attempted_output.as_ref(),
                    &error,
                );
                if fallback_error.is_none() {
                    fallback_error = Some(error);
                }
            }
        }
    }

    Err(fallback_error
        .unwrap_or_else(|| anyhow::anyhow!("Failed to resolve a working audio output")))
}

fn open_output_sink_for_device(
    device: rodio::cpal::Device,
    error_tx: mpsc::Sender<StreamError>,
) -> Result<(MixerDeviceSink, Option<OutputDeviceSnapshot>)> {
    let sink_output = snapshot_output_device(&device);
    let device_name = sink_output
        .as_ref()
        .map(|snapshot| snapshot.description.clone())
        .unwrap_or_else(|| "unknown output device".to_string());

    let error_callback = move |error| {
        let _ = error_tx.send(error);
    };

    let sink = DeviceSinkBuilder::from_device(device)
        .with_context(|| format!("Failed to configure audio output for {device_name}"))?
        .with_error_callback(error_callback)
        .open_sink_or_fallback()
        .with_context(|| format!("Failed to open audio output for {device_name}"))?;

    Ok((sink, sink_output))
}

fn output_device_id(device: &rodio::cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

fn usable_output_device(device: &rodio::cpal::Device) -> bool {
    device
        .description()
        .map(|description| description.driver().is_some_and(|driver| driver != "null"))
        .unwrap_or(false)
}

fn output_device_preference(device: &rodio::cpal::Device) -> (u8, String) {
    let snapshot = snapshot_output_device(device);
    let description = snapshot
        .as_ref()
        .map(|snapshot| snapshot.description.to_ascii_lowercase())
        .unwrap_or_default();
    let id = snapshot
        .as_ref()
        .map(|snapshot| snapshot.id.to_ascii_lowercase())
        .unwrap_or_default();

    (output_device_priority(&description, &id), description)
}

fn output_device_priority(description: &str, id: &str) -> u8 {
    #[cfg(target_os = "linux")]
    {
        if id == "pipewire" || description.contains("pipewire") {
            return 0;
        }
        if id == "pulse" || description.contains("pulseaudio") {
            return 1;
        }
        if id == "default" || description.contains("default audio device") {
            return 2;
        }
        if id == "jack" || description.contains("jack audio connection kit") {
            return 4;
        }
        if id == "oss" || description.contains("open sound system") {
            return 5;
        }
        return 3;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (description, id);
        0
    }
}

fn log_selected_output(prefix: &str, output: Option<&OutputDeviceSnapshot>) {
    if let Some(output) = output {
        eprintln!("{prefix}: {} [{}]", output.description, output.id);
    } else {
        eprintln!("{prefix}: unknown output device");
    }
}

fn log_output_open_failure(
    prefix: &str,
    output: Option<&OutputDeviceSnapshot>,
    error: &anyhow::Error,
) {
    if let Some(output) = output {
        eprintln!("{prefix}: {} [{}]: {error}", output.description, output.id);
    } else {
        eprintln!("{prefix}: unknown output device: {error}");
    }
}

fn build_player(
    sink: &MixerDeviceSink,
    source: &PlaybackSource,
    position: Option<Duration>,
) -> Result<Player> {
    let player = Player::connect_new(sink.mixer());
    match source {
        PlaybackSource::LocalFile(audio_path) => {
            let file = File::open(audio_path).with_context(|| {
                format!("Failed to open local audio file {}", audio_path.display())
            })?;
            let mut decoder = Decoder::try_from(file).with_context(|| {
                format!("Failed to decode local audio file {}", audio_path.display())
            })?;
            seek_source_to(&mut decoder, position, audio_path)?;
            player.append(decoder);
        }
        PlaybackSource::GrowingFile {
            path,
            final_path,
            download,
        } => {
            let reader = open_progressive_reader(download, path, final_path)?;
            let mut decoder_builder = Decoder::builder().with_data(BufReader::new(reader));
            if let Some(total_bytes) = download.snapshot().total_bytes {
                decoder_builder = decoder_builder.with_byte_len(total_bytes);
            }
            let mut decoder = decoder_builder.build().with_context(|| {
                format!("Failed to decode progressive audio file {}", path.display())
            })?;
            seek_source_to(&mut decoder, position, path)?;
            player.append(decoder);
        }
    }
    player.play();
    Ok(player)
}

fn seek_source_to<S>(
    source: &mut S,
    position: Option<Duration>,
    path: &std::path::Path,
) -> Result<()>
where
    S: Source,
{
    let Some(position) = position else {
        return Ok(());
    };

    source
        .try_seek(position)
        .map_err(|error| anyhow::anyhow!("Failed to seek audio source {}: {error}", path.display()))
}

fn open_progressive_reader(
    download: &crate::progressive::ProgressiveDownload,
    path: &std::path::Path,
    final_path: &std::path::Path,
) -> Result<crate::progressive::GrowingFileReader> {
    match download.open_reader(path) {
        Ok(reader) => Ok(reader),
        Err(_error) if path != final_path && !path.is_file() && final_path.is_file() => {
            download.open_reader(final_path).with_context(|| {
                format!(
                    "Failed to open progressive audio file {} after temp file {} disappeared",
                    final_path.display(),
                    path.display()
                )
            })
        }
        Err(error) => Err(error)
            .with_context(|| format!("Failed to open progressive audio file {}", path.display())),
    }
}

fn rebuild_output_if_needed(backend: &mut PlaybackBackend) -> Result<()> {
    drain_stream_errors(backend);
    if !backend.needs_rebuild {
        return Ok(());
    }

    let position = playback_position(backend).unwrap_or_default();
    let (mut sink, stream_errors, _sink_output) = open_output_sink()?;
    sink.log_on_drop(false);
    if let Some(player) = backend.player.take() {
        player.stop();
    }
    backend.sink = sink;
    backend.stream_errors = stream_errors;
    backend.needs_rebuild = false;
    backend.last_default_output = current_default_output_device();
    backend.route_changed_since_rebuild = false;

    let Some(current) = backend.current.clone() else {
        return Ok(());
    };

    let player = build_player(&backend.sink, &current.source, Some(position))?;
    backend.player = Some(player);
    backend.clock.start(position);
    match current.playback {
        PlaybackState::Playing => {}
        PlaybackState::Paused => {
            if let Some(player) = backend.player.as_ref() {
                player.pause();
            }
            backend.clock.pause();
        }
        PlaybackState::Idle => {
            if let Some(player) = backend.player.take() {
                player.stop();
            }
            backend.clock.stop();
            backend.current = None;
            return Ok(());
        }
    }

    media::publish_now_playing(&mut backend.media_controls, &current.track);
    media::set_media_playback_state(
        &mut backend.media_controls,
        media_playback(current.playback, Some(MediaPosition(position))),
    );

    Ok(())
}

fn drain_stream_errors(backend: &mut PlaybackBackend) {
    while let Ok(error) = backend.stream_errors.try_recv() {
        match error {
            StreamError::DeviceNotAvailable | StreamError::StreamInvalidated => {
                backend.needs_rebuild = true;
                eprintln!("audio output invalidated: {error}");
            }
            other => {
                eprintln!("audio stream error: {other}");
            }
        }
    }
}

fn observe_output_route(backend: &mut PlaybackBackend) {
    let default_output = current_default_output_device();
    mark_output_route_change(backend, default_output);
}

fn should_poll_output_route(current: Option<&CurrentPlayback>) -> bool {
    current.is_some()
}

fn mark_output_route_change(
    backend: &mut PlaybackBackend,
    default_output: Option<OutputDeviceSnapshot>,
) {
    if output_route_changed(
        backend.last_default_output.as_ref(),
        default_output.as_ref(),
    ) {
        // This flag intentionally survives until the next sink rebuild. A route change that
        // happens while "Playing" is just as dangerous as one that happens while "Paused": macOS
        // may pause playback after the route flip, and by then the stream can already be stale.
        // Requiring a rebuild only for paused-time changes misses that sequence.
        backend.route_changed_since_rebuild = true;
        if should_rebuild_on_output_route_change(backend.current.as_ref()) {
            backend.needs_rebuild = true;
        }
        backend.last_default_output = default_output;
    }
}

fn should_rebuild_on_output_route_change(current: Option<&CurrentPlayback>) -> bool {
    current.is_some()
}

fn output_route_changed(
    last_default_output: Option<&OutputDeviceSnapshot>,
    default_output: Option<&OutputDeviceSnapshot>,
) -> bool {
    last_default_output != default_output
}

fn debug_force_rebuild_on_resume_enabled() -> bool {
    std::env::var_os("ORYX_DEBUG_FORCE_REBUILD_ON_RESUME").is_some()
}

fn current_default_output_device() -> Option<OutputDeviceSnapshot> {
    let host = rodio::cpal::default_host();
    let device = host.default_output_device()?;
    snapshot_output_device(&device)
}

fn snapshot_output_device(device: &rodio::cpal::Device) -> Option<OutputDeviceSnapshot> {
    let id = device.id().ok()?.to_string();
    let description = device
        .description()
        .map(|description| description.to_string())
        .unwrap_or_else(|_| "unknown output device".to_string());
    Some(OutputDeviceSnapshot { id, description })
}

fn playback_position(backend: &PlaybackBackend) -> Option<Duration> {
    match (
        backend.player.as_ref().map(Player::get_pos),
        backend.clock.elapsed(),
    ) {
        (Some(player_position), Some(clock_position)) => Some(player_position.max(clock_position)),
        (Some(player_position), None) => Some(player_position),
        (None, Some(clock_position)) => Some(clock_position),
        (None, None) => None,
    }
}

fn current_progressive_failure(current: &CurrentPlayback) -> Option<String> {
    match &current.source {
        PlaybackSource::GrowingFile { download, .. } => download.failure_message(),
        PlaybackSource::LocalFile(_) => None,
    }
}

fn current_progressive_retrying(current: &CurrentPlayback) -> bool {
    match &current.source {
        PlaybackSource::GrowingFile { download, .. } => download.is_retrying(),
        PlaybackSource::LocalFile(_) => false,
    }
}

fn publish_session(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
    track: &MediaSessionTrack,
    playback: PlaybackState,
    position: Option<Duration>,
    prime: bool,
) -> Result<()> {
    let Some(controls) = media_controls_mut(backend, media_controls) else {
        return Ok(());
    };

    media::set_media_metadata(controls, track);
    let progress = position.map(MediaPosition);
    if prime {
        media::set_media_playback(controls, MediaPlayback::Playing { progress });
    }
    media::set_media_playback(controls, media_playback(playback, progress));

    Ok(())
}

fn media_playback(playback: PlaybackState, progress: Option<MediaPosition>) -> MediaPlayback {
    match playback {
        PlaybackState::Idle => MediaPlayback::Stopped,
        PlaybackState::Paused => MediaPlayback::Paused { progress },
        PlaybackState::Playing => MediaPlayback::Playing { progress },
    }
}

fn media_controls_mut<'a>(
    backend: &'a mut Option<PlaybackBackend>,
    media_controls: &'a mut Option<MediaControls>,
) -> Option<&'a mut MediaControls> {
    if let Some(backend) = backend.as_mut() {
        if backend.media_controls.is_none() {
            backend.media_controls = media_controls.take();
        }
        backend.media_controls.as_mut()
    } else {
        media_controls.as_mut()
    }
}

fn update_media_position(
    backend: &mut Option<PlaybackBackend>,
    media_controls: &mut Option<MediaControls>,
    playback: PlaybackState,
    position: Duration,
) -> Result<()> {
    let Some(controls) = media_controls_mut(backend, media_controls) else {
        return Ok(());
    };

    media::set_media_playback(
        controls,
        media_playback(playback, Some(MediaPosition(position))),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn progressive_reader_falls_back_to_final_file_after_temp_rename() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let final_path = std::env::temp_dir().join(format!(
            "oryx-progressive-final-{}-{unique}.bin",
            std::process::id()
        ));
        let temp_path = final_path.with_extension("bin.part");

        let mut file = File::create(&final_path).expect("final file should be creatable");
        file.write_all(b"oryx")
            .expect("final file should be writable");
        drop(file);

        let download = crate::progressive::ProgressiveDownload::new();
        download.set_total_bytes(Some(4));
        download.finish(4);

        let mut reader = open_progressive_reader(&download, &temp_path, &final_path)
            .expect("reader should fall back to the final path");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut bytes)
            .expect("reader should read the final file");
        assert_eq!(bytes, b"oryx");

        fs::remove_file(&final_path).expect("final file should be removable");
    }

    #[test]
    fn treats_near_end_playback_as_finished_instead_of_stalled() {
        let position = Duration::from_millis(3_950);
        let duration = Some(4);

        let at_track_end = duration
            .map(|seconds| {
                position.saturating_add(TRACK_END_TOLERANCE) >= Duration::from_secs(seconds as u64)
            })
            .unwrap_or(false);

        assert!(at_track_end);
    }

    #[test]
    fn output_route_polling_only_runs_while_a_track_is_loaded() {
        assert!(!should_poll_output_route(None));
        let current = fixture_current_playback();
        assert!(should_poll_output_route(Some(&current)));
    }

    #[test]
    fn output_route_change_detects_device_difference() {
        let last_output = Some(OutputDeviceSnapshot {
            id: "speaker-a".to_string(),
            description: "Built-in Output".to_string(),
        });
        let new_output = Some(OutputDeviceSnapshot {
            id: "speaker-b".to_string(),
            description: "External Speaker".to_string(),
        });

        assert!(output_route_changed(
            last_output.as_ref(),
            new_output.as_ref(),
        ));
    }

    #[test]
    fn loaded_track_requires_rebuild_after_output_route_change() {
        assert!(!should_rebuild_on_output_route_change(None));
        let current = fixture_current_playback();
        assert!(should_rebuild_on_output_route_change(Some(&current)));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_output_priority_prefers_pipewire_then_pulse() {
        assert!(
            output_device_priority("PipeWire Sound Server", "pipewire")
                < output_device_priority("PulseAudio Sound Server", "pulse")
        );
        assert!(
            output_device_priority("PulseAudio Sound Server", "pulse")
                < output_device_priority("Built-in Audio", "hw:0,0")
        );
        assert!(
            output_device_priority("Built-in Audio", "hw:0,0")
                < output_device_priority("JACK Audio Connection Kit", "jack")
        );
    }

    fn fixture_current_playback() -> CurrentPlayback {
        CurrentPlayback {
            track: MediaSessionTrack {
                title: "Track 1".to_string(),
                artist: Some("Artist".to_string()),
                album: Some("Album".to_string()),
                artwork_url: None,
                duration_seconds: Some(180),
            },
            source: PlaybackSource::LocalFile(std::env::temp_dir().join("oryx-idle.mp3")),
            playback: PlaybackState::Paused,
        }
    }
}
