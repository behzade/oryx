use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use chromaprint::{Algorithm, Fingerprinter};
use rodio::{Decoder, Source};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioFingerprint {
    pub value: String,
    pub duration_seconds: u32,
}

#[derive(Clone, Debug, Default)]
pub struct FingerprintResolver;

impl FingerprintResolver {
    pub fn new() -> Self {
        Self
    }

    pub fn fingerprint_file(&self, path: &Path) -> Result<Option<AudioFingerprint>> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open audio file {}", path.display()))?;
        let mut decoder = Decoder::try_from(file)
            .with_context(|| format!("Failed to decode audio file {}", path.display()))?;

        let sample_rate = decoder.sample_rate().get();
        let num_channels = decoder.channels().get();
        let known_duration_seconds = decoder
            .total_duration()
            .map(|duration| duration.as_secs().min(u32::MAX as u64) as u32);

        let mut fingerprinter = Fingerprinter::new(Algorithm::default());
        fingerprinter
            .start(sample_rate, num_channels)
            .context("Failed to initialize chromaprint fingerprinter")?;

        let mut chunk = Vec::with_capacity(16 * 1024);
        let mut sample_count = 0_u64;
        for sample in &mut decoder {
            chunk.push(to_pcm_i16(sample));
            sample_count += 1;

            if chunk.len() >= 16 * 1024 {
                fingerprinter
                    .feed(&chunk)
                    .context("Failed to fingerprint PCM audio chunk")?;
                chunk.clear();
            }
        }

        if !chunk.is_empty() {
            fingerprinter
                .feed(&chunk)
                .context("Failed to fingerprint trailing PCM audio chunk")?;
        }

        if sample_count == 0 {
            return Ok(None);
        }

        fingerprinter
            .finish()
            .context("Failed to finalize audio fingerprint")?;

        let value = fingerprinter.encode();
        if value.is_empty() {
            return Ok(None);
        }

        let duration_seconds = known_duration_seconds.unwrap_or_else(|| {
            let frames = sample_count / u64::from(num_channels.max(1));
            (frames / u64::from(sample_rate.max(1))).min(u32::MAX as u64) as u32
        });

        Ok(Some(AudioFingerprint {
            value,
            duration_seconds,
        }))
    }
}

fn to_pcm_i16(sample: rodio::Sample) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * f32::from(i16::MAX);
    scaled.round() as i16
}
