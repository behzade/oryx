use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct PlaybackClock {
    base_position: Duration,
    started_at: Option<Instant>,
    paused_at: Option<Instant>,
    paused_duration: Duration,
}

impl PlaybackClock {
    pub(super) fn start(&mut self, position: Duration) {
        self.base_position = position;
        self.started_at = Some(Instant::now());
        self.paused_at = None;
        self.paused_duration = Duration::ZERO;
    }

    pub(super) fn pause(&mut self) {
        if self.started_at.is_some() && self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    pub(super) fn resume(&mut self) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_duration += paused_at.elapsed();
        }
    }

    pub(super) fn stop(&mut self) {
        self.base_position = Duration::ZERO;
        self.started_at = None;
        self.paused_at = None;
        self.paused_duration = Duration::ZERO;
    }

    pub(super) fn elapsed(&self) -> Option<Duration> {
        let started_at = self.started_at?;
        let end = self.paused_at.unwrap_or_else(Instant::now);
        Some(
            self.base_position
                + end
                    .duration_since(started_at)
                    .saturating_sub(self.paused_duration),
        )
    }
}
