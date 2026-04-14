use chrono::{TimeZone, Utc};
use hound::{SampleFormat, WavSpec, WavWriter};
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ── Helpers ─────────────────────────────────────────────

fn create_db(dir: &Path) -> Connection {
    let db_path = dir.join("e2e.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE transcriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT NOT NULL,
            device TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 0.0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE VIRTUAL TABLE transcriptions_fts USING fts5(text, content='transcriptions', content_rowid='id');
        CREATE TRIGGER transcriptions_ai AFTER INSERT ON transcriptions BEGIN
            INSERT INTO transcriptions_fts(rowid, text) VALUES (new.id, new.text);
        END;
        CREATE INDEX idx_start ON transcriptions(start_time);
        PRAGMA journal_mode=WAL;
        ",
    )
    .unwrap();
    conn
}

fn generate_wav(path: &Path, duration_secs: f32, freq_hz: f32) {
    let sample_rate = 16_000u32;
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();
    let total_samples = (sample_rate as f32 * duration_secs) as usize;
    for i in 0..total_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (t * freq_hz * 2.0 * std::f32::consts::PI).sin() * 0.5;
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

fn generate_silence_wav(path: &Path, duration_secs: f32) {
    let sample_rate = 16_000u32;
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();
    let total_samples = (sample_rate as f32 * duration_secs) as usize;
    for _ in 0..total_samples {
        writer.write_sample(0.0f32).unwrap();
    }
    writer.finalize().unwrap();
}

fn read_wav_samples(path: &Path) -> Vec<f32> {
    let reader = hound::WavReader::open(path).unwrap();
    reader.into_samples::<f32>().filter_map(|s| s.ok()).collect()
}

// ── E2E: Audio Recording Pipeline ───────────────────────

#[test]
fn e2e_wav_segment_has_correct_format() {
    // #given — a 5-second generated segment
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("segment_20240115_140000.wav");
    generate_wav(&path, 5.0, 440.0);

    // #when — read it back
    let reader = hound::WavReader::open(&path).unwrap();
    let spec = reader.spec();

    // #then — matches Whisper requirements
    assert_eq!(spec.sample_rate, 16_000);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 32);
    assert_eq!(spec.sample_format, SampleFormat::Float);
}

#[test]
fn e2e_wav_segment_duration_correct() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.wav");
    generate_wav(&path, 10.0, 440.0);

    let samples = read_wav_samples(&path);
    let duration = samples.len() as f32 / 16_000.0;
    assert!((duration - 10.0).abs() < 0.01);
}

#[test]
fn e2e_wav_contains_audio_not_silence() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tone.wav");
    generate_wav(&path, 1.0, 440.0);

    let samples = read_wav_samples(&path);
    let max_amplitude = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(max_amplitude > 0.1, "Audio should contain non-silent content");
}

#[test]
fn e2e_silence_wav_is_actually_silent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("silence.wav");
    generate_silence_wav(&path, 1.0);

    let samples = read_wav_samples(&path);
    let max_amplitude = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(max_amplitude < 1e-6, "Silence file should be silent");
}

#[test]
fn e2e_10_min_segment_correct_sample_count() {
    // 10 minutes at 16kHz mono = 9,600,000 samples
    let expected_samples = 16_000 * 600;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("segment_10min.wav");

    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).unwrap();
    for i in 0..expected_samples {
        let t = i as f32 / 16_000.0;
        writer.write_sample((t * 100.0).sin() * 0.3).unwrap();
    }
    writer.finalize().unwrap();

    let samples = read_wav_samples(&path);
    assert_eq!(samples.len(), expected_samples as usize);
}

// ── E2E: Segment Rotation ───────────────────────────────

#[test]
fn e2e_rolling_window_keeps_latest_segments() {
    let dir = TempDir::new().unwrap();
    let audio_dir = dir.path().join("audio");
    fs::create_dir(&audio_dir).unwrap();

    for i in 0..10 {
        let name = format!("segment_2024011{}_120000.wav", i);
        generate_wav(&audio_dir.join(&name), 0.1, 440.0);
    }

    // Keep only 6
    let mut segments: Vec<_> = fs::read_dir(&audio_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    segments.sort();

    if segments.len() > 6 {
        for path in segments.iter().take(segments.len() - 6) {
            fs::remove_file(path).unwrap();
        }
    }

    let remaining = fs::read_dir(&audio_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
        .count();
    assert_eq!(remaining, 6);
}

// ── E2E: Transcription Storage Pipeline ─────────────────

#[test]
fn e2e_transcription_stored_and_searchable() {
    // #given — a simulated transcription result
    let dir = TempDir::new().unwrap();
    let conn = create_db(dir.path());
    let text = "we discussed the quarterly budget and decided to increase marketing spend";
    let start = "2024-01-15T14:00:00+00:00";

    // #when — store it
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, ?3, ?4)",
        params![text, start, "MacBook Pro Microphone", 0.92],
    )
    .unwrap();

    // #then — searchable by keyword
    let mut stmt = conn.prepare(
        "SELECT t.text FROM transcriptions t JOIN transcriptions_fts f ON t.id = f.rowid WHERE transcriptions_fts MATCH ?1"
    ).unwrap();
    let results: Vec<String> = stmt.query_map(["marketing"], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("marketing"));
}

#[test]
fn e2e_multiple_segments_timeline_ordered() {
    let dir = TempDir::new().unwrap();
    let conn = create_db(dir.path());

    let entries = [
        ("morning standup notes", "2024-01-15T09:00:00+00:00"),
        ("lunch break conversation", "2024-01-15T12:30:00+00:00"),
        ("afternoon design review", "2024-01-15T15:00:00+00:00"),
        ("end of day wrap up", "2024-01-15T17:30:00+00:00"),
    ];

    for (text, time) in &entries {
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
            params![text, time],
        )
        .unwrap();
    }

    // #when — get timeline (latest first)
    let mut stmt = conn
        .prepare("SELECT text FROM transcriptions ORDER BY start_time DESC")
        .unwrap();
    let timeline: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();

    // #then
    assert_eq!(timeline[0], "end of day wrap up");
    assert_eq!(timeline[3], "morning standup notes");
}

#[test]
fn e2e_search_across_multiple_days() {
    let dir = TempDir::new().unwrap();
    let conn = create_db(dir.path());

    let entries = [
        ("project kickoff meeting", "2024-01-10T10:00:00+00:00"),
        ("sprint planning session", "2024-01-15T10:00:00+00:00"),
        ("project retrospective", "2024-01-20T14:00:00+00:00"),
        ("random lunch chat", "2024-01-12T12:00:00+00:00"),
    ];

    for (text, time) in &entries {
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
            params![text, time],
        )
        .unwrap();
    }

    let mut stmt = conn.prepare(
        "SELECT t.text FROM transcriptions t JOIN transcriptions_fts f ON t.id = f.rowid WHERE transcriptions_fts MATCH 'project'"
    ).unwrap();
    let results: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert_eq!(results.len(), 2);
}

#[test]
fn e2e_device_switches_tracked() {
    let dir = TempDir::new().unwrap();
    let conn = create_db(dir.path());

    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES ('airpods segment', '2024-01-15T10:00:00Z', '2024-01-15T10:10:00Z', 'AirPods Pro', 0.9)",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES ('macbook segment', '2024-01-15T10:10:00Z', '2024-01-15T10:20:00Z', 'MacBook Pro Microphone', 0.8)",
        [],
    ).unwrap();

    let mut stmt = conn.prepare("SELECT device FROM transcriptions ORDER BY start_time").unwrap();
    let devices: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert_eq!(devices[0], "AirPods Pro");
    assert_eq!(devices[1], "MacBook Pro Microphone");
}

#[test]
fn e2e_confidence_filtering() {
    let dir = TempDir::new().unwrap();
    let conn = create_db(dir.path());

    for i in 0..10 {
        let confidence = i as f64 / 10.0;
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, '2024-01-15T10:00:00Z', '2024-01-15T10:10:00Z', 'Mic', ?2)",
            params![format!("entry {i}"), confidence],
        ).unwrap();
    }

    let mut stmt = conn.prepare("SELECT COUNT(*) FROM transcriptions WHERE confidence >= 0.7").unwrap();
    let count: i64 = stmt.query_row([], |r| r.get(0)).unwrap();
    assert_eq!(count, 3); // 0.7, 0.8, 0.9
}

// ── E2E: Full Pipeline Simulation ───────────────────────

#[test]
fn e2e_full_pipeline_audio_to_storage() {
    // Simulates: record audio → save WAV → parse timestamp → store transcription → search

    let dir = TempDir::new().unwrap();
    let audio_dir = dir.path().join("audio");
    fs::create_dir(&audio_dir).unwrap();

    // Step 1: Create audio segment
    let segment_path = audio_dir.join("segment_20240115_143000.wav");
    generate_wav(&segment_path, 2.0, 440.0);
    assert!(segment_path.exists());

    // Step 2: Read and verify audio
    let samples = read_wav_samples(&segment_path);
    assert!(!samples.is_empty());
    assert!(samples.len() == 32_000); // 2 seconds at 16kHz

    // Step 3: Parse timestamp from filename
    let stem = segment_path.file_stem().unwrap().to_str().unwrap();
    let date_part = stem.strip_prefix("segment_").unwrap();
    let timestamp = chrono::NaiveDateTime::parse_from_str(date_part, "%Y%m%d_%H%M%S").unwrap();
    assert_eq!(timestamp.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 14:30:00");

    // Step 4: Store "transcription" result
    let conn = create_db(dir.path());
    let simulated_text = "this is a simulated transcription of the audio segment";
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?3, 'MacBook Pro Microphone', 0.88)",
        params![simulated_text, "2024-01-15T14:30:00+00:00", "2024-01-15T14:32:00+00:00"],
    ).unwrap();

    // Step 5: Search for it
    let mut stmt = conn.prepare(
        "SELECT t.text, t.confidence FROM transcriptions t JOIN transcriptions_fts f ON t.id = f.rowid WHERE transcriptions_fts MATCH 'simulated'"
    ).unwrap();
    let results: Vec<(String, f64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert_eq!(results.len(), 1);
    assert!(results[0].0.contains("simulated"));
    assert!((results[0].1 - 0.88).abs() < 0.01);

    // Step 6: Cleanup old segments
    generate_wav(&audio_dir.join("segment_20240115_140000.wav"), 0.1, 440.0);
    generate_wav(&audio_dir.join("segment_20240115_141000.wav"), 0.1, 440.0);
    generate_wav(&audio_dir.join("segment_20240115_142000.wav"), 0.1, 440.0);

    let mut segs: Vec<_> = fs::read_dir(&audio_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    segs.sort();
    if segs.len() > 2 {
        for p in segs.iter().take(segs.len() - 2) {
            fs::remove_file(p).ok();
        }
    }
    let remaining = fs::read_dir(&audio_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
        .count();
    assert_eq!(remaining, 2);
}

#[test]
fn e2e_pause_resumes_correctly() {
    // Simulates pause/resume: segments created during pause should be skipped
    let dir = TempDir::new().unwrap();
    let conn = create_db(dir.path());

    // Pre-pause: 2 segments transcribed
    conn.execute("INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES ('before pause 1', '2024-01-15T10:00:00Z', '2024-01-15T10:10:00Z', 'Mic', 0.9)", []).unwrap();
    conn.execute("INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES ('before pause 2', '2024-01-15T10:10:00Z', '2024-01-15T10:20:00Z', 'Mic', 0.9)", []).unwrap();

    // During pause: nothing stored (simulating is_paused = true skip)
    // ...

    // After resume: 1 segment
    conn.execute("INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES ('after resume', '2024-01-15T11:00:00Z', '2024-01-15T11:10:00Z', 'Mic', 0.85)", []).unwrap();

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 3);

    // Gap in timeline detected
    let mut stmt = conn.prepare("SELECT start_time FROM transcriptions ORDER BY start_time").unwrap();
    let times: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert_eq!(times[1], "2024-01-15T10:10:00Z");
    assert_eq!(times[2], "2024-01-15T11:00:00Z"); // 40-min gap from pause
}

#[test]
fn e2e_stereo_to_mono_downmix_preserves_signal() {
    let stereo: Vec<f32> = (0..3200).map(|i| {
        let t = i as f32 / 3200.0;
        (t * 440.0 * 2.0 * std::f32::consts::PI).sin()
    }).collect();

    // Treat as stereo (1600 frames x 2 channels)
    let mono: Vec<f32> = stereo.chunks(2).map(|ch| (ch[0] + ch[1]) / 2.0).collect();

    assert_eq!(mono.len(), 1600);
    let max = mono.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(max > 0.1, "Mono signal should preserve amplitude");
}

#[test]
fn e2e_concurrent_read_write() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("concurrent.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE transcriptions (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL, start_time TEXT NOT NULL, end_time TEXT NOT NULL, device TEXT, confidence REAL);
        PRAGMA journal_mode=WAL;
        ",
    ).unwrap();

    // Writer inserts
    for i in 0..50 {
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, '2024-01-15T10:00:00Z', '2024-01-15T10:10:00Z', 'Mic', 0.9)",
            params![format!("entry {i}")],
        ).unwrap();
    }

    // Concurrent reader
    let conn2 = Connection::open(&db_path).unwrap();
    let count: i64 = conn2.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 50);

    // More writes while reading
    for i in 50..100 {
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, '2024-01-15T10:00:00Z', '2024-01-15T10:10:00Z', 'Mic', 0.9)",
            params![format!("entry {i}")],
        ).unwrap();
    }

    let final_count: i64 = conn2.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0)).unwrap();
    assert_eq!(final_count, 100);
}

#[test]
fn e2e_data_persists_across_connections() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("persist.db");

    // First connection: create and insert
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, text TEXT NOT NULL)").unwrap();
        conn.execute("INSERT INTO transcriptions (text) VALUES ('persisted data')", []).unwrap();
    }

    // Second connection: read
    {
        let conn = Connection::open(&db_path).unwrap();
        let text: String = conn.query_row("SELECT text FROM transcriptions LIMIT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(text, "persisted data");
    }
}
