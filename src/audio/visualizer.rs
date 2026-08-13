use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rodio::{ChannelCount, SampleRate, Source};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

pub const VISUALIZER_NOTES: usize = 12;
pub const VISUALIZER_OCTAVES: usize = 6;
const FFT_SIZE: usize = 4_096;
const FIRST_MIDI_NOTE: i32 = 36;
const LAST_MIDI_NOTE: i32 = FIRST_MIDI_NOTE + (VISUALIZER_OCTAVES * VISUALIZER_NOTES) as i32 - 1;
const LOW_FREQUENCY_HZ: f32 = 65.406_39;
const HIGH_FREQUENCY_HZ: f32 = 3_951.066_4;
const CONCERT_A_HZ: f32 = 440.0;
const CONCERT_A_MIDI: f32 = 69.0;
const MIN_LEVEL: f32 = 0.008;

type SampleBuffer = Box<[f32; FFT_SIZE]>;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisualizerSnapshot {
    pub level: f32,
    pub octaves: [[f32; VISUALIZER_NOTES]; VISUALIZER_OCTAVES],
}

struct AnalysisWork {
    samples: SampleBuffer,
    sample_rate: f32,
    generation: u64,
}

#[derive(Clone)]
pub struct AudioVisualizer {
    snapshot: Arc<Mutex<VisualizerSnapshot>>,
    work_tx: SyncSender<AnalysisWork>,
    recycled_rx: Arc<Mutex<Receiver<SampleBuffer>>>,
    generation: Arc<AtomicU64>,
}

impl Default for AudioVisualizer {
    fn default() -> Self {
        let snapshot = Arc::new(Mutex::new(VisualizerSnapshot::default()));
        let generation = Arc::new(AtomicU64::new(0));
        let (work_tx, work_rx) = mpsc::sync_channel::<AnalysisWork>(2);
        let (recycled_tx, recycled_rx) = mpsc::sync_channel::<SampleBuffer>(3);
        for _ in 0..3 {
            recycled_tx
                .try_send(Box::new([0.0; FFT_SIZE]))
                .expect("new visualizer buffer pool should have room");
        }
        let worker_snapshot = snapshot.clone();
        let worker_generation = generation.clone();

        thread::Builder::new()
            .name("audio-chroma".to_string())
            .spawn(move || {
                let mut analyzer = ChromaAnalyzer::new();
                let mut active_generation = 0;
                while let Ok(work) = work_rx.recv() {
                    if work.generation != active_generation {
                        analyzer.reset();
                        active_generation = work.generation;
                    }
                    let next_snapshot = analyzer.analyze(&work.samples, work.sample_rate);
                    if work.generation == worker_generation.load(Ordering::Acquire) {
                        *worker_snapshot.lock().expect("visualizer state poisoned") = next_snapshot;
                    }
                    let _ = recycled_tx.try_send(work.samples);
                }
            })
            .expect("failed to spawn audio chroma worker");

        Self {
            snapshot,
            work_tx,
            recycled_rx: Arc::new(Mutex::new(recycled_rx)),
            generation,
        }
    }
}

impl AudioVisualizer {
    pub fn snapshot(&self) -> VisualizerSnapshot {
        *self.snapshot.lock().expect("visualizer state poisoned")
    }

    pub fn clear(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self.snapshot.lock().expect("visualizer state poisoned") = VisualizerSnapshot::default();
        generation
    }

    fn submit(&self, samples: &mut SampleBuffer, sample_rate: f32, generation: u64) {
        let replacement = self
            .recycled_rx
            .lock()
            .expect("visualizer buffer pool poisoned")
            .try_recv()
            .unwrap_or_else(|_| Box::new([0.0; FFT_SIZE]));
        let completed = std::mem::replace(samples, replacement);
        let work = AnalysisWork {
            samples: completed,
            sample_rate,
            generation,
        };

        match self.work_tx.try_send(work) {
            Ok(()) => {}
            Err(TrySendError::Full(work) | TrySendError::Disconnected(work)) => {
                *samples = work.samples;
            }
        }
    }
}

struct ChromaAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    fft_buffer: Vec<Complex<f32>>,
    smoothed: VisualizerSnapshot,
}

impl ChromaAnalyzer {
    fn new() -> Self {
        let mut planner = FftPlanner::new();
        Self {
            fft: planner.plan_fft_forward(FFT_SIZE),
            fft_buffer: vec![Complex::ZERO; FFT_SIZE],
            smoothed: VisualizerSnapshot::default(),
        }
    }

    fn reset(&mut self) {
        self.smoothed = VisualizerSnapshot::default();
    }

    fn analyze(&mut self, samples: &[f32; FFT_SIZE], sample_rate: f32) -> VisualizerSnapshot {
        let square_sum = samples.iter().map(|sample| sample * sample).sum::<f32>();
        let rms = (square_sum / FFT_SIZE as f32).sqrt();
        for (index, (input, output)) in samples.iter().zip(self.fft_buffer.iter_mut()).enumerate() {
            let window = 0.5 - 0.5 * (TAU * index as f32 / (FFT_SIZE - 1) as f32).cos();
            *output = Complex::new(input * window, 0.0);
        }
        self.fft.process(&mut self.fft_buffer);

        let target_level = if rms < MIN_LEVEL {
            0.0
        } else {
            (rms * 3.8).sqrt().clamp(0.0, 1.0)
        };
        smooth_value(&mut self.smoothed.level, target_level, 0.68, 0.24);

        let nyquist = sample_rate * 0.5;
        let high_frequency = HIGH_FREQUENCY_HZ.min(nyquist);
        let bin_hz = sample_rate / FFT_SIZE as f32;
        let mut octave_energy = [[0.0_f32; VISUALIZER_NOTES]; VISUALIZER_OCTAVES];
        let start_bin = (LOW_FREQUENCY_HZ / bin_hz).ceil().max(1.0) as usize;
        let end_bin = ((high_frequency / bin_hz).floor() as usize).min(FFT_SIZE / 2 - 1);

        for bin in start_bin..=end_bin {
            let left = self.fft_buffer[bin - 1].norm_sqr();
            let center = self.fft_buffer[bin].norm_sqr();
            let right = self.fft_buffer[bin + 1].norm_sqr();
            if center < left || center <= right {
                continue;
            }
            let frequency = interpolated_peak_frequency(bin, bin_hz, left, center, right);
            let weight = center * 4.0 / (FFT_SIZE * FFT_SIZE) as f32;
            let (lower_midi, upper_weight) = midi_note_weights(frequency);
            if let Some((octave, note)) = octave_note_slot(lower_midi) {
                octave_energy[octave][note] += weight * (1.0 - upper_weight);
            }
            if let Some((octave, note)) = octave_note_slot(lower_midi + 1) {
                octave_energy[octave][note] += weight * upper_weight;
            }
        }

        let strongest_note = octave_energy
            .iter()
            .flatten()
            .copied()
            .fold(0.0_f32, f32::max);
        for (smoothed_octave, octave) in self.smoothed.octaves.iter_mut().zip(octave_energy) {
            for (smoothed, energy) in smoothed_octave.iter_mut().zip(octave) {
                let target = if rms < MIN_LEVEL || strongest_note == 0.0 {
                    0.0
                } else {
                    (energy / strongest_note).sqrt()
                };
                smooth_value(smoothed, target, 0.58, 0.20);
            }
        }
        self.smoothed
    }
}

fn interpolated_peak_frequency(
    bin: usize,
    bin_hz: f32,
    left_power: f32,
    center_power: f32,
    right_power: f32,
) -> f32 {
    let left = left_power.max(f32::MIN_POSITIVE).ln();
    let center = center_power.max(f32::MIN_POSITIVE).ln();
    let right = right_power.max(f32::MIN_POSITIVE).ln();
    let denominator = left - 2.0 * center + right;
    let offset = if denominator.abs() > f32::EPSILON {
        (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    (bin as f32 + offset) * bin_hz
}

fn midi_note_weights(frequency: f32) -> (i32, f32) {
    let midi_note = CONCERT_A_MIDI + 12.0 * (frequency / CONCERT_A_HZ).log2();
    let lower_midi = midi_note.floor();
    (lower_midi as i32, midi_note - lower_midi)
}

fn octave_note_slot(midi_note: i32) -> Option<(usize, usize)> {
    if !(FIRST_MIDI_NOTE..=LAST_MIDI_NOTE).contains(&midi_note) {
        return None;
    }
    let relative_note = (midi_note - FIRST_MIDI_NOTE) as usize;
    Some((
        relative_note / VISUALIZER_NOTES,
        midi_note.rem_euclid(VISUALIZER_NOTES as i32) as usize,
    ))
}

fn smooth_value(value: &mut f32, target: f32, rise: f32, fall: f32) {
    let amount = if target > *value { rise } else { fall };
    *value += (target - *value) * amount;
}

pub struct VisualizerSource<S> {
    input: S,
    visualizer: AudioVisualizer,
    channels: usize,
    sample_rate: f32,
    channel_samples_seen: usize,
    mono_sum: f32,
    sample_buffer: SampleBuffer,
    sample_index: usize,
    generation: u64,
}

impl<S: Source> VisualizerSource<S> {
    pub fn new(input: S, visualizer: AudioVisualizer) -> Self {
        let sample_rate = input.sample_rate().get() as f32;
        let channels = input.channels().get() as usize;
        let generation = visualizer.clear();
        Self {
            input,
            visualizer,
            channels,
            sample_rate,
            channel_samples_seen: 0,
            mono_sum: 0.0,
            sample_buffer: Box::new([0.0; FFT_SIZE]),
            sample_index: 0,
            generation,
        }
    }

    fn record_mono_sample(&mut self, sample: f32) {
        self.sample_buffer[self.sample_index] = sample;
        self.sample_index += 1;
        if self.sample_index == FFT_SIZE {
            self.visualizer
                .submit(&mut self.sample_buffer, self.sample_rate, self.generation);
            self.sample_index = 0;
        }
    }

    fn reset(&mut self) {
        self.channel_samples_seen = 0;
        self.mono_sum = 0.0;
        self.sample_index = 0;
        self.sample_buffer.fill(0.0);
    }
}

impl<S: Source> Iterator for VisualizerSource<S> {
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.input.next()?;
        self.mono_sum += sample;
        self.channel_samples_seen += 1;
        if self.channel_samples_seen >= self.channels {
            let mono_sample = self.mono_sum / self.channels as f32;
            self.channel_samples_seen = 0;
            self.mono_sum = 0.0;
            self.record_mono_sample(mono_sample);
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
        self.generation = self.visualizer.clear();
        self.reset();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::{NonZeroU16, NonZeroU32};
    use std::time::Instant;

    fn sine_wave(frequency: f32, sample_rate: u32) -> [f32; FFT_SIZE] {
        std::array::from_fn(|index| {
            (TAU * frequency * index as f32 / sample_rate as f32).sin() * 0.5
        })
    }

    #[test]
    fn source_passes_samples_through_and_records_chroma() {
        let sample_rate = 8_000;
        let samples = sine_wave(440.0, sample_rate).repeat(2);
        let source = SamplesBuffer::new(
            NonZeroU16::new(1).unwrap(),
            NonZeroU32::new(sample_rate).unwrap(),
            samples.clone(),
        );
        let visualizer = AudioVisualizer::default();

        let output = VisualizerSource::new(source, visualizer.clone()).collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(1);
        while visualizer.snapshot().level == 0.0 && Instant::now() < deadline {
            thread::yield_now();
        }

        assert_eq!(output, samples);
        assert!(visualizer.snapshot().level > 0.0);
        assert!(
            visualizer
                .snapshot()
                .octaves
                .iter()
                .flatten()
                .any(|level| *level > 0.0)
        );
    }

    #[test]
    fn same_note_in_different_octaves_uses_separate_lines() {
        let sample_rate = 48_000.0;
        let mut analyzer = ChromaAnalyzer::new();
        let low = analyzer.analyze(&sine_wave(110.0, sample_rate as u32), sample_rate);
        analyzer.reset();
        let high = analyzer.analyze(&sine_wave(440.0, sample_rate as u32), sample_rate);
        let low_peak = low.octaves[0]
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();
        let high_peak = high.octaves[2]
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .unwrap();

        assert_eq!(low_peak, 9);
        assert_eq!(high_peak, 9);
        assert!(low.octaves[0][9] > low.octaves[2][9]);
        assert!(high.octaves[2][9] > high.octaves[0][9]);
    }

    #[test]
    fn octave_slots_wrap_from_b_to_the_next_c() {
        let frequency = CONCERT_A_HZ * 2.0_f32.powf((59.5 - CONCERT_A_MIDI) / 12.0);
        let (b_midi, c_weight) = midi_note_weights(frequency);
        let b = octave_note_slot(b_midi).unwrap();
        let c = octave_note_slot(b_midi + 1).unwrap();

        assert_eq!(b, (1, 11));
        assert_eq!(c, (2, 0));
        assert!((c_weight - 0.5).abs() < 0.001);
    }

    #[test]
    fn silence_keeps_the_spectrum_at_rest() {
        let mut analyzer = ChromaAnalyzer::new();
        let snapshot = analyzer.analyze(&[0.0; FFT_SIZE], 48_000.0);

        assert_eq!(snapshot, VisualizerSnapshot::default());
    }
}
