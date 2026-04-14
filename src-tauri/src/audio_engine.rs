use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use hound::{SampleFormat, WavSpec, WavWriter};
use parking_lot::Mutex;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;
const RESAMPLE_CHUNK: usize = 1024;

pub fn segment_duration_secs() -> u64 {
    std::env::var("WHISPER_SEGMENT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120) // 2 minutes — appended to hourly slots
}

pub static AUDIO_LEVEL: AtomicU32 = AtomicU32::new(0);

pub struct AudioEngine {
    _stream: cpal::Stream,
}

struct RecordingState {
    writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>>,
    samples_written: u64,
    segment_started: std::time::Instant,
    audio_dir: PathBuf,
    current_path: Option<PathBuf>,
    segment_tx: Sender<PathBuf>,
    resample_buf: VecDeque<f32>,
}

fn select_non_bluetooth_input(host: &cpal::Host) -> Option<cpal::Device> {
    use cpal::traits::HostTrait;

    let devices = host.input_devices().ok()?;
    let mut built_in: Option<cpal::Device> = None;
    let mut fallback: Option<cpal::Device> = None;

    for device in devices {
        let name = device.name().unwrap_or_default().to_lowercase();

        // Skip Bluetooth devices entirely — they cause A2DP→HFP codec switch
        if name.contains("airpods")
            || name.contains("bluetooth")
            || name.contains("beats")
            || name.contains("bose")
            || name.contains("sony wh")
            || name.contains("sony wf")
            || name.contains("jabra")
        {
            log::info!("Skipping Bluetooth input: {}", device.name().unwrap_or_default());
            continue;
        }

        // Prefer MacBook built-in mic
        if name.contains("macbook") || name.contains("built-in") || name.contains("internal") {
            log::info!("Found built-in mic: {}", device.name().unwrap_or_default());
            built_in = Some(device);
        } else if fallback.is_none() {
            fallback = Some(device);
        }
    }

    built_in.or(fallback)
}

impl AudioEngine {
    pub fn new(audio_dir: PathBuf, segment_tx: Sender<PathBuf>) -> Result<Self, String> {
        let host = cpal::default_host();

        // Prefer built-in mic over Bluetooth to avoid AirPods A2DP→HFP codec switch
        // which degrades audio output quality system-wide
        let device = select_non_bluetooth_input(&host)
            .or_else(|| host.default_input_device())
            .ok_or("no input device available")?;

        let device_name = device.name().unwrap_or_else(|_| "Unknown".into());
        log::info!("Using input device: {device_name}");

        let supported = device
            .default_input_config()
            .map_err(|e| format!("No supported input config: {e}"))?;

        let native_rate = supported.sample_rate().0;
        let native_channels = supported.channels();
        log::info!("Native config: {native_rate}Hz, {native_channels}ch");

        let config = cpal::StreamConfig {
            channels: native_channels,
            sample_rate: cpal::SampleRate(native_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let needs_resample = native_rate != TARGET_SAMPLE_RATE;
        let resampler: Option<Arc<Mutex<SincFixedIn<f32>>>> = if needs_resample {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 256,
                window: WindowFunction::BlackmanHarris2,
            };
            let r = SincFixedIn::<f32>::new(
                TARGET_SAMPLE_RATE as f64 / native_rate as f64,
                2.0,
                params,
                RESAMPLE_CHUNK,
                1,
            )
            .map_err(|e| format!("Resampler init failed: {e}"))?;
            log::info!("Resampler: {native_rate}Hz -> {TARGET_SAMPLE_RATE}Hz (chunk={RESAMPLE_CHUNK})");
            Some(Arc::new(Mutex::new(r)))
        } else {
            None
        };

        let state = Arc::new(Mutex::new(RecordingState {
            writer: None,
            samples_written: 0,
            segment_started: std::time::Instant::now(),
            audio_dir: audio_dir.clone(),
            current_path: None,
            segment_tx,
            resample_buf: VecDeque::with_capacity(RESAMPLE_CHUNK * 4),
        }));

        {
            let mut s = state.lock();
            open_new_segment(&mut s);
        }

        let state_clone = state.clone();
        let resampler_clone = resampler.clone();
        let native_ch = native_channels;
        let callback_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let callback_count_clone = callback_count.clone();

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if data.is_empty() {
                        return;
                    }

                    let count = callback_count_clone.fetch_add(1, Ordering::Relaxed);
                    if count == 0 || count == 100 || count == 1000 {
                        let max = data.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                        let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();
                        eprintln!("[AUDIO] callback #{count}: len={}, max={max:.6}, rms={rms:.6}, channels={native_ch}", data.len());
                    }

                    // Downmix to mono
                    let mono: Vec<f32> = if native_ch > 1 {
                        data.chunks(native_ch as usize)
                            .map(|ch| ch.iter().sum::<f32>() / ch.len() as f32)
                            .collect()
                    } else {
                        data.to_vec()
                    };

                    // Smoothed audio level for UI (asymmetric: fast attack, slow release)
                    let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
                    let new_level = (rms * 300.0).min(100.0) as u32;
                    let prev = AUDIO_LEVEL.load(Ordering::Relaxed);
                    let smoothed = if new_level > prev {
                        (prev as f32 * 0.4 + new_level as f32 * 0.6) as u32
                    } else {
                        (prev as f32 * 0.92 + new_level as f32 * 0.08) as u32
                    };
                    AUDIO_LEVEL.store(smoothed, Ordering::Relaxed);

                    let mut s = state_clone.lock();

                    if let Some(ref resampler) = resampler_clone {
                        // Buffer mono samples, process in chunks of RESAMPLE_CHUNK
                        s.resample_buf.extend(mono.iter());

                        while s.resample_buf.len() >= RESAMPLE_CHUNK {
                            let chunk: Vec<f32> = s.resample_buf.drain(..RESAMPLE_CHUNK).collect();
                            let mut r = resampler.lock();
                            match r.process(&[chunk], None) {
                                Ok(resampled) => {
                                    if !resampled.is_empty() && !resampled[0].is_empty() {
                                        write_samples(&mut s, &resampled[0]);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Resample error: {e}");
                                }
                            }
                        }
                    } else {
                        // No resampling needed — write directly
                        write_samples(&mut s, &mono);
                    }
                },
                |err| log::error!("Audio stream error: {err}"),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {e}"))?;

        log::info!("Audio stream playing — capturing at {native_rate}Hz {native_ch}ch, writing {TARGET_SAMPLE_RATE}Hz mono");
        eprintln!("[AUDIO] Stream playing: {native_rate}Hz {native_ch}ch -> {TARGET_SAMPLE_RATE}Hz mono, device={device_name}");

        Ok(Self { _stream: stream })
    }
}

fn write_samples(state: &mut RecordingState, samples: &[f32]) {
    if let Some(ref mut writer) = state.writer {
        for &sample in samples {
            if writer.write_sample(sample).is_err() {
                return;
            }
        }
        state.samples_written += samples.len() as u64;

        // Rotate by wall clock time — stays in sync with UI timer even if audio is throttled
        if state.segment_started.elapsed().as_secs() >= segment_duration_secs() {
            rotate_segment(state);
        }
    }
}

fn open_new_segment(state: &mut RecordingState) {
    let now = chrono::Utc::now();
    let filename = format!("segment_{}.wav", now.format("%Y%m%d_%H%M%S"));
    let path = state.audio_dir.join(&filename);

    let spec = WavSpec {
        channels: CHANNELS,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };

    match WavWriter::create(&path, spec) {
        Ok(writer) => {
            state.writer = Some(writer);
            state.samples_written = 0;
            state.segment_started = std::time::Instant::now();
            state.current_path = Some(path);
            log::info!("New segment: {filename}");
        }
        Err(e) => log::error!("Failed to create WAV: {e}"),
    }
}

fn rotate_segment(state: &mut RecordingState) {
    if let Some(writer) = state.writer.take() {
        let samples = state.samples_written;
        if let Err(e) = writer.finalize() {
            log::error!("WAV finalize failed: {e}");
        }
        if let Some(path) = state.current_path.take() {
            log::info!(
                "Segment complete: {} ({} samples, {:.1}s)",
                path.display(),
                samples,
                samples as f64 / TARGET_SAMPLE_RATE as f64
            );
            state.segment_tx.send(path).ok();
        }
    }
    open_new_segment(state);
}

pub fn cleanup_old_audio(audio_dir: &Path, max_age_secs: u64) {
    let now = std::time::SystemTime::now();
    if let Ok(entries) = std::fs::read_dir(audio_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "wav") {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        if let Ok(age) = now.duration_since(modified) {
                            if age.as_secs() > max_age_secs {
                                std::fs::remove_file(&path).ok();
                                log::info!("Cleaned old audio: {}", path.display());
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn cleanup_old_segments(audio_dir: &Path, max_segments: usize) {
    let mut segments: Vec<PathBuf> = std::fs::read_dir(audio_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
        .collect();

    segments.sort();

    if segments.len() > max_segments {
        let to_remove = segments.len() - max_segments;
        for path in segments.into_iter().take(to_remove) {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("Failed to remove old segment {}: {e}", path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_old_segments_removes_excess() {
        let dir = TempDir::new().unwrap();
        for i in 0..5 {
            fs::write(dir.path().join(format!("segment_2024010{i}_120000.wav")), b"fake").unwrap();
        }
        cleanup_old_segments(dir.path(), 3);
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_cleanup_noop_when_under_limit() {
        let dir = TempDir::new().unwrap();
        for i in 0..2 {
            fs::write(dir.path().join(format!("segment_2024010{i}_120000.wav")), b"fake").unwrap();
        }
        cleanup_old_segments(dir.path(), 6);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn test_resample_buffer_accumulation() {
        let mut buf: VecDeque<f32> = VecDeque::new();
        // Simulate small cpal callbacks (e.g., 256 samples)
        for _ in 0..5 {
            buf.extend((0..256).map(|i| (i as f32 * 0.01).sin()));
        }
        // 5 * 256 = 1280, should have 1 full chunk of 1024
        assert!(buf.len() >= RESAMPLE_CHUNK);
        let chunk: Vec<f32> = buf.drain(..RESAMPLE_CHUNK).collect();
        assert_eq!(chunk.len(), RESAMPLE_CHUNK);
        assert_eq!(buf.len(), 256); // 1280 - 1024 leftover
    }

    #[test]
    fn test_audio_level_ranges() {
        AUDIO_LEVEL.store(0, Ordering::Relaxed);
        assert_eq!(AUDIO_LEVEL.load(Ordering::Relaxed), 0);
        AUDIO_LEVEL.store(100, Ordering::Relaxed);
        assert_eq!(AUDIO_LEVEL.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_downmix_stereo() {
        let stereo = vec![1.0f32, -1.0, 0.5, 0.5];
        let mono: Vec<f32> = stereo.chunks(2).map(|ch| ch.iter().sum::<f32>() / 2.0).collect();
        assert!((mono[0] - 0.0).abs() < 1e-6);
        assert!((mono[1] - 0.5).abs() < 1e-6);
    }
}
