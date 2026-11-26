//! The project report the Project screen shows (port of `job_inspect`,
//! `editable_settings` and `processing_settings` in pamflow Desktop
//! `bridge/run_job.R`). Reads only; changes nothing on disk.

use crate::audio_files::{list_audio, parse_filenames};
use crate::config::{load_file, num, Config, TEMPLATE};
use crate::config_save::{Kind, EDITABLE};
use crate::project::{is_absolute, Found};
use serde_json::{json, Map, Value};
use std::path::Path;
use yaml_rust2::Yaml;

/// Indices the app does not offer (`PD_UNAVAILABLE_INDICES`): Acoustic
/// Entropy comes out empty in pamflow 0.6.0.
pub const UNAVAILABLE_INDICES: &[&str] = &["H"];

/// A YAML value as R's yaml + jsonlite (`auto_unbox = TRUE`) write it: a
/// one-item list of plain values becomes that value, unless `keep_list`.
pub fn yaml_json(y: &Yaml, keep_list: bool) -> Value {
    match y {
        Yaml::Null | Yaml::BadValue => Value::Null,
        Yaml::Boolean(b) => json!(b),
        Yaml::Integer(i) => json!(i),
        Yaml::Real(_) => num(y).map(|f| json!(f)).unwrap_or(Value::Null),
        Yaml::String(s) => json!(s),
        Yaml::Array(a) => {
            let plain = a.iter().all(|x| !matches!(x, Yaml::Array(_) | Yaml::Hash(_)));
            if a.len() == 1 && plain && !keep_list {
                yaml_json(&a[0], false)
            } else {
                Value::Array(a.iter().map(|x| yaml_json(x, false)).collect())
            }
        }
        Yaml::Hash(h) => Value::Object(
            h.iter().map(|(k, v)| (k.as_str().map(str::to_string).unwrap_or_default(), yaml_json(v, false))).collect(),
        ),
        _ => Value::Null,
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Folder => "folder",
        Kind::File => "file",
        Kind::Choice => "choice",
        Kind::Number => "number",
        Kind::Integer => "integer",
        Kind::IntegerOrNull => "integer_or_null",
        Kind::Boolean => "boolean",
        Kind::Text => "text",
        Kind::List => "list",
        Kind::Indices => "indices",
    }
}

/// Each editable setting: its value in the settings file (or pamflow's
/// default) and whether the file sets it itself (`editable_settings`).
pub fn editable_settings(config_path: &Path) -> Value {
    let raw = load_file(config_path).unwrap_or(Yaml::Null);
    let defaults = crate::config::parse(TEMPLATE).unwrap_or(Yaml::Null);
    Value::Array(
        EDITABLE
            .iter()
            .map(|&(section, key, kind)| {
                let in_file = raw[section].as_hash().is_some_and(|h| h.contains_key(&Yaml::String(key.into())));
                let v = if in_file { &raw[section][key] } else { &defaults[section][key] };
                let value = match kind {
                    Kind::Indices => {
                        let names: Vec<Value> = v
                            .as_hash()
                            .map(|h| {
                                h.keys()
                                    .filter_map(|k| k.as_str())
                                    .filter(|k| !UNAVAILABLE_INDICES.contains(k))
                                    .map(|k| json!(k))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Value::Array(names)
                    }
                    Kind::List => yaml_json(v, true),
                    _ => yaml_json(v, false),
                };
                json!({ "section": section, "key": key, "kind": kind_name(kind), "value": value, "in_file": in_file })
            })
            .collect(),
    )
}

fn count_list(y: &Yaml) -> usize {
    match y {
        Yaml::Array(a) => a.iter().map(count_list).sum(),
        Yaml::Hash(h) => h.values().map(count_list).sum(),
        Yaml::Null | Yaml::BadValue => 0,
        _ => 1,
    }
}

/// `processing_settings`.
pub fn processing_settings(cfg: &Config) -> Value {
    let pc = &cfg.data["processing"];
    let or = |y: &Yaml, d: Value| if matches!(y, Yaml::Null | Yaml::BadValue) { d } else { yaml_json(y, false) };
    json!({
        "min_confidence": yaml_json(&pc["min_confidence"], false),
        "drop_outside_deployment": pc["drop_outside_deployment"].as_bool().unwrap_or(
            matches!(pc["drop_outside_deployment"], Yaml::Null | Yaml::BadValue)),
        "merge_gap_s": or(&pc["merge_gap_s"], json!(0)),
        "max_overrun_s": or(&pc["max_overrun_s"], json!(3)),
        "thresholds_file": yaml_json(&pc["species_thresholds_file"], false),
        "season_file": yaml_json(&pc["species_season_file"], false),
        "n_whitelist": count_list(&pc["species_whitelist"]),
        "n_blacklist": count_list(&pc["species_blacklist"]),
        // unlist(): an empty list is NULL
        "date_range": if count_list(&pc["date_range"]) == 0 { Value::Null } else { yaml_json(&pc["date_range"], false) },
    })
}

/// `file.path(root, p)` unless `p` is absolute.
fn in_project(root: &str, p: &str) -> String {
    if is_absolute(p) {
        p.to_string()
    } else {
        format!("{root}/{p}")
    }
}

fn exists(p: &str) -> bool {
    !p.is_empty() && Path::new(p).exists()
}

/// Total of the `file_size_b` column of audio_master.csv, if it has one.
fn scanned_bytes(master: &str) -> Option<f64> {
    let mut r = csv::ReaderBuilder::new().flexible(true).from_path(master).ok()?;
    let col = r.headers().ok()?.iter().position(|h| h == "file_size_b")?;
    Some(r.records().flatten().filter_map(|rec| rec.get(col)?.trim().parse::<f64>().ok()).sum())
}

/// Rows of the deployment sheet (non-blank lines, without the header line).
fn count_deployments(sheet: &str, has_header: bool) -> Option<usize> {
    let text = std::fs::read_to_string(sheet).ok()?;
    let rows = text.lines().filter(|l| !l.trim().is_empty()).count();
    Some(if has_header { rows.saturating_sub(1) } else { rows })
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// The fields of the "project" event (`job_inspect`).
pub fn project_event(cfg: &Config, found: &Found) -> Value {
    let paths = &cfg.data["paths"];
    let audio_dir = paths["audio_standardized"].as_str().unwrap_or("").to_string();
    let audio_found = Path::new(&audio_dir).is_dir();
    let files = if audio_found { list_audio(&audio_dir) } else { Vec::new() };
    let bc = &cfg.data["birdnet"];
    let root = &cfg.root;

    let model_dir = bc["model_dir"].as_str().map(|d| in_project(root, d));
    let name = bc["classifier_name"].as_str().unwrap_or("");
    let name = if name.to_lowercase().ends_with(".tflite") { &name[..name.len() - 7] } else { name };
    let model_file = model_dir.as_ref().map(|d| format!("{d}/{name}.tflite"));
    let mut labels_file = model_dir.as_ref().map(|d| format!("{d}/{name}_Labels.txt"));
    if let Some(l) = bc["labels_file"].as_str().filter(|l| !l.is_empty()) {
        labels_file = Some(in_project(root, l));
    }

    let metadata = paths["metadata"].as_str().unwrap_or("");
    let master = format!("{metadata}/audio_master.csv");
    let audio_gb = if files.is_empty() {
        Value::Null
    } else {
        let bytes = scanned_bytes(&master).unwrap_or_else(|| {
            files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len() as f64).sum()
        });
        json!(round2(bytes / 1024f64.powi(3)))
    };

    let (mut example, mut unreadable) = (Value::Null, Value::Null);
    if !files.is_empty() {
        let sample: Vec<String> = files.iter().take(200).cloned().collect();
        let pattern = cfg.data["filenames"]["pattern"].as_str().unwrap_or("");
        let tz = cfg.data["filenames"]["timezone"].as_str().unwrap_or("UTC");
        if let Ok(parsed) = parse_filenames(&sample, pattern, tz) {
            unreadable = json!(parsed.iter().filter(|p| !p.valid()).count());
            let first = &parsed[0];
            example = json!({
                "file": first.file_name,
                "grid": first.grid,
                "start": if first.valid() { first.start.map(|s| s.format("%Y-%m-%d %H:%M:%S").to_string()) } else { None },
                "valid": first.valid(),
                "sample_size": parsed.len(),
            });
        }
    }

    let sheet = paths["deployment_sheet"].as_str().unwrap_or("").to_string();
    let has_header = cfg.data["deployments"]["has_header"].as_bool().unwrap_or(true);
    let n_deployments = if exists(&sheet) { json!(count_deployments(&sheet, has_header)) } else { Value::Null };

    let labels_found = labels_file.as_deref().is_some_and(exists);
    let n_labels = if labels_found {
        std::fs::read_to_string(labels_file.as_deref().unwrap()).map(|t| json!(t.lines().count())).unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    let mut ev = Map::new();
    let mut put = |k: &str, v: Value| {
        ev.insert(k.into(), v);
    };
    put("config", json!(crate::config_edit::norm_path(&found.config)));
    put("root", json!(found.root));
    put("audio_folder", json!(audio_dir));
    put("audio_folder_found", json!(audio_found));
    put("n_audio", if audio_found { json!(files.len()) } else { Value::Null });
    put("audio_gb", audio_gb);
    put("filename_example", example);
    put("n_unreadable_names", unreadable);
    put("scanned", json!(exists(&master)));
    put("deployment_sheet", json!(sheet));
    put("deployment_sheet_found", json!(exists(&sheet)));
    put("n_deployments", n_deployments);
    put("model_file", json!(model_file));
    put("model_found", json!(model_file.as_deref().is_some_and(exists)));
    put("labels_found", json!(labels_found));
    put("n_labels", n_labels);
    put("run_id", yaml_json(&bc["run_id"], false));
    put("min_confidence", yaml_json(&bc["min_confidence"], false));
    put("workers", yaml_json(&bc["workers"], false));
    put("cores_physical", json!(num_cpus::get_physical()));
    put("cores_logical", json!(num_cpus::get()));
    put("detections_folder", yaml_json(&paths["detections"], false));
    put("processing", processing_settings(cfg));
    put("settings", editable_settings(&found.config));
    Value::Object(ev)
}

/// The "no_config" event: no settings file was found for `project`.
pub fn no_config_event(project: Option<&str>, config: Option<&str>) -> Value {
    let folder = project.filter(|p| !p.is_empty()).map(|p| crate::config_edit::norm_path(Path::new(p))).unwrap_or_default();
    let missing = config.filter(|c| !c.is_empty()).map(|c| crate::config_edit::norm_path(Path::new(c)));
    json!({
        "folder": folder,
        "exists": !folder.is_empty() && Path::new(&folder).is_dir(),
        "missing_file": missing,
        "suggested": format!("{folder}/config/config.yml"),
    })
}
