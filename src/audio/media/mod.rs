use std::ffi::c_void;
use std::sync::mpsc::Sender;
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

use super::MediaSessionTrack;

#[cfg(target_os = "macos")]
mod macos;

pub(super) fn init_media_controls(
    event_tx: Sender<MediaControlEvent>,
    hwnd: Option<*mut c_void>,
) -> Option<MediaControls> {
    let config = PlatformConfig {
        dbus_name: "oryx",
        display_name: "Oryx",
        hwnd,
    };

    let mut controls = match MediaControls::new(config) {
        Ok(controls) => controls,
        Err(error) => {
            eprintln!("media controls unavailable: {error}");
            return None;
        }
    };

    if let Err(error) = controls.attach(move |event| {
        let _ = event_tx.send(event);
    }) {
        eprintln!("failed to attach media controls: {error}");
        return None;
    }

    Some(controls)
}

pub(super) fn publish_now_playing(
    media_controls: &mut Option<MediaControls>,
    track: &MediaSessionTrack,
) {
    if let Some(controls) = media_controls.as_mut() {
        set_media_metadata(controls, track);
    }
}

pub(super) fn set_media_playback_state(
    media_controls: &mut Option<MediaControls>,
    playback: MediaPlayback,
) {
    if let Some(controls) = media_controls.as_mut() {
        set_media_playback(controls, playback);
    }
}

pub(super) fn set_media_metadata(controls: &mut MediaControls, track: &MediaSessionTrack) {
    let metadata = MediaMetadata {
        title: Some(track.title.as_str()),
        album: track.album.as_deref(),
        artist: track.artist.as_deref(),
        cover_url: sanitized_media_artwork_url(track.artwork_url.as_deref()),
        duration: track
            .duration_seconds
            .map(|seconds| Duration::from_secs(seconds.into())),
    };

    if let Err(error) = controls.set_metadata(metadata) {
        eprintln!("failed to publish media metadata: {error}");
    }
}

pub(super) fn set_media_playback(controls: &mut MediaControls, playback: MediaPlayback) {
    if let Err(error) = controls.set_playback(playback) {
        eprintln!("failed to update media playback state: {error}");
    }
}

#[cfg(target_os = "macos")]
fn sanitized_media_artwork_url(_cover_url: Option<&str>) -> Option<&str> {
    macos::sanitized_media_artwork_url()
}

#[cfg(not(target_os = "macos"))]
fn sanitized_media_artwork_url(cover_url: Option<&str>) -> Option<&str> {
    cover_url
}
