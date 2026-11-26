//! Saving edited settings (port of `commit_config`, `check_allowed_changes`,
//! `job_config_save` and `job_config_save_as` in pamflow Desktop
//! `bridge/run_job.R`).
//!
//! Only the edited lines change (see [`crate::config_edit`]). The new file is
//! first written next to the old one as a check file and read the way pamflow
//! reads it; every changed setting must read back as sent. Only then is the old
//! file copied to `backups/<name>.<YYYYmmdd-HHMMSS>.bak` and replaced.

use crate::config::{load_file, num, Config};
use crate::config_edit::{self, derive_root, norm_path, Change, ConfigText};
use crate::project::{config_root, Found};
use serde_json::Value;
use std::path::{Path, PathBuf};
use yaml_rust2::Yaml;

/// Kind of value a setting holds, as the Project screen edits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Choice,
    Number,
    Integer,
    IntegerOrNull,
    Boolean,
    Text,
    List,
    Indices,
}

/// The settings the app may edit (`EDITABLE` in run_job.R), in screen order.
pub const EDITABLE: &[(&str, &str, Kind)] = {
    use Kind::*;
    &[
        ("paths", "audio_raw", Folder),
        ("paths", "audio_isolated", Folder),
        ("standardization", "target_format", Choice),
        ("standardization", "target_hz", Number),
        ("standardization", "channels", Integer),
        ("standardization", "min_duration_s", Number),
        ("standardization", "organize_folders", Choice),
        ("standardization", "rename_strategy", Choice),
        ("standardization", "prefix", Text),
        ("standardization", "isolate_non_audio", Boolean),
        ("standardization", "isolate_short", Boolean),
        ("paths", "audio_standardized", Folder),
        ("paths", "deployment_sheet", File),
        ("filenames", "timezone", Text),
        ("paths", "filtered", Folder),
        ("filter", "enabled", Boolean),
        ("filter", "highpass_hz", Number),
        ("filter", "order", Integer),
        ("filter", "workers", Integer),
        ("birdnet", "model_dir", Folder),
        ("birdnet", "classifier_name", Text),
        ("birdnet", "labels_file", File),
        ("birdnet", "min_confidence", Number),
        ("birdnet", "workers", IntegerOrNull),
        ("birdnet", "overlap_s", Number),
        ("birdnet", "input", Choice),
        ("birdnet", "run_id", Text),
        ("processing", "min_confidence", Number),
        ("processing", "drop_outside_deployment", Boolean),
        ("processing", "merge_gap_s", Number),
        ("occupancy", "occasion_length_days", Integer),
        ("occupancy", "min_effort_hours", Number),
        ("occupancy", "min_detections", Integer),
        ("occupancy", "run_id", Text),
        ("soundscape", "enabled", Boolean),
        ("soundscape", "window_mins", IntegerOrNull),
        ("soundscape", "low_stop_hz", Number),
        ("soundscape", "workers", IntegerOrNull),
        ("soundscape", "batch_size", IntegerOrNull),
        ("soundscape", "day_hours", List),
        ("soundscape", "run_id", Text),
        ("soundscape", "acoustic_indices", Indices),
        ("validation", "reviewer", Text),
        ("validation", "show_confidence", Boolean),
        ("report", "title", Text),
        ("report", "author", Text),
        // the rest of pamflow's settings (shown under "More settings")
        ("report", "formats", List),
        ("deployments", "has_header", Boolean),
        ("deployments", "grid_prefix", Text),
        ("deployments", "date_format", Text),
        ("deployments", "time_format", Text),
        ("deployments", "default_recovery_time", Text),
        ("deployments", "columns", List),
        ("birdnet", "top_k", IntegerOrNull),
        ("birdnet", "sigmoid_sensitivity", Number),
        ("birdnet", "device", Choice),
        ("birdnet", "use_bandpass", Boolean),
        ("birdnet", "bandpass_fmin", Number),
        ("birdnet", "bandpass_fmax", Number),
        ("birdnet", "py_batch_size", Integer),
        ("birdnet", "n_producers", Integer),
        ("processing", "species_thresholds_file", File),
        ("processing", "species_whitelist", List),
        ("processing", "species_blacklist", List),
        ("processing", "date_range", List),
        ("processing", "species_season_file", File),
        ("processing", "max_overrun_s", Number),
        ("validation", "bands", List),
        ("validation", "n_per_cell", Integer),
        ("validation", "rare_max_events", Integer),
        ("validation", "rare_n", Integer),
        ("validation", "clip_pad_s", Number),
        ("validation", "spec_max_hz", Number),
        ("validation", "spec_px", List),
        ("validation", "seed", Integer),
        ("recall", "segment_s", Number),
        ("recall", "n_segments", Integer),
        ("recall", "hours", List),
        ("recall", "audio", Choice),
        ("recall", "spec_px", List),
        ("recall", "spec_max_hz", Number),
        ("recall", "seed", Integer),
        ("recall", "reviewer", Text),
    ]
};

/// Refuse a change to a setting the app may not edit.
pub fn check_allowed(changes: &[Change]) -> Result<(), String> {
    for ch in changes {
        let ok = (ch.section == "paths" && ch.key == "project_root")
            || EDITABLE.iter().any(|(s, k, _)| *s == ch.section && *k == ch.key);
        if !ok {
            return Err(format!("This setting cannot be changed here: {} {}", ch.section, ch.key));
        }
    }
    Ok(())
}

// ---- reading a value back ---------------------------------------------------

#[derive(Debug)]
enum Atom {
    Bool(bool),
    Num(f64),
    Str(String),
}

fn atoms_yaml(y: &Yaml, out: &mut Vec<Atom>) {
    match y {
        Yaml::Array(a) => a.iter().for_each(|x| atoms_yaml(x, out)),
        Yaml::Hash(h) => h.values().for_each(|x| atoms_yaml(x, out)),
        Yaml::Boolean(b) => out.push(Atom::Bool(*b)),
        Yaml::String(s) => out.push(Atom::Str(s.clone())),
        Yaml::Integer(_) | Yaml::Real(_) => out.push(Atom::Num(num(y).unwrap_or(f64::NAN))),
        _ => {}
    }
}

fn atoms_json(v: &Value, out: &mut Vec<Atom>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| atoms_json(x, out)),
        Value::Object(o) => o.values().for_each(|x| atoms_json(x, out)),
        Value::Bool(b) => out.push(Atom::Bool(*b)),
        Value::String(s) => out.push(Atom::Str(s.clone())),
        Value::Number(n) => out.push(Atom::Num(n.as_f64().unwrap_or(f64::NAN))),
        Value::Null => {}
    }
}

/// Does the value read from the file equal the value sent? Like R's
/// `identical(unlist(got), want) || isTRUE(all.equal(unlist(got), want))`:
/// lists are flattened, null and an empty list are the same, numbers are equal
/// within all.equal's tolerance.
fn reads_back(got: &Yaml, want: &Value) -> bool {
    let (mut g, mut w) = (Vec::new(), Vec::new());
    atoms_yaml(got, &mut g);
    atoms_json(want, &mut w);
    if g.len() != w.len() {
        return false;
    }
    let num_pairs: Vec<(f64, f64)> = g
        .iter()
        .zip(&w)
        .filter_map(|p| match p {
            (Atom::Num(a), Atom::Num(b)) => Some((*a, *b)),
            _ => None,
        })
        .collect();
    let others_equal = g.iter().zip(&w).all(|p| match p {
        (Atom::Num(_), Atom::Num(_)) => true,
        (Atom::Bool(a), Atom::Bool(b)) => a == b,
        (Atom::Str(a), Atom::Str(b)) => a == b,
        _ => false,
    });
    if !others_equal {
        return false;
    }
    // all.equal: mean relative difference below 1.5e-8
    let sum_abs: f64 = num_pairs.iter().map(|(_, b)| b.abs()).sum();
    let sum_diff: f64 = num_pairs.iter().map(|(a, b)| (a - b).abs()).sum();
    sum_diff == 0.0 || (sum_abs > 0.0 && sum_diff / sum_abs < 1.5e-8) || (sum_abs == 0.0 && sum_diff < 1.5e-8)
}

fn index_names(y: &Yaml) -> Vec<String> {
    let mut v: Vec<String> = match y {
        Yaml::Hash(h) => h.keys().filter_map(|k| k.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    v.sort();
    v
}

fn json_names(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    };
    out.sort();
    out
}

// ---- writing -------------------------------------------------------------------

/// Where a save went.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    /// The settings file written.
    pub config: PathBuf,
    /// The backup of the previous file, or "" when there was none.
    pub backup: String,
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(".{}.app-write", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Check a new version of a settings file, back up the old one, write it
/// (`commit_config`). `stamp` is the local time as `YYYYmmdd-HHMMSS`.
pub fn commit(path: &Path, new_lines: &[String], changes: &[Change], crlf: bool, stamp: &str) -> Result<Saved, String> {
    let text = ConfigText::render(new_lines, crlf);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let check = path.with_file_name(format!(".{name}.app-check.yml"));
    std::fs::write(&check, &text).map_err(|e| format!("Not saved: could not write {}: {e}", check.display()))?;
    let outcome = (|| {
        if let Err(e) = Config::read(&check) {
            return Err(format!("Not saved: pamflow would not accept these settings. {e}"));
        }
        let saved = load_file(&check).map_err(|e| format!("Not saved: {e}"))?;
        for ch in changes {
            let got = &saved[ch.section.as_str()][ch.key.as_str()];
            let ok = if ch.key == "acoustic_indices" {
                index_names(got) == json_names(&ch.value)
            } else {
                reads_back(got, &ch.value)
            };
            if !ok {
                return Err(format!("Not saved: {}: {} would not be read back correctly.", ch.section, ch.key));
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&check);
    outcome?;

    let mut backup = String::new();
    if path.exists() {
        let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
        std::fs::create_dir_all(&dir).map_err(|_| "Not saved: could not make a backup copy.".to_string())?;
        // R refuses to save twice in one second (the backup name exists);
        // here the second backup gets a number instead
        let mut b = dir.join(format!("{name}.{stamp}.bak"));
        let mut n = 2;
        while b.exists() {
            b = dir.join(format!("{name}.{stamp}_{n}.bak"));
            n += 1;
        }
        std::fs::copy(path, &b).map_err(|_| "Not saved: could not make a backup copy.".to_string())?;
        backup = norm_path(&b);
    }
    write_atomic(path, &text).map_err(|e| format!("Not saved: could not write {}: {e}", path.display()))?;
    Ok(Saved { config: path.to_path_buf(), backup })
}

/// The local time as used in backup names.
pub fn stamp_now() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Save `changes` into the settings file itself (`job_config_save`).
/// `Ok(None)` when there was nothing to save.
pub fn save(found: &Found, changes: &[Change], stamp: &str) -> Result<Option<Saved>, String> {
    if changes.is_empty() {
        return Ok(None);
    }
    check_allowed(changes)?;
    let cur = ConfigText::read(&found.config).map_err(|e| format!("Could not read {}: {e}", found.config.display()))?;
    let lines = config_edit::apply(cur.lines, changes).map_err(|e| e.to_string())?;
    commit(&found.config, &lines, changes, cur.crlf, stamp).map(Some)
}

/// Save the settings with `changes` to another file and carry on with that
/// file (`job_config_save_as`). If the new file does not sit in the project's
/// config/ folder, `project_root` is written into it so its folders still point
/// at the same project. Returns the save and the project as found from now on.
pub fn save_as(found: &Found, target: &str, changes: &[Change], stamp: &str) -> Result<(Saved, Found), String> {
    if target.is_empty() {
        return Err("No file was chosen.".into());
    }
    let mut target = norm_path(Path::new(target));
    let lower = target.to_lowercase();
    if !(lower.ends_with(".yml") || lower.ends_with(".yaml")) {
        target.push_str(".yml");
    }
    if target.to_lowercase() == norm_path(&found.config).to_lowercase() {
        let saved = save(found, changes, stamp)?.unwrap_or(Saved { config: found.config.clone(), backup: String::new() });
        return Ok((saved, found.clone()));
    }
    check_allowed(changes)?;
    let target = PathBuf::from(target);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|_| format!("Could not create the folder {}.", dir.display()))?;
    }
    let cur = ConfigText::read(&found.config).map_err(|e| format!("Could not read {}: {e}", found.config.display()))?;
    let mut lines = config_edit::apply(cur.lines, changes).map_err(|e| e.to_string())?;
    if derive_root(&target).to_lowercase() != found.root.to_lowercase() {
        lines = config_edit::set(lines, "paths", "project_root", &Value::String(found.root.clone()));
    }
    let saved = commit(&target, &lines, changes, cur.crlf, stamp)?;
    let root = config_root(&target);
    Ok((saved, Found { config: target, root }))
}
