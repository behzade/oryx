use super::*;

fn quality(audio_format: AudioFormat, bitrate_bps: u32) -> AudioQuality {
    AudioQuality {
        audio_format: Some(audio_format),
        bitrate_bps: Some(bitrate_bps),
    }
}

#[test]
fn grades_lossless_as_lossless() {
    assert_eq!(
        normalized_audio_quality_grade(&AudioQuality {
            audio_format: Some(AudioFormat::Flac),
            bitrate_bps: None,
        }),
        Some(AudioQualityGrade::Lossless)
    );
}

#[test]
fn grades_common_lossy_formats_with_codec_aware_thresholds() {
    assert_eq!(
        normalized_audio_quality_grade(&quality(AudioFormat::Mp3, 320_000)),
        Some(AudioQualityGrade::High)
    );
    assert_eq!(
        normalized_audio_quality_grade(&quality(AudioFormat::Mp3, 256_000)),
        Some(AudioQualityGrade::Standard)
    );
    assert_eq!(
        normalized_audio_quality_grade(&quality(AudioFormat::Aac, 256_000)),
        Some(AudioQualityGrade::High)
    );
    assert_eq!(
        normalized_audio_quality_grade(&quality(AudioFormat::Opus, 160_000)),
        Some(AudioQualityGrade::High)
    );
    assert_eq!(
        normalized_audio_quality_grade(&quality(AudioFormat::Opus, 128_000)),
        Some(AudioQualityGrade::Standard)
    );
}

#[test]
fn summarizes_high_and_lossless_as_a_range_instead_of_flat_mixed() {
    assert_eq!(
        summarize_audio_quality_grades([
            AudioQualityGrade::Lossless,
            AudioQualityGrade::High,
            AudioQualityGrade::High,
        ]),
        Some(CollectionQualitySummary::Range {
            lowest: AudioQualityGrade::High,
            highest: AudioQualityGrade::Lossless,
        })
    );
}
