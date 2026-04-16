//! Owns everything about the audio directory on disk:
//!   - the segment-filename grammar (`segment_YYYYMMDD_HHMMSS.wav`)
//!   - cleanup primitives (by count and by age)
//!   - orphan discovery for restart recovery
//!   - the periodic cleanup timer
//!
//! Before this module existed, three other modules (`audio_engine`, `pipeline`,
//! `transcriber`) each knew a slightly-different slice of this grammar, and
//! cleanup constants were scattered. Centralizing makes the grammar one edit.

use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Files smaller than this are header-only or partial writes — skip them.
pub const MIN_SEGMENT_BYTES: u64 = 1_000;
/// How long to keep finalized WAVs around before deletion.
pub const AUDIO_MAX_AGE_SECS: u64 = 3_600;
/// Maximum number of WAV files to keep in the audio dir at any time.
pub const MAX_RETAINED_SEGMENTS: usize = 6;
/// Cleanup runs on this cadence on a dedicated timer thread (not per-segment).
pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Prefix of every recorded segment filename. `segment_` is what separates
/// "our WAVs" from any other WAV the user might drop into the audio dir.
pub const SEGMENT_PREFIX: &str = "segment_";
/// `chrono` format of the timestamp portion of a segment filename. Kept as a
/// compile-time constant so any change ripples to every parser in one place.
pub const SEGMENT_TIME_FMT: &str = "%Y%m%d_%H%M%S";

/// Build the canonical filename for a segment captured at `ts`. The audio
/// engine calls this when opening a new WAV writer.
pub fn format_segment_filename(ts: &DateTime<Utc>) -> String {
    format!("{SEGMENT_PREFIX}{}.wav", ts.format(SEGMENT_TIME_FMT))
}

/// Parse the UTC timestamp encoded in a segment filename, or `None` if the
/// file does not follow the grammar. Used by the orphan-recovery and dedup
/// paths; the audio engine uses its own live timestamp, not this.
pub fn parse_segment_timestamp(path: &Path) -> Option<DateTime<Utc>> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let date_part = stem.strip_prefix(SEGMENT_PREFIX)?;
    chrono::NaiveDateTime::parse_from_str(date_part, SEGMENT_TIME_FMT)
        .ok()
        .map(|dt| dt.and_utc())
}

/// List every WAV in `audio_dir`, oldest first. Used by cleanup + orphan scan.
fn list_wavs(audio_dir: &Path) -> Vec<PathBuf> {
    let mut segments: Vec<PathBuf> = std::fs::read_dir(audio_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    segments.sort();
    segments
}

/// Remove WAVs older than `max_age_secs` (by mtime).
pub fn cleanup_old_audio(audio_dir: &Path, max_age_secs: u64) {
    let now = std::time::SystemTime::now();
    for path in list_wavs(audio_dir) {
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let Ok(modified) = meta.modified() else { continue };
        let Ok(age) = now.duration_since(modified) else { continue };
        if age.as_secs() > max_age_secs {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("Failed to remove old audio {}: {e}", path.display());
            } else {
                log::info!("Cleaned old audio: {}", path.display());
            }
        }
    }
}

/// Keep at most `max_segments` WAVs — delete the oldest by filename (which
/// sorts chronologically because of the `YYYYMMDD_HHMMSS` encoding).
pub fn cleanup_old_segments(audio_dir: &Path, max_segments: usize) {
    let segments = list_wavs(audio_dir);
    if segments.len() <= max_segments {
        return;
    }
    let to_remove = segments.len() - max_segments;
    for path in segments.into_iter().take(to_remove) {
        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("Failed to remove old segment {}: {e}", path.display());
        }
    }
}

/// Find WAVs that may have been left over from a previous run (everything
/// except the most recent file — which may still be written to by another
/// instance or by the current audio engine). Caller is expected to filter
/// further by size and by dedup-against-DB.
pub fn find_orphan_segments(audio_dir: &Path) -> Vec<PathBuf> {
    let segments = list_wavs(audio_dir);
    if segments.len() <= 1 {
        return Vec::new();
    }
    segments[..segments.len() - 1].to_vec()
}

/// Spawn a background thread that runs both cleanup passes on a fixed cadence.
/// The handle is currently discarded — graceful shutdown is a separate audit
/// item (C4).
pub fn spawn_cleanup_timer(audio_dir: PathBuf) {
    std::thread::spawn(move || loop {
        std::thread::sleep(CLEANUP_INTERVAL);
        cleanup_old_segments(&audio_dir, MAX_RETAINED_SEGMENTS);
        cleanup_old_audio(&audio_dir, AUDIO_MAX_AGE_SECS);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_format_parse_roundtrip() {
        // #given a known UTC instant
        let ts = chrono::NaiveDateTime::parse_from_str("20240115_143022", SEGMENT_TIME_FMT)
            .unwrap()
            .and_utc();

        // #when we format to a filename and parse back
        let name = format_segment_filename(&ts);
        let round = parse_segment_timestamp(&PathBuf::from(format!("/audio/{name}"))).unwrap();

        // #then the seconds match
        assert_eq!(round, ts);
        assert!(name.starts_with(SEGMENT_PREFIX));
        assert!(name.ends_with(".wav"));
    }

    #[test]
    fn test_parse_non_segment_file_returns_none() {
        assert!(parse_segment_timestamp(Path::new("/tmp/random.wav")).is_none());
        assert!(parse_segment_timestamp(Path::new("/tmp/segment_not_a_date.wav")).is_none());
        assert!(parse_segment_timestamp(Path::new("/tmp/garbage")).is_none());
    }

    #[test]
    fn test_cleanup_old_segments_removes_excess() {
        // #given more segments than the retention limit
        let dir = TempDir::new().unwrap();
        for i in 0..5 {
            fs::write(
                dir.path().join(format!("segment_2024010{i}_120000.wav")),
                b"fake",
            )
            .unwrap();
        }

        // #when we cleanup keeping 3
        cleanup_old_segments(dir.path(), 3);

        // #then only 3 remain
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[test]
    fn test_cleanup_noop_when_under_limit() {
        let dir = TempDir::new().unwrap();
        for i in 0..2 {
            fs::write(
                dir.path().join(format!("segment_2024010{i}_120000.wav")),
                b"fake",
            )
            .unwrap();
        }
        cleanup_old_segments(dir.path(), 6);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn test_find_orphan_segments_skips_latest() {
        // #given three WAVs
        let dir = TempDir::new().unwrap();
        for i in 0..3 {
            fs::write(
                dir.path().join(format!("segment_2024010{i}_120000.wav")),
                b"fake",
            )
            .unwrap();
        }

        // #when we find orphans
        let orphans = find_orphan_segments(dir.path());

        // #then the latest is left alone
        assert_eq!(orphans.len(), 2);
    }

    #[test]
    fn test_find_orphans_empty_when_single_or_none() {
        let dir = TempDir::new().unwrap();
        assert!(find_orphan_segments(dir.path()).is_empty());
        fs::write(dir.path().join("segment_20240101_120000.wav"), b"fake").unwrap();
        assert!(find_orphan_segments(dir.path()).is_empty());
    }
}
