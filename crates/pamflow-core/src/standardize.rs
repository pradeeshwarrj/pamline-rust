//! Stage 00: standardize the raw recordings (port of pamflow 0.6.0
//! `pam_standardize`, `pam_inspect_raw`, `pam_parse_datetime`,
//! `pam_extract_grid` and pamflow Desktop's `bridge/standardize.R`).
//!
//! Every raw file is inspected and triaged: recordings are copied (already in
//! the target format) or converted into the standardized folder; memos
//! (shorter than `min_duration_s`), non-audio files and unreadable audio are
//! copied to the isolated folder. Raw files are never changed or deleted.
//!
//! Headers are read as pamflow does (`pam_audio_info`): WAV with tuneR's
//! rules, FLAC from STREAMINFO; any other audio extension (mp3, m4a, ...) is
//! unreadable there and so triaged as corrupt. Conversion uses ffmpeg.

use crate::audio_files::is_audio_name;
use crate::audio_header::audio_info;
use crate::config::{json_to_yaml, modify_list, num, Config};
use crate::inspect::yaml_json;
use crate::rcsv::{write_table, Cell, Table};
use crate::rnum::rsum;
use crate::stage_log::StageLog;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use yaml_rust2::Yaml;

pub const STAGE: &str = "00_standardization";
pub const AUDIO_EXTS: [&str; 8] = ["wav", "flac", "mp3", "m4a", "ogg", "wma", "aiff", "aif"];

/// Stage 00 settings: the settings file's, with this run's overrides.
#[derive(Debug, Clone)]
pub struct Settings {
    pub raw_dir: String,
    pub standardized_dir: String,
    pub isolated_dir: String,
    pub target_format: String,
    pub target_hz: Yaml,
    pub channels: Yaml,
    pub min_duration_s: Yaml,
    pub organize_folders: String,
    pub rename_strategy: String,
    pub prefix: String,
    pub isolate_non_audio: bool,
    pub isolate_short: bool,
    pub timezone: String,
}

fn text(y: &Yaml, default: &str) -> String {
    match y {
        Yaml::String(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Real(r) => r.clone(),
        _ => default.to_string(),
    }
}

fn or(y: &Yaml, default: i64) -> Yaml {
    if matches!(y, Yaml::Null | Yaml::BadValue) {
        Yaml::Integer(default)
    } else {
        y.clone()
    }
}

impl Settings {
    /// `std_settings`: `overrides` is the job's `std` object.
    pub fn from(cfg: &Config, overrides: &Value) -> Settings {
        let mut sc = match &cfg.data["standardization"] {
            h @ Yaml::Hash(_) => h.clone(),
            _ => Yaml::Hash(Default::default()),
        };
        if overrides.as_object().is_some_and(|o| !o.is_empty()) {
            modify_list(&mut sc, &json_to_yaml(overrides));
        }
        let paths = &cfg.data["paths"];
        let p = |k: &str| paths[k].as_str().unwrap_or("").to_string();
        let flag = |y: &Yaml| !matches!(y, Yaml::Boolean(false));
        let tz = match (&sc["timezone"], &cfg.data["filenames"]["timezone"]) {
            (Yaml::String(s), _) => s.clone(),
            (_, Yaml::String(s)) => s.clone(),
            _ => "Asia/Kolkata".into(),
        };
        let s = |y: &Yaml, d: &str| if matches!(y, Yaml::Null | Yaml::BadValue) { d.to_string() } else { text(y, d) };
        Settings {
            raw_dir: p("audio_raw"),
            standardized_dir: p("audio_standardized"),
            isolated_dir: p("audio_isolated"),
            target_format: s(&sc["target_format"], "flac").to_lowercase(),
            target_hz: or(&sc["target_hz"], 48000),
            channels: or(&sc["channels"], 1),
            min_duration_s: or(&sc["min_duration_s"], 60),
            organize_folders: s(&sc["organize_folders"], "grid"),
            rename_strategy: s(&sc["rename_strategy"], "auto"),
            prefix: s(&sc["prefix"], ""),
            isolate_non_audio: flag(&sc["isolate_non_audio"]),
            isolate_short: flag(&sc["isolate_short"]),
            timezone: tz,
        }
    }

    /// The settings as the `std_state` event shows them.
    pub fn json(&self) -> Value {
        json!({
            "raw_dir": self.raw_dir, "standardized_dir": self.standardized_dir, "isolated_dir": self.isolated_dir,
            "target_format": self.target_format, "target_hz": yaml_json(&self.target_hz, false),
            "channels": yaml_json(&self.channels, false), "min_duration_s": yaml_json(&self.min_duration_s, false),
            "organize_folders": self.organize_folders, "rename_strategy": self.rename_strategy, "prefix": self.prefix,
            "isolate_non_audio": self.isolate_non_audio, "isolate_short": self.isolate_short, "timezone": self.timezone,
        })
    }
}

/// R's `tolower(normalizePath(p, winslash = "/", mustWork = FALSE))`; "" stays "".
fn std_norm(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    crate::config::canonical_slash(Path::new(p)).to_lowercase()
}

/// Problems that make running unsafe (`std_problems`); empty = fine.
pub fn problems(s: &Settings) -> Vec<String> {
    let (raw, out, iso) = (std_norm(&s.raw_dir), std_norm(&s.standardized_dir), std_norm(&s.isolated_dir));
    let mut probs = Vec::new();
    if raw.is_empty() || !Path::new(&s.raw_dir).is_dir() {
        probs.push(format!("Raw folder not found: {}", s.raw_dir));
    }
    let inside = |a: &str, b: &str| !a.is_empty() && !b.is_empty() && (a == b || a.starts_with(&format!("{b}/")));
    if inside(&out, &raw) || inside(&raw, &out) {
        probs.push("The raw and standardized folders must be separate (neither inside the other).".into());
    }
    if inside(&iso, &raw) || inside(&raw, &iso) {
        probs.push("The raw and isolated folders must be separate (neither inside the other).".into());
    }
    probs
}

pub fn plan_path(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.data["paths"]["metadata"].as_str().unwrap_or("")).join("00_standardization_plan.csv")
}

pub fn log_path(s: &Settings) -> PathBuf {
    PathBuf::from(&s.standardized_dir).join("standardization_log.csv")
}

// ---- inspecting the raw folder ---------------------------------------------------

/// One raw file after inspection (`pam_inspect_raw`).
#[derive(Debug, Clone)]
pub struct Inspected {
    pub file_path: String,
    pub rel_path: String,
    pub filename: String,
    pub ext: String,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub parsed_datetime: String,
    pub grid: String,
    pub category: &'static str,
}

/// The order Windows (NTFS) lists a folder in: names compared by their upper-case
/// UTF-16 code units, so `rec-z` < `rec1` < `recorder_x` < `rec_one` < `z` < `_u` < `~t`.
fn ntfs_key(name: &str) -> Vec<u16> {
    name.chars()
        .flat_map(|c| {
            let mut up = c.to_uppercase();
            let mapped = match (up.next(), up.next()) {
                (Some(u), None) => u, // simple, one-to-one upper-casing only
                _ => c,
            };
            let mut buf = [0u16; 2];
            mapped.encode_utf16(&mut buf).to_vec()
        })
        .collect()
}

/// `fs::dir_ls(dir, recurse = TRUE, type = "file")`: hidden files left out,
/// folder by folder in the file system's order, a sub-folder listed in place
/// (`c` < `c/z.wav` < `c.wav`), paths `dir/relative`.
pub fn dir_ls(dir: &str) -> Vec<String> {
    fn walk(base: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(base) else { return };
        let mut items: Vec<(String, bool, bool)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    return None;
                }
                let t = e.file_type().ok()?;
                Some((name, t.is_dir(), t.is_file()))
            })
            .collect();
        items.sort_by_cached_key(|(n, _, _)| ntfs_key(n));
        for (name, is_dir, is_file) in items {
            let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if is_dir {
                walk(&base.join(&name), &child, out);
            } else if is_file {
                out.push(child);
            }
        }
    }
    let mut rel = Vec::new();
    walk(Path::new(dir), "", &mut rel);
    let dir = dir.trim_end_matches(['/', '\\']);
    rel.into_iter().map(|r| format!("{dir}/{r}")).collect()
}

/// `fs::path_ext`: the text after the last dot of the file name ("" if none).
fn path_ext(name: &str) -> &str {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i + 1 < base.len() => &base[i + 1..],
        _ => "",
    }
}

fn path_ext_remove(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && !name[i + 1..].contains(['/', '\\']) => &name[..i],
        _ => name,
    }
}

/// `pam_extract_grid`: the grid from the file-name pattern, else the text before
/// the first underscore, else "unassigned".
pub fn extract_grid(filename: &str, pattern: Option<&str>) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    if let Some(p) = pattern.filter(|p| !p.is_empty()) {
        if let Ok(re) = fancy_regex::Regex::new(p) {
            if let Ok(Some(caps)) = re.captures(base) {
                for name in ["grid", "recorder", "site", "deployment"] {
                    if re.capture_names().flatten().any(|n| n == name) {
                        if let Some(m) = caps.name(name).filter(|m| !m.as_str().is_empty()) {
                            return m.as_str().to_string();
                        }
                        break;
                    }
                }
            }
        }
    }
    let re = fancy_regex::Regex::new(r"^[A-Za-z0-9\-]+(?=_)").unwrap();
    if let Ok(Some(m)) = re.find(base) {
        if !m.as_str().is_empty() {
            return m.as_str().to_string();
        }
    }
    "unassigned".into()
}

/// `pam_parse_datetime`: `YYYYMMDD_HHMMSS` from the file name, else from the
/// file's modification time.
pub fn parse_datetime(filename: &str, file_path: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let stem = path_ext_remove(base);
    let find = |pat: &str| -> Option<String> {
        let re = fancy_regex::Regex::new(pat).ok()?;
        re.find(stem).ok().flatten().map(|m| m.as_str().to_string())
    };
    if let Some(m) = find("20[0-9]{2}[01][0-9][0-3][0-9][-_T][0-2][0-9][0-5][0-9][0-5][0-9]") {
        let clean: String = m.chars().filter(char::is_ascii_digit).collect();
        if clean.len() == 14 {
            return format!("{}_{}", &clean[..8], &clean[8..14]);
        }
    }
    if let Some(m) = find("20[0-9]{2}[01][0-9][0-3][0-9][0-2][0-9][0-5][0-9][0-5][0-9]") {
        return format!("{}_{}", &m[..8], &m[8..14]);
    }
    if let Some(m) = find("[2-3][0-9][01][0-9][0-3][0-9][0-2][0-9][0-5][0-9][0-5][0-9]") {
        return format!("20{}_{}", &m[..6], &m[6..12]);
    }
    let mtime = std::fs::metadata(file_path).and_then(|m| m.modified()).ok();
    let t: chrono::DateTime<chrono::Local> = mtime.map(Into::into).unwrap_or_else(chrono::Local::now);
    t.format("%Y%m%d_%H%M%S").to_string()
}

/// `pam_inspect_raw`. `tick` is called once per audio file whose header is read.
pub fn inspect_raw(
    s: &Settings,
    pattern: Option<&str>,
    log: &mut StageLog,
    tick: &mut dyn FnMut(),
) -> Result<Vec<Inspected>, String> {
    let raw = &s.raw_dir;
    if !Path::new(raw).is_dir() {
        let msg = format!("Raw directory does not exist: {raw}");
        log.log("ERROR", &msg);
        return Err(msg);
    }
    log.info(&format!("Scanning raw directory: {raw} (recursive = TRUE)"));
    let files = dir_ls(raw);
    if files.is_empty() {
        log.warn(&format!("No files found in raw directory: {raw}"));
        return Ok(Vec::new());
    }
    let min = num(&s.min_duration_s).unwrap_or(60.0);
    let prefix_len = raw.trim_end_matches(['/', '\\']).len() + 1;
    let out: Vec<Inspected> = files
        .iter()
        .map(|fp| {
            let rel = fp[prefix_len.min(fp.len())..].to_string();
            let filename = fp.rsplit('/').next().unwrap_or(fp).to_string();
            let ext = path_ext(&filename).to_lowercase();
            let grid = extract_grid(&filename, pattern);
            let dt = parse_datetime(&filename, fp);
            let mk = |sr, ch, category| Inspected {
                file_path: fp.clone(),
                rel_path: rel.clone(),
                filename: filename.clone(),
                ext: ext.clone(),
                sample_rate: sr,
                channels: ch,
                parsed_datetime: dt.clone(),
                grid: grid.clone(),
                category,
            };
            if !AUDIO_EXTS.contains(&ext.as_str()) {
                return mk(None, None, "isolated_non_audio");
            }
            tick();
            let (info, _warnings) = audio_info(Path::new(fp));
            if !info.readable {
                return mk(None, None, "corrupt");
            }
            let short = info.duration_s.is_some_and(|d| d < min);
            mk(info.sample_rate, info.channels, if short { "isolated_memo" } else { "standardizable" })
        })
        .collect();
    let n = |c: &str| out.iter().filter(|r| r.category == c).count();
    log.info(&format!(
        "Inspection complete. Total files: {} | Standardizable: {} | Non-audio: {} | Memos: {}",
        out.len(),
        n("standardizable"),
        n("isolated_non_audio"),
        n("isolated_memo")
    ));
    Ok(out)
}

// ---- running ---------------------------------------------------------------------------

/// One row of the plan / the standardization log.
#[derive(Debug, Clone)]
pub struct Row {
    pub source_path: String,
    pub dest_path: Option<String>,
    pub category: &'static str,
    pub action: String,
    pub status: &'static str,
    pub message: String,
}

impl Row {
    fn cells(&self) -> Vec<Cell> {
        vec![
            Cell::Str(self.source_path.clone()),
            Cell::opt_str(self.dest_path.as_deref()),
            Cell::Str(self.category.into()),
            Cell::Str(self.action.clone()),
            Cell::Str(self.status.into()),
            Cell::Str(self.message.clone()),
        ]
    }
}

pub const ROW_COLS: [&str; 6] = ["source_path", "dest_path", "category", "action", "status", "message"];

pub fn write_rows(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    let cells: Vec<Vec<Cell>> = rows.iter().map(Row::cells).collect();
    write_table(path, &ROW_COLS, &cells)
}

/// R's `as.character(<number>)` for the values shown in messages.
pub fn r_number(y: &Yaml) -> String {
    match y {
        Yaml::Integer(i) => i.to_string(),
        other => match num(other) {
            Some(x) if x == x.trunc() && x.abs() < 1e15 => {
                let fixed = format!("{}", x as i64);
                let digits = fixed.trim_start_matches('-').trim_end_matches('0');
                let sci_len = digits.len() + usize::from(digits.len() > 1) + 4;
                if sci_len < fixed.trim_start_matches('-').len() {
                    let e = fixed.trim_start_matches('-').len() - 1;
                    let mant = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.to_string() };
                    format!("{}{}e+{:02}", if x < 0.0 { "-" } else { "" }, mant, e)
                } else {
                    fixed
                }
            }
            Some(x) => crate::rnum::fwrite_num(x),
            None => text(other, ""),
        },
    }
}

/// Convert `src` into `dest` (format from its extension) at `hz` and
/// `channels`, like `av::av_audio_convert`, with ffmpeg.
pub fn convert(src: &str, dest: &str, hz: i64, channels: i64) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i", src, "-vn", "-ac"])
        .arg(channels.to_string())
        .arg("-ar")
        .arg(hz.to_string())
        .arg(dest);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("could not start ffmpeg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed").trim().to_string())
    }
}

/// What the run reports while it works (the app's trace counters).
pub trait Progress {
    /// A header was read.
    fn header_read(&mut self);
    /// A file was written (copied or converted), `what` is "copied"/"converted".
    fn file_written(&mut self, dest: &str, what: &str);
    /// A conversion failed (from the ERROR log line).
    fn conversion_failed(&mut self, src: &str);
}

fn copy_file(src: &str, dest: &str) -> Result<(), String> {
    if let Some(dir) = Path::new(dest).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create directory '{}': {e}", dir.display()))?;
    }
    std::fs::copy(src, dest).map(|_| ()).map_err(|e| format!("Failed to copy '{src}' to '{dest}': {e}"))
}

/// `pam_standardize`: the plan (`dry_run`) or the run. Returns the log table.
pub fn standardize(
    cfg: &Config,
    s: &Settings,
    dry_run: bool,
    log: &mut StageLog,
    progress: &mut dyn Progress,
) -> Result<Vec<Row>, String> {
    log.info(&format!("Starting Stage 00 Standardization (dry_run = {})", if dry_run { "TRUE" } else { "FALSE" }));
    log.info(&format!(
        "Target format: {} | Target Hz: {} | Layout: {} | Timezone: {}",
        s.target_format,
        r_number(&s.target_hz),
        s.organize_folders,
        s.timezone
    ));
    let pattern = cfg.data["filenames"]["pattern"].as_str().map(str::to_string);
    let mut inspected = {
        let mut tick = || progress.header_read();
        inspect_raw(s, pattern.as_deref(), log, &mut tick)?
    };
    if inspected.is_empty() {
        log.warn("No files to process.");
        return Ok(Vec::new());
    }
    // the app makes the grid folder the prefix when one is set (for this run only)
    if !s.prefix.is_empty() {
        for r in inspected.iter_mut() {
            r.grid = s.prefix.clone();
        }
    }
    let target_hz = num(&s.target_hz);
    let channels = num(&s.channels);
    let mut rows: Vec<Row> = Vec::with_capacity(inspected.len());
    for row in &inspected {
        let (src, rel) = (&row.file_path, &row.rel_path);
        let isolate = |kind: &str, planned: &str, done: &str, message: &str, status: &'static str, log: &mut StageLog,
                       progress: &mut dyn Progress|
         -> Result<Row, String> {
            let dest = format!("{}/{kind}/{rel}", s.isolated_dir);
            if !dry_run {
                copy_file(src, &dest)?;
                progress.file_written(&dest, "copied");
            }
            let _ = log;
            Ok(Row {
                source_path: src.clone(),
                dest_path: Some(dest),
                category: row.category,
                action: if dry_run { planned } else { done }.into(),
                status,
                message: message.into(),
            })
        };
        let retained = |message: &str| Row {
            source_path: src.clone(),
            dest_path: None,
            category: row.category,
            action: "SKIPPED".into(),
            status: "SUCCESS",
            message: message.into(),
        };
        match row.category {
            "isolated_non_audio" => {
                rows.push(if s.isolate_non_audio {
                    isolate("non_audio", "PLANNED_ISOLATE_NON_AUDIO", "ISOLATED_NON_AUDIO",
                        "Non-audio file moved to isolated directory", "SUCCESS", log, progress)?
                } else {
                    retained("Non-audio file retained in raw")
                });
                continue;
            }
            "isolated_memo" => {
                rows.push(if s.isolate_short {
                    isolate("memos", "PLANNED_ISOLATE_MEMO", "ISOLATED_MEMO",
                        "Short audio memo moved to isolated directory", "SUCCESS", log, progress)?
                } else {
                    retained("Short audio memo retained in raw")
                });
                continue;
            }
            "corrupt" => {
                rows.push(isolate("corrupt", "PLANNED_ISOLATE_CORRUPT", "ISOLATED_CORRUPT",
                    "Corrupt/unreadable audio stream isolated", "WARN", log, progress)?);
                continue;
            }
            _ => {}
        }
        let base_name =
            if s.rename_strategy == "auto" { row.parsed_datetime.clone() } else { path_ext_remove(&row.filename).to_string() };
        let grid = &row.grid;
        let starts_with_grid = fancy_regex::Regex::new(&format!("^{grid}")).ok().is_some_and(|re| re.is_match(&row.filename).unwrap_or(false));
        let final_filename = if !s.prefix.is_empty() {
            format!("{}_{}.{}", s.prefix, base_name, s.target_format)
        } else if starts_with_grid {
            format!("{}.{}", path_ext_remove(&row.filename), s.target_format)
        } else if grid != "unassigned" {
            format!("{grid}_{base_name}.{}", s.target_format)
        } else {
            format!("{base_name}.{}", s.target_format)
        };
        let dt = &row.parsed_datetime;
        let sub = |a: usize, b: usize| dt.get(a..b).unwrap_or("").to_string();
        let dest_dir = match s.organize_folders.as_str() {
            "grid" => format!("{}/{grid}", s.standardized_dir),
            "grid_date" => format!("{}/{grid}/{}/{}/{}", s.standardized_dir, sub(0, 4), sub(4, 6), sub(6, 8)),
            "date" => format!("{}/{}/{}/{}", s.standardized_dir, sub(0, 4), sub(4, 6), sub(6, 8)),
            "preserve" => {
                let d = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
                format!("{}/{d}", s.standardized_dir)
            }
            _ => s.standardized_dir.clone(),
        };
        let dest_path = format!("{dest_dir}/{final_filename}");
        let src_hz = row.sample_rate;
        let actual_hz = match (target_hz, src_hz) {
            (Some(t), Some(sr)) if t < sr as f64 => t,
            (_, Some(sr)) => sr as f64,
            (Some(t), None) => t,
            _ => 0.0,
        };
        let compliant = row.ext == s.target_format
            && src_hz.is_none_or(|sr| sr as f64 == actual_hz)
            && match (row.channels, channels) {
                (Some(c), Some(want)) => c as f64 == want,
                _ => true,
            };
        if compliant {
            if !dry_run {
                copy_file(src, &dest_path)?;
                progress.file_written(&dest_path, "copied");
            }
            rows.push(Row {
                source_path: src.clone(),
                dest_path: Some(dest_path),
                category: row.category,
                action: if dry_run { "PLANNED_COPY_COMPLIANT" } else { "COPIED_COMPLIANT" }.into(),
                status: "SUCCESS",
                message: "Compliant file copied directly without re-encoding".into(),
            });
        } else {
            if !dry_run {
                if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                    return Err(format!("Failed to create directory '{dest_dir}': {e}"));
                }
                match convert(src, &dest_path, actual_hz as i64, channels.unwrap_or(1.0) as i64) {
                    Ok(()) => progress.file_written(&dest_path, "converted"),
                    Err(e) => {
                        log.log("ERROR", &format!("Failed to convert {src}: {e}"));
                        progress.conversion_failed(src);
                        rows.push(Row {
                            source_path: src.clone(),
                            dest_path: Some(dest_path),
                            category: row.category,
                            action: "FAILED_TRANSCODE".into(),
                            status: "ERROR",
                            message: "FFmpeg audio conversion failed".into(),
                        });
                        continue;
                    }
                }
            }
            rows.push(Row {
                source_path: src.clone(),
                dest_path: Some(dest_path),
                category: row.category,
                action: if dry_run { "PLANNED_TRANSCODE" } else { "TRANSCODED" }.into(),
                status: "SUCCESS",
                message: format!(
                    "Transcoded to {} ({} Hz, {} ch)",
                    s.target_format,
                    r_number(&Yaml::Real(format!("{actual_hz}"))),
                    r_number(&s.channels)
                ),
            });
        }
    }
    if !dry_run {
        let _ = std::fs::create_dir_all(&s.standardized_dir);
        let file = log_path(s);
        write_rows(&file, &rows).map_err(|e| format!("Could not write {}: {e}", file.display()))?;
        log.info(&format!("Standardization complete. Log saved to: {}", file.to_string_lossy().replace('\\', "/")));
    } else {
        log.info("Dry-run simulation complete. No files were modified.");
    }
    Ok(rows)
}

// ---- the summary the screen shows ------------------------------------------------------

fn round_to(x: f64, d: i32) -> f64 {
    let p = 10f64.powi(d);
    (x * p).round_ties_even() / p
}

/// `std_summarise`: a plan or a finished run, read back from its CSV.
pub fn summarise(path: &Path, kind: &str) -> Value {
    let Some(t) = Table::read(path).filter(|t| {
        !t.rows.is_empty() && ["action", "dest_path", "status"].iter().all(|c| t.col(c).is_some())
    }) else {
        return json!({ "available": false, "kind": kind });
    };
    let c = |n: &str| t.col(n).unwrap_or(usize::MAX);
    let (src_i, dest_i, cat_i, act_i, st_i, msg_i) =
        (c("source_path"), c("dest_path"), c("category"), c("action"), c("status"), c("message"));
    let get = |r: &Vec<String>, i: usize| r.get(i).cloned().unwrap_or_default();
    let base = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    let cat_n = |x: &str| t.rows.iter().filter(|r| get(r, cat_i) == x).count();
    let act = |r: &Vec<String>| get(r, act_i).strip_prefix("PLANNED_").map(str::to_string).unwrap_or_else(|| get(r, act_i));
    let good: Vec<&Vec<String>> = t.rows.iter().filter(|r| get(r, cat_i) == "standardizable").collect();
    let size: f64 = rsum(good.iter().filter_map(|r| std::fs::metadata(get(r, src_i)).ok()).map(|m| m.len() as f64));
    // per grid: the folder the file goes to, counted, in C-locale order
    let mut per: Vec<(String, usize)> = Vec::new();
    for r in &good {
        let d = get(r, dest_i);
        let parent = d.rsplit_once('/').map(|(p, _)| p).unwrap_or(".");
        let g = base(parent);
        match per.iter_mut().find(|(n, _)| *n == g) {
            Some((_, k)) => *k += 1,
            None => per.push((g, 1)),
        }
    }
    per.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let problems: Vec<Value> = t
        .rows
        .iter()
        .filter(|r| get(r, cat_i) != "standardizable" || get(r, st_i) != "SUCCESS")
        .take(200)
        .map(|r| json!({ "file": base(&get(r, src_i)), "category": get(r, cat_i), "action": get(r, act_i), "message": get(r, msg_i) }))
        .collect();
    let updated = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|m| chrono::DateTime::<chrono::Local>::from(m).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    json!({
        "available": true, "kind": kind, "updated": updated,
        "n_files": t.rows.len(),
        "n_recordings": cat_n("standardizable"),
        "n_memos": cat_n("isolated_memo"),
        "n_non_audio": cat_n("isolated_non_audio"),
        "n_corrupt": cat_n("corrupt"),
        "n_convert": t.rows.iter().filter(|r| matches!(act(r).as_str(), "TRANSCODE" | "TRANSCODED")).count(),
        "n_copy": t.rows.iter().filter(|r| matches!(act(r).as_str(), "COPY_COMPLIANT" | "COPIED_COMPLIANT")).count(),
        "n_failed": t.rows.iter().filter(|r| get(r, st_i) == "ERROR").count(),
        "raw_gb": round_to(size / 1024f64.powi(3), 2),
        "per_grid": per.iter().take(500).map(|(g, n)| json!({ "grid": g, "files": n })).collect::<Vec<_>>(),
        "examples": good.iter().take(8).map(|r| json!({ "from": base(&get(r, src_i)), "to": get(r, dest_i) })).collect::<Vec<_>>(),
        "problems": problems,
    })
}

/// The `std_state` event (`std_emit_state`).
pub fn state_event(cfg: &Config, s: &Settings) -> Value {
    let raw_found = Path::new(&s.raw_dir).is_dir();
    let n_raw = if raw_found {
        let mut n = 0usize;
        fn count(dir: &Path, n: &mut usize) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                match e.file_type() {
                    Ok(t) if t.is_dir() => count(&e.path(), n),
                    Ok(_) => *n += 1,
                    _ => {}
                }
            }
        }
        count(Path::new(&s.raw_dir), &mut n);
        json!(n)
    } else {
        Value::Null
    };
    let probs = problems(s);
    // jsonlite: a one-element character vector is written as a plain string
    let probs = match probs.len() {
        1 => json!(probs[0]),
        _ => json!(probs),
    };
    json!({
        "settings": s.json(), "raw_found": raw_found, "n_raw_files": n_raw, "problems": probs,
        "plan": summarise(&plan_path(cfg), "plan"),
        "last_run": summarise(&log_path(s), "run"),
    })
}

/// Recordings the raw folder holds by extension (`std_count_audio`).
pub fn count_audio(raw_dir: &str) -> (usize, usize) {
    let files = {
        let mut v = Vec::new();
        fn walk(d: &Path, out: &mut Vec<String>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&e.path(), out),
                    Ok(_) => out.push(name),
                    _ => {}
                }
            }
        }
        walk(Path::new(raw_dir), &mut v);
        v
    };
    let audio = files.iter().filter(|f| AUDIO_EXTS.contains(&path_ext(f).to_lowercase().as_str())).count();
    (files.len(), audio)
}

/// Where progress events go: (event type, fields).
pub type Emit<'a> = Box<dyn FnMut(&str, Value) + 'a>;

/// The app's progress counters (`std_counter`, `.pd_std_tick`, `std_flush`):
/// "prep" events while headers are read, "files" events while files are written.
pub struct Counters<'a> {
    pub phase: &'static str,
    pub total: usize,
    pub done: i64,
    pub failed: i64,
    rows: Vec<Value>,
    last_emit: Instant,
    emit: Emit<'a>,
}

impl<'a> Counters<'a> {
    pub fn new(emit: Emit<'a>) -> Self {
        Counters { phase: "read", total: 0, done: 0, failed: 0, rows: Vec::new(), last_emit: Instant::now(), emit }
    }

    pub fn event(&mut self, kind: &str, fields: Value) {
        (self.emit)(kind, fields);
    }

    /// `std_reset`
    pub fn reset(&mut self, phase: &'static str, total: usize) {
        self.phase = phase;
        self.total = total;
        self.done = 0;
        self.failed = 0;
        self.rows.clear();
        self.last_emit = Instant::now();
        if phase == "write" {
            (self.emit)("total", json!({ "n": total }));
        }
    }

    /// `std_flush`: at most once a second unless forced.
    pub fn flush(&mut self, force: bool) {
        if !force && self.last_emit.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_emit = Instant::now();
        if self.phase == "read" {
            (self.emit)("prep", json!({ "done": self.done, "total": self.total }));
        } else {
            let rows = if self.rows.is_empty() { Value::Null } else { Value::Array(std::mem::take(&mut self.rows)) };
            (self.emit)(
                "files",
                json!({ "batch": 0, "batches": 0, "done": self.done, "failed": self.failed, "detections": 0, "rows": rows }),
            );
        }
    }

    fn tick(&mut self, dest: Option<(&str, &str)>) {
        self.done += 1;
        if let Some((d, what)) = dest {
            if self.phase == "write" {
                self.rows.push(json!({ "rel": d.rsplit('/').next().unwrap_or(d), "ok": true, "message": what }));
                if self.rows.len() > 50 {
                    let cut = self.rows.len() - 50;
                    self.rows.drain(..cut);
                }
            }
        }
        self.flush(false);
    }
}

/// Progress for the plan (reading only) and for the run (reading, then writing).
pub struct RunProgress<'a, 'b> {
    pub c: &'b mut Counters<'a>,
    /// Files the run will write (`to_write`); None for the plan.
    pub write_total: Option<usize>,
    pub out_dir: String,
}

impl RunProgress<'_, '_> {
    /// `std_start_writing`
    fn start_writing(&mut self, total: usize) {
        self.c.flush(true);
        let dir = self.out_dir.clone();
        self.c.event("stage", json!({ "stage": "standardize", "label": "Writing standardized files", "run_dir": dir, "already_done": 0 }));
        self.c.reset("write", total);
    }

    /// After the run: nothing was written at all.
    pub fn finish(&mut self) {
        if let (Some(t), "read") = (self.write_total, self.c.phase) {
            self.start_writing(t);
        }
        self.c.flush(true);
    }
}

impl Progress for RunProgress<'_, '_> {
    fn header_read(&mut self) {
        if self.c.phase == "read" {
            self.c.tick(None);
        }
    }
    fn file_written(&mut self, dest: &str, what: &str) {
        if let (Some(t), "read") = (self.write_total, self.c.phase) {
            self.start_writing(t);
        }
        self.c.tick(Some((dest, what)));
    }
    fn conversion_failed(&mut self, src: &str) {
        if self.c.phase == "write" {
            self.c.done = (self.c.done - 1).max(0);
            self.c.failed += 1;
            let rel = src.rsplit(['/', '\\']).next().unwrap_or(src);
            self.c.rows.push(json!({ "rel": rel, "ok": false, "message": "conversion failed" }));
        }
    }
}

/// Is this a recording name pamflow lists (used by the raw-folder count)?
pub fn is_recording_name(name: &str) -> bool {
    is_audio_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_and_times_from_names() {
        let pat = r"^(?<grid>G\d{4})_(?<date>\d{8})_(?<time>\d{6})\.wav$";
        assert_eq!(extract_grid("G0059_20260405_173058.wav", Some(pat)), "G0059");
        assert_eq!(extract_grid("SW001_x_y.wav", None), "SW001");
        assert_eq!(extract_grid("20260407-080000.wav", None), "unassigned");
        assert_eq!(parse_datetime("G0059_20260405_173058.wav", ""), "20260405_173058");
        assert_eq!(parse_datetime("a_20260407-080000.wav", ""), "20260407_080000");
        assert_eq!(parse_datetime("20260407T080000.wav", ""), "20260407_080000");
        assert_eq!(parse_datetime("recorder_260408090000.wav", ""), "20260408_090000");
    }

    #[test]
    fn extensions_like_fs() {
        assert_eq!(path_ext("a/b.FLAC"), "FLAC");
        assert_eq!(path_ext("noext"), "");
        assert_eq!(path_ext("x.tar.gz"), "gz");
        assert_eq!(path_ext_remove("a.b.wav"), "a.b");
        assert_eq!(path_ext_remove("x"), "x");
    }

    #[test]
    fn windows_listing_order() {
        let mut v: Vec<&str> = vec!["rec_one.flac", "recorder_x.wav", "REC_Y.wav", "rec-z.wav", "rec1.wav", "_u.wav", "Z.wav", "c.wav", "c", "~t.wav"];
        v.sort_by_cached_key(|n| ntfs_key(n));
        assert_eq!(v, ["c", "c.wav", "rec-z.wav", "rec1.wav", "recorder_x.wav", "rec_one.flac", "REC_Y.wav", "Z.wav", "_u.wav", "~t.wav"]);
    }

    #[test]
    fn numbers_in_messages() {
        for (x, want) in [(48000.0, "48000"), (100000.0, "1e+05"), (44100.0, "44100"), (123456.0, "123456"), (1e15, "1e+15")] {
            assert_eq!(r_number(&Yaml::Real(x.to_string())), want, "{x}");
        }
        assert_eq!(r_number(&Yaml::Integer(1)), "1");
    }
}
