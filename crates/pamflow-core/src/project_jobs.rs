//! New projects, the example project and the diagnostics file (port of
//! `pd_create_project`, the `example_project` block, `pd_localize_example` and
//! `pd_diagnostics` in pamflow Desktop `bridge/run_job.R` / `protocol.R`, and
//! of `pam_example_project` in pamflow).

use crate::config::{Config, TEMPLATE};
use crate::config_edit::{new_config_lines, norm_path, ConfigText};
use crate::project::Found;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The pamflow version the embedded template comes from.
pub const TEMPLATE_VERSION: &str = "0.6.0";

fn template_lines() -> Vec<String> {
    ConfigText::parse(TEMPLATE).lines
}

fn ensure_yml(target: String) -> String {
    let lower = target.to_lowercase();
    if lower.ends_with(".yml") || lower.ends_with(".yaml") {
        target
    } else {
        format!("{target}.yml")
    }
}

/// Make `<folder>/config/config.yml` (or `target`) from pamflow's default
/// settings (`pd_create_project`). Returns the new settings file.
pub fn create_project(folder: &str, target: Option<&str>, date: &str) -> Result<Found, String> {
    if folder.is_empty() {
        return Err("No project folder was given.".into());
    }
    let folder = norm_path(Path::new(folder));
    let fpath = PathBuf::from(&folder);
    if !fpath.is_dir() && std::fs::create_dir_all(&fpath).is_err() {
        return Err(format!("Could not create the folder {folder}."));
    }
    let target = match target.filter(|t| !t.is_empty()) {
        Some(t) => ensure_yml(norm_path(Path::new(t))),
        None => format!("{folder}/config/config.yml"),
    };
    let tpath = PathBuf::from(&target);
    if tpath.exists() {
        return Err(format!("A settings file already exists there: {target}"));
    }
    let lines = new_config_lines(&template_lines(), &fpath, &tpath, TEMPLATE_VERSION, date);
    if let Some(dir) = tpath.parent() {
        if !dir.is_dir() && std::fs::create_dir_all(dir).is_err() {
            return Err(format!("Could not create the folder {}.", dir.display()));
        }
    }
    std::fs::write(&tpath, ConfigText::render(&lines, false)).map_err(|e| format!("Could not write {target}: {e}"))?;
    if let Err(e) = Config::read(&tpath) {
        let _ = std::fs::remove_file(&tpath);
        return Err(format!("pamflow would not accept the default settings: {e}"));
    }
    Ok(Found { config: tpath, root: folder })
}

// ---- the example project ------------------------------------------------------

/// A 16-bit mono WAV file the way tuneR::writeWave writes it
/// (WAVE_FORMAT_EXTENSIBLE header with a fact chunk).
fn write_wav_like_tuner(path: &Path, samples: &[i16], rate: u32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut b: Vec<u8> = Vec::with_capacity(80 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(72 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
    b.extend_from_slice(&1u16.to_le_bytes()); // channels
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits
    b.extend_from_slice(&22u16.to_le_bytes()); // extension size
    b.extend_from_slice(&16u16.to_le_bytes()); // valid bits
    b.extend_from_slice(&1u32.to_le_bytes()); // channel mask: front left
    // KSDATAFORMAT_SUBTYPE_PCM
    b.extend_from_slice(&[0x01, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
    b.extend_from_slice(b"fact");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b)
}

/// One sample-detection row: file index (1-based), species, start, end, confidence.
type Det = (usize, &'static str, f64, f64, f64);

/// pamflow's example project in `dir` (`pam_example_project`), localized to
/// the Western Ghats like the app does (`pd_localize_example`). Returns the
/// project root and its settings file.
pub fn example_project(dir: &Path) -> Result<Found, String> {
    let busy = dir.is_dir() && std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(false);
    if busy {
        return Err(format!("Folder is not empty: {}", norm_path(dir)));
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dir).map_err(io)?;
    let root = crate::config::canonical_slash(dir);
    let r = PathBuf::from(&root);
    std::fs::create_dir_all(r.join("metadata")).map_err(io)?;
    std::fs::create_dir_all(r.join("config")).map_err(io)?;

    // six short tones: three files per grid, one right after the other
    let recordings = [
        ("G0059", "20260405_183000", 440.0),
        ("G0059", "20260405_183002", 550.0),
        ("G0059", "20260405_183004", 660.0),
        ("G0110", "20260406_060000", 330.0),
        ("G0110", "20260406_060002", 440.0),
        ("G0110", "20260406_060004", 880.0),
    ];
    let rate = 8000u32;
    let n = (rate * 2) as usize;
    let mut rel = Vec::new();
    for (grid, stamp, freq) in recordings {
        let gdir = r.join("data").join("standardized").join(grid);
        std::fs::create_dir_all(&gdir).map_err(io)?;
        let tone: Vec<i16> = (0..n)
            .map(|i| {
                let x = 10000.0 * (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin();
                x.round_ties_even() as i16 // R's round() rounds half to even
            })
            .collect();
        let name = format!("{grid}_{stamp}.wav");
        write_wav_like_tuner(&gdir.join(&name), &tone, rate).map_err(io)?;
        rel.push(format!("{grid}/{name}"));
    }

    // deployment sheet (already localized: Western Ghats sites)
    let deployments = [
        "deployment_id,recorder,grid,habitat,deployer,latitude,longitude,deploy_date,deploy_time,retrieve_date,retrieve_time",
        "1,SW010,59,Shola forest,Asha,11.41,76.70,05-04-2026,18:00,08-04-2026,16:00",
        "2,SW012,110,Tea plantation,Ravi,11.38,76.73,01-04-2026,06:00,10-04-2026,18:00",
    ];
    write_lines(&r.join("metadata").join("deployments.csv"), &deployments).map_err(io)?;

    // BirdNET-style detections, as Stage 3's batch files look
    let species = |s: &str| match s {
        "parakeet" => ("Psittacula columboides", "Malabar Parakeet"),
        "pigeon" => ("Ocyceros griseus", "Malabar Grey Hornbill"),
        _ => ("Gracula religiosa", "Common Hill Myna"),
    };
    let mut dets: Vec<Det> = Vec::new();
    for (f, (c1, c2)) in [(0.92, 0.88), (0.81, 0.77), (0.66, 0.52), (0.74, 0.69), (0.58, 0.55), (0.45, 0.35)]
        .into_iter()
        .enumerate()
    {
        dets.push((f + 1, "parakeet", 0.0, 3.0, c1));
        dets.push((f + 1, "parakeet", 1.5, 4.5, c2));
    }
    for (f, c) in [(1, 0.71), (2, 0.64), (4, 0.83), (5, 0.58)] {
        dets.push((f, "pigeon", 0.0, 3.0, c));
    }
    dets.push((4, "myna", 0.0, 3.0, 0.95));
    dets.push((2, "myna", 0.0, 3.0, 0.31)); // below the default threshold
    dets.push((3, "parakeet", 4.0, 7.0, 0.60)); // a window beyond the end of the recording
    dets.push((1, "parakeet", 0.0, 3.0, 0.92)); // a duplicate of an earlier row
    let batch_dir = r.join("data").join("detections").join("run01").join("batches");
    std::fs::create_dir_all(&batch_dir).map_err(io)?;
    let fmt = |x: f64| crate::config_edit::format_number(&serde_json::Number::from_f64(x).unwrap());
    let mut rows = vec!["\"rel\",\"start_s\",\"end_s\",\"scientific_name\",\"common_name\",\"confidence\"".to_string()];
    for (f, sp, s, e, c) in dets {
        let (sci, common) = species(sp);
        rows.push(format!("\"{}\",{},{},\"{sci}\",\"{common}\",{}", rel[f - 1], fmt(s), fmt(e), fmt(c)));
    }
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    write_lines(&batch_dir.join("batch_001.csv"), &rows).map_err(io)?;

    // only what differs from the template defaults
    let config = r.join("config").join("config.yml");
    let pin = format!("  project_root: \"{root}\"");
    let lines = [
        "paths:", &pin, "filenames:", "  timezone: \"Asia/Kolkata\"", "birdnet:", "  run_id: \"run01\"",
        "occupancy:", "  run_id: \"run01\"", "  min_effort_hours: 0", "  min_detections: 1", "recall:",
        "  segment_s: 1", "filter:", "  workers: 1",
    ];
    write_lines(&config, &lines).map_err(io)?;
    Ok(Found { config, root })
}

/// Lines with the platform's line ending, like R's writeLines/write.csv
/// (CRLF on Windows).
fn write_lines(path: &Path, lines: &[&str]) -> std::io::Result<()> {
    let eol: &[u8] = if cfg!(windows) { b"\r\n" } else { b"\n" };
    let mut f = std::fs::File::create(path)?;
    for l in lines {
        f.write_all(l.as_bytes())?;
        f.write_all(eol)?;
    }
    Ok(())
}

/// The example project's folder: `<base>/pamflow example`, or `... 2`, `... 3`
/// when that one exists and is not empty.
pub fn example_folder(base: &Path) -> PathBuf {
    let empty_or_missing =
        |d: &Path| !d.is_dir() || std::fs::read_dir(d).map(|mut x| x.next().is_none()).unwrap_or(true);
    let mut dir = base.join("pamflow example");
    let mut n = 1;
    while !empty_or_missing(&dir) {
        n += 1;
        dir = base.join(format!("pamflow example {n}"));
    }
    dir
}

// ---- diagnostics -----------------------------------------------------------------

/// Facts about the computer the diagnostics file reports.
pub struct SystemFacts {
    pub os: String,
    pub memory_gb: Option<f64>,
    pub time_zone: String,
}

/// One readable text file to attach to a bug report (`pd_diagnostics`):
/// versions, computer, the project's folders and settings file, and the tail of
/// its newest logs. Paths (and so a user name) can appear in it.
pub fn diagnostics_text(job: &Value, found: Option<&Found>, facts: &SystemFacts, now: &str) -> String {
    let mut out: Vec<String> = vec![
        "pamflow Desktop diagnostics".into(),
        format!("made {now}"),
        "Look through this file before sending it: it can contain folder paths (and so your user name).".into(),
    ];
    let section = |out: &mut Vec<String>, x: &str| {
        out.push(String::new());
        out.push(format!("== {x} =="));
    };
    section(&mut out, "Versions");
    out.push(format!("app: {}", job["app_version"].as_str().unwrap_or("unknown")));
    out.push(format!("engine: pamflow-engine (Rust) {}", env!("CARGO_PKG_VERSION")));
    out.push(format!("pamflow settings template: {TEMPLATE_VERSION}"));
    out.push(format!("ffmpeg: {}", ffmpeg_version()));
    section(&mut out, "Computer");
    out.push(format!("system: {}", facts.os));
    out.push(format!("cores: {} physical, {} logical", num_cpus::get_physical(), num_cpus::get()));
    out.push(format!(
        "memory: {}",
        facts.memory_gb.map(|g| format!("{} GB", (g * 10.0).round() / 10.0)).unwrap_or_else(|| "unknown".into())
    ));
    out.push(format!("time zone: {}", facts.time_zone));
    section(&mut out, "Analysis engine");
    out.push(format!("runtime: {}", job["runtime"].as_str().unwrap_or("")));
    out.push(format!("downloaded models: {}", job["models_dir"].as_str().unwrap_or("unknown")));

    match found {
        None => {
            section(&mut out, "Project");
            out.push("No project is open (or no settings file was found).".into());
        }
        Some(found) => {
            section(&mut out, "Project");
            out.push(format!("settings file: {}", norm_path(&found.config)));
            out.push(format!("project folder: {}", found.root));
            let cfg = Config::read(&found.config);
            match &cfg {
                Err(e) => out.push(format!("pamflow could not read the settings file: {e}")),
                Ok(cfg) => {
                    section(&mut out, "Folders in the settings (exists?)");
                    if let Some(h) = cfg.data["paths"].as_hash() {
                        for (k, v) in h {
                            let (Some(k), Some(v)) = (k.as_str(), v.as_str()) else { continue };
                            if k == "project_root" {
                                continue;
                            }
                            let yes = if Path::new(v).exists() { "yes" } else { "NO " };
                            out.push(format!("{k:<20} {yes}  {v}"));
                        }
                    }
                }
            }
            section(&mut out, "Settings file");
            out.extend(ConfigText::read(&found.config).map(|t| t.lines).unwrap_or_default());
            let logs_dir = cfg.as_ref().ok().and_then(|c| c.data["paths"]["logs"].as_str().map(PathBuf::from));
            let mut logs: Vec<(std::time::SystemTime, PathBuf)> = logs_dir
                .and_then(|d| std::fs::read_dir(d).ok())
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.file_name().to_string_lossy().ends_with(".log"))
                        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                        .collect()
                })
                .unwrap_or_default();
            if logs.is_empty() {
                section(&mut out, "Logs");
                out.push("No log files yet.".into());
            }
            logs.sort_by_key(|l| std::cmp::Reverse(l.0));
            for (mtime, lg) in logs.into_iter().take(12) {
                let text = std::fs::read_to_string(&lg).unwrap_or_default();
                let lines: Vec<&str> = text.lines().collect();
                let changed: chrono::DateTime<chrono::Local> = mtime.into();
                section(
                    &mut out,
                    &format!(
                        "Log {} (last {} of {} lines, changed {})",
                        lg.file_name().unwrap_or_default().to_string_lossy(),
                        lines.len().min(150),
                        lines.len(),
                        changed.format("%Y-%m-%d %H:%M")
                    ),
                );
                out.extend(lines[lines.len().saturating_sub(150)..].iter().map(|s| s.to_string()));
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn ffmpeg_version() -> String {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string))
        .unwrap_or_else(|| "ffmpeg not found".into())
}

/// Write the diagnostics file to `target` (folders made as needed).
pub fn write_diagnostics(target: &str, text: &str) -> Result<(String, u64), String> {
    if target.is_empty() {
        return Err("No file was chosen.".into());
    }
    let target = norm_path(Path::new(target));
    let p = PathBuf::from(&target);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(&p, text).map_err(|e| format!("Could not write {target}: {e}"))?;
    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    Ok((target, bytes))
}
