use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::{ChannelCount, SampleRate, Source};

pub const VISUALIZER_BUCKETS: usize = 64;
const VISUALIZER_UPDATES_PER_SECOND: usize = 30;

#[derive(Clone)]
pub struct AudioVisualizer {
    levels: Arc<Mutex<[f32; VISUALIZER_BUCKETS]>>,
}

impl Default for AudioVisualizer {
    fn default() -> Self {
        Self {
            levels: Arc::new(Mutex::new([0.0; VISUALIZER_BUCKETS])),
        }
    }
}

impl AudioVisualizer {
    pub fn snapshot(&self) -> [f32; VISUALIZER_BUCKETS] {
        *self.levels.lock().expect("visualizer state poisoned")
    }

    pub fn clear(&self) {
        self.levels
            .lock()
            .expect("visualizer state poisoned")
            .fill(0.0);
    }

    fn push(&self, level: f32) {
        let mut levels = self.levels.lock().expect("visualizer state poisoned");
        levels.rotate_left(1);
        levels[VISUALIZER_BUCKETS - 1] = level.clamp(0.0, 1.0);
    }
}

pub struct VisualizerSource<S> {
    input: S,
    visualizer: AudioVisualizer,
    samples_per_bucket: usize,
    samples_seen: usize,
    square_sum: f32,
}

impl<S: Source> VisualizerSource<S> {
    pub fn new(input: S, visualizer: AudioVisualizer) -> Self {
        let samples_per_second =
            input.sample_rate().get() as usize * input.channels().get() as usize;
        let samples_per_bucket = (samples_per_second / VISUALIZER_UPDATES_PER_SECOND).max(1);
        visualizer.clear();
        Self {
            input,
            visualizer,
            samples_per_bucket,
            samples_seen: 0,
            square_sum: 0.0,
        }
    }

    fn reset_bucket(&mut self) {
        self.samples_seen = 0;
        self.square_sum = 0.0;
    }
}

impl<S: Source> Iterator for VisualizerSource<S> {
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.input.next()?;
        self.square_sum += sample * sample;
        self.samples_seen += 1;
        if self.samples_seen >= self.samples_per_bucket {
            let rms = (self.square_sum / self.samples_seen as f32).sqrt();
            self.visualizer.push((rms * 2.4).sqrt());
            self.reset_bucket();
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.input.size_hint()
    }
}

impl<S: Source> Source for VisualizerSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.input.try_seek(position)?;
        self.visualizer.clear();
        self.reset_bucket();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZeroU16;
    use std::num::NonZeroU32;

    #[test]
    fn source_passes_samples_through_and_records_levels() {
        let samples = vec![0.5; 60];
        let source = SamplesBuffer::new(
            NonZeroU16::new(1).unwrap(),
            NonZeroU32::new(30).unwrap(),
            samples.clone(),
        );
        let visualizer = AudioVisualizer::default();

        let output = VisualizerSource::new(source, visualizer.clone()).collect::<Vec<_>>();

        assert_eq!(output, samples);
        assert!(visualizer.snapshot().iter().any(|level| *level > 0.0));
    }
}
