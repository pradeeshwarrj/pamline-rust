//! Finding recordings and reading grid and start time from their names
//! (`pam_list_audio`, `pam_parse_filenames` in pamflow 0.6.0).

use chrono::{NaiveDateTime, TimeZone};
use std::path::Path;

/// Recordings pamflow works with: `.wav` / `.flac` (lower or upper case).
pub fn is_audio_name(name: &str) -> bool {
    [".wav", ".flac", ".WAV", ".FLAC"].iter().any(|e| name.ends_with(e))
}

/// All recordings under `dir`, like R's `list.files(dir, pattern, full.names =
/// TRUE, recursive = TRUE)`: `dir` + "/" + the relative path with forward
/// slashes, hidden files (".name") left out, sorted by full path in R's
/// (locale) order.
pub fn list_audio(dir: &str) -> Vec<String> {
    fn walk(base: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(base) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path(), &child, out),
                Ok(_) if is_audio_name(&name) => out.push(child),
                _ => {}
            }
        }
    }
    let mut rel = Vec::new();
    walk(Path::new(dir), "", &mut rel);
    let dir = dir.trim_end_matches(['/', '\\']);
    let mut out: Vec<String> = rel.into_iter().map(|r| format!("{dir}/{r}")).collect();
    crate::collate::sort(&mut out);
    out
}

/// Same as [`list_audio`] (R's order).
pub fn list_audio_r(dir: &str) -> Vec<String> {
    list_audio(dir)
}

/// One file name read with `filenames: pattern:` and `timezone:`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedName {
    pub file_name: String,
    pub grid: Option<String>,
    /// Start of the recording, local time of the time zone.
    pub start: Option<NaiveDateTime>,
    /// Start as UTC seconds since 1970 (what R stores in POSIXct).
    pub start_utc: Option<i64>,
}

impl ParsedName {
    pub fn valid(&self) -> bool {
        self.grid.is_some() && self.start.is_some()
    }
}

/// The pattern lacks a needed group, does not compile, or the zone is unknown.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum NameError {
    #[error("filenames$pattern must contain named groups: grid, date, time")]
    MissingGroups,
    #[error("filenames$pattern is not a valid regex: {0}")]
    BadPattern(String),
    #[error("unknown time zone: {0}")]
    BadZone(String),
}

/// `pam_parse_filenames`: names that do not match keep `None` (never dropped).
pub fn parse_filenames(files: &[String], pattern: &str, timezone: &str) -> Result<Vec<ParsedName>, NameError> {
    let re = fancy_regex::Regex::new(pattern).map_err(|e| NameError::BadPattern(e.to_string()))?;
    let names: Vec<&str> = re.capture_names().flatten().collect();
    if !["grid", "date", "time"].iter().all(|n| names.contains(n)) {
        return Err(NameError::MissingGroups);
    }
    let tz: chrono_tz::Tz = timezone.parse().map_err(|_| NameError::BadZone(timezone.to_string()))?;
    Ok(files
        .iter()
        .map(|f| {
            let file_name = f.rsplit(['/', '\\']).next().unwrap_or(f).to_string();
            let caps = re.captures(&file_name).ok().flatten();
            let grab = |n: &str| caps.as_ref().and_then(|c| c.name(n)).map(|m| m.as_str().to_string());
            let (grid, date, time) = (grab("grid"), grab("date"), grab("time"));
            let start = match (&date, &time) {
                (Some(d), Some(t)) => NaiveDateTime::parse_from_str(&format!("{d} {t}"), "%Y%m%d %H%M%S").ok(),
                _ => None,
            };
            // a local time that does not exist (clocks moved forward) has no start
            let start_utc = start.and_then(|s| tz.from_local_datetime(&s).earliest()).map(|t| t.timestamp());
            let start = start.filter(|_| start_utc.is_some());
            ParsedName { file_name, grid, start, start_utc }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAT: &str = r"^(?<grid>G\d{4})_(?<date>\d{8})_(?<time>\d{6})(_chunk\d+)?\.(flac|wav)$";

    #[test]
    fn names() {
        let files: Vec<String> =
            ["a/G0059_20260405_173058.wav", "b/G0059_20261305_000000.flac", "x.wav"].iter().map(|s| s.to_string()).collect();
        let p = parse_filenames(&files, PAT, "Asia/Kolkata").unwrap();
        assert!(p[0].valid());
        assert_eq!(p[0].grid.as_deref(), Some("G0059"));
        assert_eq!(p[0].start.unwrap().format("%Y-%m-%d %H:%M:%S").to_string(), "2026-04-05 17:30:58");
        assert_eq!(p[0].start_utc, Some(1775390458)); // 12:00:58 UTC
        assert!(!p[1].valid() && p[1].grid.is_some(), "month 13: grid read, no start");
        assert!(!p[2].valid() && p[2].grid.is_none());
        assert_eq!(parse_filenames(&files, r"^(?<grid>G\d+)", "UTC"), Err(NameError::MissingGroups));
    }
}
