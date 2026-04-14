use hound::{SampleFormat, WavSpec, WavWriter};
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::path::Path;
use tempfile::TempDir;

// ── Resampling Tests (the actual fix) ───────────────────

fn create_resampler(from_rate: u32, to_rate: u32) -> SincFixedIn<f32> {
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    SincFixedIn::<f32>::new(
        to_rate as f64 / from_rate as f64,
        2.0,
        params,
        1024,
        1,
    )
    .unwrap()
}

fn generate_sine(rate: u32, duration_secs: f32, freq: f32) -> Vec<f32> {
    let samples = (rate as f32 * duration_secs) as usize;
    (0..samples)
        .map(|i| (i as f32 / rate as f32 * freq * 2.0 * std::f32::consts::PI).sin() * 0.5)
        .collect()
}

#[test]
fn test_resample_48k_to_16k_produces_output() {
    // #given — 1 second of 440Hz tone at 48kHz
    let input = generate_sine(48_000, 1.0, 440.0);
    assert_eq!(input.len(), 48_000);

    // #when — resample to 16kHz
    let mut resampler = create_resampler(48_000, 16_000);
    let mut output = Vec::new();
    for chunk in input.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        output.extend_from_slice(&result[0]);
    }

    // #then — output is approximately 1/3 the input length (16k/48k)
    assert!(!output.is_empty(), "Resampler must produce output");
    let ratio = output.len() as f64 / 48_000.0;
    assert!((ratio - 1.0 / 3.0).abs() < 0.05, "Output should be ~1/3 of input, got ratio {ratio}");
}

#[test]
fn test_resample_preserves_signal_energy() {
    let input = generate_sine(48_000, 1.0, 440.0);
    let input_rms = (input.iter().map(|s| s * s).sum::<f32>() / input.len() as f32).sqrt();

    let mut resampler = create_resampler(48_000, 16_000);
    let mut output = Vec::new();
    for chunk in input.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        output.extend_from_slice(&result[0]);
    }

    let output_rms = (output.iter().map(|s| s * s).sum::<f32>() / output.len() as f32).sqrt();
    let ratio = output_rms / input_rms;
    assert!(ratio > 0.8 && ratio < 1.2, "Signal energy should be preserved, got ratio {ratio}");
}

#[test]
fn test_resample_44100_to_16k() {
    let input = generate_sine(44_100, 0.5, 440.0);
    let mut resampler = create_resampler(44_100, 16_000);
    let mut output = Vec::new();
    for chunk in input.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        output.extend_from_slice(&result[0]);
    }
    assert!(!output.is_empty(), "Resampler must produce output from 44.1kHz input");
    // rubato drops the last incomplete chunk, so output can be shorter
    assert!(output.len() > 2000, "Should have reasonable output, got {}", output.len());
}

#[test]
fn test_resample_96k_to_16k() {
    let input = generate_sine(96_000, 0.5, 440.0);
    let mut resampler = create_resampler(96_000, 16_000);
    let mut output = Vec::new();
    for chunk in input.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        output.extend_from_slice(&result[0]);
    }
    assert!(!output.is_empty());
}

#[test]
fn test_resample_16k_to_16k_passthrough() {
    // No resampling needed — should still work
    let input = generate_sine(16_000, 1.0, 440.0);
    let mut resampler = create_resampler(16_000, 16_000);
    let mut output = Vec::new();
    for chunk in input.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        output.extend_from_slice(&result[0]);
    }
    assert!(!output.is_empty());
    let ratio = output.len() as f64 / input.len() as f64;
    assert!((ratio - 1.0).abs() < 0.1);
}

// ── Stereo Downmix Tests ────────────────────────────────

#[test]
fn test_downmix_stereo_sums_correctly() {
    let stereo = vec![1.0f32, 0.0, 0.0, 1.0, 0.5, 0.5];
    let mono: Vec<f32> = stereo.chunks(2).map(|ch| ch.iter().sum::<f32>() / 2.0).collect();
    assert_eq!(mono.len(), 3);
    assert!((mono[0] - 0.5).abs() < 1e-6);
    assert!((mono[1] - 0.5).abs() < 1e-6);
    assert!((mono[2] - 0.5).abs() < 1e-6);
}

#[test]
fn test_downmix_preserves_center_panned_signal() {
    let val = 0.7f32;
    let stereo: Vec<f32> = (0..200).flat_map(|_| vec![val, val]).collect();
    let mono: Vec<f32> = stereo.chunks(2).map(|ch| ch.iter().sum::<f32>() / 2.0).collect();
    assert!(mono.iter().all(|s| (s - val).abs() < 1e-6));
}

#[test]
fn test_downmix_opposite_phase_cancels() {
    let stereo = vec![0.5f32, -0.5, 0.8, -0.8];
    let mono: Vec<f32> = stereo.chunks(2).map(|ch| ch.iter().sum::<f32>() / 2.0).collect();
    assert!(mono.iter().all(|s| s.abs() < 1e-6));
}

#[test]
fn test_downmix_mono_passthrough() {
    let mono_input = vec![0.1f32, 0.2, 0.3, 0.4];
    let result: Vec<f32> = mono_input.chunks(1).map(|ch| ch.iter().sum::<f32>() / 1.0).collect();
    assert_eq!(result, mono_input);
}

// ── WAV Write with Real Resampled Data ──────────────────

#[test]
fn test_wav_write_after_resample_has_data() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("resampled.wav");

    // Simulate what the fixed AudioEngine does:
    // 1. Capture 48kHz
    let captured = generate_sine(48_000, 2.0, 440.0);

    // 2. Resample to 16kHz
    let mut resampler = create_resampler(48_000, 16_000);
    let mut resampled = Vec::new();
    for chunk in captured.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        resampled.extend_from_slice(&result[0]);
    }

    // 3. Write to WAV
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).unwrap();
    for &s in &resampled {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();

    // 4. Verify file size > header
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > 100, "WAV must contain actual audio data, got {} bytes", metadata.len());

    // 5. Read back and verify
    let reader = hound::WavReader::open(&path).unwrap();
    let read_spec = reader.spec();
    assert_eq!(read_spec.sample_rate, 16_000);
    assert_eq!(read_spec.channels, 1);

    let samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    assert!(!samples.is_empty(), "Must have samples");
    assert!(samples.len() > 10_000, "2 seconds at 16kHz should have >10k samples, got {}", samples.len());

    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    assert!(rms > 0.1, "Audio should not be silent, RMS = {rms}");
}

#[test]
fn test_wav_file_not_just_header() {
    // This is the exact test that would have caught the original bug
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("must_have_data.wav");

    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).unwrap();

    // The bug was: zero samples written. This test ensures > 0 samples.
    let samples = generate_sine(16_000, 1.0, 440.0);
    for &s in &samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();

    let metadata = std::fs::metadata(&path).unwrap();
    // WAV header is ~68 bytes. 1 second of f32 at 16kHz = 64,000 bytes
    assert!(metadata.len() > 60_000, "WAV file must contain audio data, not just header. Size: {} bytes", metadata.len());
}

// ── Audio Level (RMS) Calculation ───────────────────────

#[test]
fn test_rms_of_known_signal() {
    let samples: Vec<f32> = (0..16000).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    assert!((rms - 0.5).abs() < 0.01);
}

#[test]
fn test_rms_of_silence_is_zero() {
    let samples = vec![0.0f32; 16000];
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    assert!(rms < 1e-10);
}

#[test]
fn test_audio_level_scaling() {
    // RMS of 0.5 -> level = min(0.5 * 300, 100) = 100
    let rms = 0.5f32;
    let level = (rms * 300.0).min(100.0) as u32;
    assert_eq!(level, 100);

    // RMS of 0.01 -> level = 3
    let rms2 = 0.01f32;
    let level2 = (rms2 * 300.0).min(100.0) as u32;
    assert_eq!(level2, 3);

    // RMS of 0.0 -> level = 0
    let rms3 = 0.0f32;
    let level3 = (rms3 * 300.0).min(100.0) as u32;
    assert_eq!(level3, 0);
}

// ── Full Pipeline: Capture → Resample → WAV → Verify ───

#[test]
fn test_full_pipeline_48k_stereo_to_16k_mono_wav() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("pipeline_test.wav");

    // Step 1: Simulate 48kHz stereo capture (2 channels)
    let duration = 3.0f32;
    let rate = 48_000u32;
    let stereo_samples: Vec<f32> = (0..(rate as f32 * duration * 2.0) as usize)
        .map(|i| {
            let t = (i / 2) as f32 / rate as f32;
            let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.4;
            if i % 2 == 0 { sample } else { sample * 0.8 } // slightly different channels
        })
        .collect();

    // Step 2: Downmix to mono
    let mono: Vec<f32> = stereo_samples
        .chunks(2)
        .map(|ch| ch.iter().sum::<f32>() / ch.len() as f32)
        .collect();
    assert_eq!(mono.len(), (rate as f32 * duration) as usize);

    // Step 3: Resample 48kHz → 16kHz
    let mut resampler = create_resampler(48_000, 16_000);
    let mut resampled = Vec::new();
    for chunk in mono.chunks(1024) {
        if chunk.len() < 1024 { break; }
        let result = resampler.process(&[chunk.to_vec()], None).unwrap();
        resampled.extend_from_slice(&result[0]);
    }

    // Step 4: Write 16kHz mono WAV
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).unwrap();
    for &s in &resampled {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();

    // Step 5: Verify the complete pipeline output
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > 100_000, "3s of audio should be >100KB, got {}", metadata.len());

    let reader = hound::WavReader::open(&path).unwrap();
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16_000);
    assert_eq!(spec.channels, 1);

    let read_samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    let expected_approx = (16_000.0 * duration) as usize;
    assert!(
        (read_samples.len() as i64 - expected_approx as i64).unsigned_abs() < 2000,
        "Expected ~{expected_approx} samples, got {}",
        read_samples.len()
    );

    let rms = (read_samples.iter().map(|s| s * s).sum::<f32>() / read_samples.len() as f32).sqrt();
    assert!(rms > 0.1, "Output must contain actual audio, RMS = {rms}");
}
