use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ── Audio Segment Cleanup ───────────────────────────────

fn create_fake_segments(dir: &Path, count: usize) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for i in 0..count {
        let name = format!("segment_2024011{:01}_{:06}.wav", i % 10, i * 100);
        let path = dir.join(&name);
        fs::write(&path, format!("fake wav {i}")).unwrap();
        paths.push(path);
    }
    paths.sort();
    paths
}

fn cleanup(dir: &Path, max: usize) {
    let mut segments: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    segments.sort();
    if segments.len() > max {
        let to_remove = segments.len() - max;
        for path in segments.into_iter().take(to_remove) {
            fs::remove_file(&path).ok();
        }
    }
}

#[test]
fn test_cleanup_removes_oldest_first() {
    let dir = TempDir::new().unwrap();
    let paths = create_fake_segments(dir.path(), 5);
    cleanup(dir.path(), 3);

    assert!(!paths[0].exists());
    assert!(!paths[1].exists());
    assert!(paths[2].exists());
    assert!(paths[3].exists());
    assert!(paths[4].exists());
}

#[test]
fn test_cleanup_exact_at_limit() {
    let dir = TempDir::new().unwrap();
    let paths = create_fake_segments(dir.path(), 3);
    cleanup(dir.path(), 3);
    for p in &paths {
        assert!(p.exists());
    }
}

#[test]
fn test_cleanup_below_limit() {
    let dir = TempDir::new().unwrap();
    let paths = create_fake_segments(dir.path(), 2);
    cleanup(dir.path(), 5);
    for p in &paths {
        assert!(p.exists());
    }
}

#[test]
fn test_cleanup_empty_dir() {
    let dir = TempDir::new().unwrap();
    cleanup(dir.path(), 3);
    let count = fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(count, 0);
}

#[test]
fn test_cleanup_max_zero_removes_all() {
    let dir = TempDir::new().unwrap();
    create_fake_segments(dir.path(), 5);
    cleanup(dir.path(), 0);
    let wav_count = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
        .count();
    assert_eq!(wav_count, 0);
}

#[test]
fn test_cleanup_ignores_non_wav_files() {
    let dir = TempDir::new().unwrap();
    create_fake_segments(dir.path(), 3);
    fs::write(dir.path().join("notes.txt"), "keep me").unwrap();
    fs::write(dir.path().join("data.json"), "{}").unwrap();

    cleanup(dir.path(), 1);

    assert!(dir.path().join("notes.txt").exists());
    assert!(dir.path().join("data.json").exists());
    let wav_count = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
        .count();
    assert_eq!(wav_count, 1);
}

#[test]
fn test_cleanup_large_number_of_segments() {
    let dir = TempDir::new().unwrap();
    create_fake_segments(dir.path(), 100);
    cleanup(dir.path(), 6);
    let wav_count = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
        .count();
    assert_eq!(wav_count, 6);
}

// ── Timestamp Parsing ───────────────────────────────────

fn parse_timestamp(filename: &str) -> Option<chrono::NaiveDateTime> {
    let stem = Path::new(filename).file_stem()?.to_str()?;
    let date_part = stem.strip_prefix("segment_")?;
    chrono::NaiveDateTime::parse_from_str(date_part, "%Y%m%d_%H%M%S").ok()
}

#[test]
fn test_parse_valid_timestamp() {
    let dt = parse_timestamp("segment_20240115_143022.wav").unwrap();
    assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 14:30:22");
}

#[test]
fn test_parse_midnight() {
    let dt = parse_timestamp("segment_20240101_000000.wav").unwrap();
    assert_eq!(dt.format("%H:%M:%S").to_string(), "00:00:00");
}

#[test]
fn test_parse_end_of_day() {
    let dt = parse_timestamp("segment_20241231_235959.wav").unwrap();
    assert_eq!(dt.format("%H:%M:%S").to_string(), "23:59:59");
}

#[test]
fn test_parse_invalid_format() {
    assert!(parse_timestamp("segment_badformat.wav").is_none());
}

#[test]
fn test_parse_no_prefix() {
    assert!(parse_timestamp("20240115_143022.wav").is_none());
}

#[test]
fn test_parse_empty_filename() {
    assert!(parse_timestamp("").is_none());
}

#[test]
fn test_parse_no_extension() {
    let dt = parse_timestamp("segment_20240115_143022");
    assert!(dt.is_some());
}

// ── WAV Spec Constants ──────────────────────────────────

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;
const SEGMENT_DURATION_SECS: u64 = 600;

#[test]
fn test_sample_rate_is_16k() {
    assert_eq!(SAMPLE_RATE, 16_000);
}

#[test]
fn test_mono_channel() {
    assert_eq!(CHANNELS, 1);
}

#[test]
fn test_segment_duration_is_10_min() {
    assert_eq!(SEGMENT_DURATION_SECS, 600);
}

#[test]
fn test_samples_per_segment() {
    let samples = SAMPLE_RATE as u64 * SEGMENT_DURATION_SECS;
    assert_eq!(samples, 9_600_000);
}

#[test]
fn test_segment_size_bytes_f32() {
    let bytes = SAMPLE_RATE as u64 * SEGMENT_DURATION_SECS * 4; // f32 = 4 bytes
    assert_eq!(bytes, 38_400_000); // ~36.6 MB
}

#[test]
fn test_one_hour_memory_f32() {
    let bytes = SAMPLE_RATE as u64 * 3600 * 4;
    assert_eq!(bytes, 230_400_000); // ~220 MB
}

// ── WAV File Write/Read ─────────────────────────────────

#[test]
fn test_wav_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.wav");

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0 * std::f32::consts::TAU).sin()).collect();
    for &s in &samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();

    let reader = hound::WavReader::open(&path).unwrap();
    let read_spec = reader.spec();
    assert_eq!(read_spec.sample_rate, 16_000);
    assert_eq!(read_spec.channels, 1);
    assert_eq!(read_spec.bits_per_sample, 32);

    let read_samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    assert_eq!(read_samples.len(), 1600);

    for (a, b) in samples.iter().zip(read_samples.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[test]
fn test_wav_empty_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.wav");

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let writer = hound::WavWriter::create(&path, spec).unwrap();
    writer.finalize().unwrap();

    let reader = hound::WavReader::open(&path).unwrap();
    let samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    assert!(samples.is_empty());
}

#[test]
fn test_wav_stereo_to_mono_downmix() {
    let stereo_samples: Vec<f32> = vec![0.5, -0.5, 0.8, -0.8, 0.0, 0.0];
    let mono: Vec<f32> = stereo_samples
        .chunks(2)
        .map(|ch| (ch[0] + ch[1]) / 2.0)
        .collect();
    assert_eq!(mono.len(), 3);
    assert!((mono[0] - 0.0).abs() < 1e-6);
    assert!((mono[1] - 0.0).abs() < 1e-6);
    assert!((mono[2] - 0.0).abs() < 1e-6);
}

#[test]
fn test_wav_single_sample() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("single.wav");
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    writer.write_sample(0.42f32).unwrap();
    writer.finalize().unwrap();

    let reader = hound::WavReader::open(&path).unwrap();
    let samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    assert_eq!(samples.len(), 1);
    assert!((samples[0] - 0.42).abs() < 1e-6);
}

#[test]
fn test_wav_max_amplitude() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("max.wav");
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    writer.write_sample(1.0f32).unwrap();
    writer.write_sample(-1.0f32).unwrap();
    writer.finalize().unwrap();

    let reader = hound::WavReader::open(&path).unwrap();
    let samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    assert!((samples[0] - 1.0).abs() < 1e-6);
    assert!((samples[1] - (-1.0)).abs() < 1e-6);
}

#[test]
fn test_wav_silence() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("silence.wav");
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for _ in 0..16_000 {
        writer.write_sample(0.0f32).unwrap();
    }
    writer.finalize().unwrap();

    let reader = hound::WavReader::open(&path).unwrap();
    let samples: Vec<f32> = reader.into_samples().filter_map(|s| s.ok()).collect();
    assert_eq!(samples.len(), 16_000);
    assert!(samples.iter().all(|&s| s.abs() < 1e-10));
}
