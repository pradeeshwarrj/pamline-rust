//! Stage 02: high-pass filter the recordings that passed the scan (port of
//! pamflow 0.6.0 `pam_filter_audio`, `pam_filter_batch`, `pam_filter_one`,
//! `pam_highpass_filter_string`, `pam_butterworth_q`, `pam_check_ffmpeg`, and
//! pamflow Desktop's `filter_settings` in `bridge/run_job.R`).
//!
//! Each recording goes through ffmpeg with a Butterworth high-pass built as a
//! cascade of 2-pole sections (ffmpeg's `highpass` has only 2 poles), in
//! batches; a batch's results are appended to `metadata/02_filter_results.csv`
//! and its verified files to the checkpoint `metadata/02_filter.done`, so a
//! stopped run continues where it stopped.

use crate::audio_header::audio_info;
use crate::config::{num, Config};
use crate::inspect::yaml_json;
use crate::rcsv::{append_table, Cell, Table, EOL};
use crate::rnum::rsum;
use crate::stage_log::StageLog;
use crate::standardize::r_number;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use yaml_rust2::Yaml;

pub const STAGE: &str = "02_filter";
const RESULT_COLS: [&str; 4] = ["rel", "ok", "verified", "message"];

/// `pam_butterworth_q`: the Q of each 2-pole section of an even-order Butterworth.
pub fn butterworth_q(order: i64) -> Result<Vec<f64>, String> {
    if order < 2 || order % 2 != 0 {
        return Err("Filter order must be an even number >= 2".into());
    }
    Ok((1..=order / 2)
        .map(|k| 1.0 / (2.0 * ((2 * k - 1) as f64 * std::f64::consts::PI / (2 * order) as f64).sin()))
        .collect())
}

/// `pam_highpass_filter_string`: the ffmpeg filter chain.
pub fn highpass_filter_string(cutoff: &Yaml, order: i64) -> Result<String, String> {
    let hz = match num(cutoff) {
        Some(x) if x > 0.0 => x,
        _ => return Err("cutoff_hz must be a single positive number".into()),
    };
    let mut q = butterworth_q(order)?;
    q.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let cut = r_number(&Yaml::Real(hz.to_string()));
    Ok(q.iter().map(|q| format!("highpass=f={cut}:p=2:t=q:w={q:.6}:r=f64")).collect::<Vec<_>>().join(","))
}

fn command(ffmpeg: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    cmd
}

/// stdout and stderr lines, as R's `system2(stdout = TRUE, stderr = TRUE)` returns them.
fn run_lines(ffmpeg: &str, args: &[String]) -> Result<(bool, Vec<String>), String> {
    let out = command(ffmpeg).args(args).output().map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text.lines().map(str::to_string).collect()))
}

/// `pam_check_ffmpeg`: ffmpeg runs and accepts the filter; returns its version line.
pub fn check_ffmpeg(ffmpeg: &str, filter: &str) -> Result<String, String> {
    let version = run_lines(ffmpeg, &["-version".to_string()]);
    let (ok, lines) = match version {
        Ok((ok, lines)) => (ok, lines),
        Err(_) => (false, Vec::new()),
    };
    if !ok {
        return Err("FFmpeg could not be run.\n\u{2139} Install it (winget install --id Gyan.FFmpeg -e), restart RStudio,\n\u{2139} or set filter$ffmpeg_path in the config to the full path of ffmpeg.exe.".into());
    }
    let args: Vec<String> = ["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-t", "0.1", "-af", filter, "-f", "null", "-"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (tok, tlines) = run_lines(ffmpeg, &args).unwrap_or((false, Vec::new()));
    if !tok {
        let tail: Vec<&str> = tlines.iter().rev().take(3).rev().map(String::as_str).collect();
        return Err(format!(
            "This FFmpeg build rejected the high-pass filter chain.\n\u{2139} Update FFmpeg to a recent version.\n\u{2716} {}",
            tail.join(" | ")
        ));
    }
    Ok(lines.first().cloned().unwrap_or_default())
}

/// What is needed to filter one file.
#[derive(Clone)]
struct Item {
    rel: String,
    input: String,
    output: String,
    bits: Option<i64>,
    exp_dur: Option<f64>,
    exp_sr: Option<i64>,
    exp_ch: Option<i64>,
    size: Option<f64>,
}

/// `pam_filter_one`: filter one file into `<output>.part`, then move it into place.
fn filter_one(item: &Item, filter: &str, ffmpeg: &str) -> (bool, String) {
    let ext = Path::new(&item.output).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let codec: Vec<&str> = if ext == "flac" {
        vec!["-c:a", "flac", "-f", "flac"]
    } else {
        let c = match item.bits {
            Some(24) => "pcm_s24le",
            Some(32) => "pcm_s32le",
            _ => "pcm_s16le",
        };
        vec!["-c:a", c, "-f", "wav"]
    };
    let tmp = format!("{}.part", item.output);
    if let Some(dir) = Path::new(&item.output).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut args: Vec<String> = ["-y", "-nostdin", "-v", "error", "-i", &item.input, "-af", filter].iter().map(|s| s.to_string()).collect();
    args.extend(codec.iter().map(|s| s.to_string()));
    args.push(tmp.clone());
    let (status_ok, lines) = run_lines(ffmpeg, &args).unwrap_or_else(|e| (false, vec![e]));
    let mut ok = status_ok && std::fs::metadata(&tmp).map(|m| m.len() > 44).unwrap_or(false);
    if ok {
        let _ = std::fs::remove_file(&item.output);
        ok = std::fs::rename(&tmp, &item.output).is_ok();
    }
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    let message = if ok { String::new() } else { lines.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join(" | ") };
    (ok, message)
}

/// One finished file of a batch.
struct Done {
    rel: String,
    ok: bool,
    verified: bool,
    message: String,
}

/// `pam_filter_batch`: filter the files (with `workers` at a time), then check each output.
fn filter_batch(batch: &[Item], filter: &str, ffmpeg: &str, workers: usize, tolerance: f64) -> Vec<Done> {
    let results: Vec<std::sync::Mutex<Option<(bool, String)>>> = batch.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1).min(batch.len().max(1)) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= batch.len() {
                    break;
                }
                *results[i].lock().unwrap() = Some(filter_one(&batch[i], filter, ffmpeg));
            });
        }
    });
    batch
        .iter()
        .zip(results)
        .map(|(item, r)| {
            let (ok, message) = r.into_inner().unwrap().unwrap_or((false, String::new()));
            if !ok {
                return Done { rel: item.rel.clone(), ok, verified: false, message };
            }
            let (info, _) = audio_info(Path::new(&item.output));
            let verified = info.readable
                && matches!((info.duration_s, item.exp_dur), (Some(d), Some(e)) if (d - e).abs() <= tolerance)
                && info.sample_rate.is_some() && info.sample_rate == item.exp_sr
                && info.channels.is_some() && info.channels == item.exp_ch;
            if !verified {
                let _ = std::fs::remove_file(&item.output);
                return Done {
                    rel: item.rel.clone(),
                    ok,
                    verified,
                    message: "output failed verification (unreadable or length/format mismatch)".into(),
                };
            }
            Done { rel: item.rel.clone(), ok, verified, message }
        })
        .collect()
}

/// Progress the engine turns into events right after the matching log line.
pub enum Progress<'a> {
    /// "N files to filter": the work to do (0 when nothing is pending).
    Total(usize),
    /// A batch finished: number, of how many, files ok, files failed, new result rows.
    Batch { batch: usize, batches: usize, ok: usize, failed: usize, rows: &'a [Value] },
}

/// What the stage did.
pub struct Outcome {
    pub n_pending: usize,
    pub n_ok: usize,
    pub n_failed: usize,
}

pub fn checkpoint_path(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.data["paths"]["metadata"].as_str().unwrap_or("")).join("02_filter.done")
}

pub fn results_path(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.data["paths"]["metadata"].as_str().unwrap_or("")).join("02_filter_results.csv")
}

/// The checkpoint's lines, unique, in order (`pam_checkpoint_read`).
fn read_checkpoint(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    text.lines().map(|l| l.trim_end_matches('\r').to_string()).filter(|l| seen.insert(l.clone())).collect()
}

/// Files in the checkpoint (`pd_checkpoint_count`): unique, trimmed, not empty.
pub fn checkpoint_count(path: &Path) -> usize {
    let Ok(text) = std::fs::read_to_string(path) else { return 0 };
    let set: std::collections::HashSet<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    set.len()
}

fn rel_to(path: &str, root: &str) -> String {
    let root = root.trim_end_matches('/');
    path.strip_prefix(&format!("{root}/")).unwrap_or(path).to_string()
}

fn opt_i(s: &str) -> Option<i64> {
    s.trim().parse::<f64>().ok().map(|x| x as i64)
}

fn opt_f(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

/// New rows of the results file since `seen`, as the "files" event shows them
/// (`pd_new_results` + the rename in `job_filter`): rel, ok, message. A message
/// column that is empty in every new row reads as null (fread types it logical).
fn new_rows(path: &Path, seen: usize) -> (Vec<Value>, usize) {
    let Some(t) = Table::read(path) else { return (Vec::new(), seen) };
    if t.rows.len() <= seen {
        return (Vec::new(), seen);
    }
    let (rel, ver, msg) = (t.col("rel"), t.col("verified"), t.col("message"));
    let slice = &t.rows[seen..];
    let all_empty = msg.is_none_or(|m| slice.iter().all(|r| r[m].is_empty() || r[m] == "NA"));
    let rows = slice
        .iter()
        .map(|r| {
            json!({
                "rel": rel.map(|i| r[i].clone()).unwrap_or_default(),
                "ok": ver.map(|i| r[i] == "TRUE").unwrap_or(false),
                "message": if all_empty { Value::Null } else { json!(msg.map(|i| r[i].clone()).unwrap_or_default()) },
            })
        })
        .collect();
    (rows, t.rows.len())
}

/// `pam_filter_audio`. `progress` is called right after the log line each step belongs to.
pub fn filter_audio(cfg: &Config, log: &mut StageLog, progress: &mut dyn FnMut(Progress)) -> Result<Option<Outcome>, String> {
    let fc = &cfg.data["filter"];
    if !fc["enabled"].as_bool().unwrap_or(false) {
        log.info("filter disabled in config; nothing to do");
        return Ok(None);
    }
    let order = num(&fc["order"]).unwrap_or(8.0) as i64;
    let filter = highpass_filter_string(&fc["highpass_hz"], order)?;
    let ffmpeg = fc["ffmpeg_path"].as_str().unwrap_or("ffmpeg").to_string();
    let version = check_ffmpeg(&ffmpeg, &filter)?;
    log.info(&format!("using {version}"));
    log.info(&format!("filter chain: {filter}"));

    let paths = &cfg.data["paths"];
    let metadata = paths["metadata"].as_str().unwrap_or("");
    let master_path = format!("{metadata}/audio_master.csv");
    let master = Table::read(Path::new(&master_path)).filter(|_| Path::new(&master_path).is_file());
    let Some(master) = master else { return Err("audio_master.csv not found. Run Stage 1 first.".into()) };
    let root = paths["audio_standardized"].as_str().unwrap_or("").to_string();
    let out_dir = paths["filtered"].as_str().unwrap_or("").to_string();
    let col = |n: &str| master.col(n);
    let (c_path, c_dur, c_sr, c_ch, c_bits, c_size, c_qc) =
        (col("file_path"), col("duration_s"), col("sample_rate"), col("channels"), col("bits"), col("file_size_b"), col("qc_pass"));
    let get = |r: &Vec<String>, c: Option<usize>| c.map(|i| r[i].clone()).unwrap_or_default();
    let todo: Vec<Item> = master
        .rows
        .iter()
        .filter(|r| get(r, c_qc) == "TRUE")
        .map(|r| {
            let input = get(r, c_path);
            let rel = rel_to(&input, &root);
            Item {
                output: format!("{out_dir}/{rel}"),
                rel,
                input,
                bits: opt_i(&get(r, c_bits)),
                exp_dur: opt_f(&get(r, c_dur)),
                exp_sr: opt_i(&get(r, c_sr)),
                exp_ch: opt_i(&get(r, c_ch)),
                size: opt_f(&get(r, c_size)),
            }
        })
        .collect();

    let ckpt = checkpoint_path(cfg);
    let results = results_path(cfg);
    let done: std::collections::HashSet<String> =
        read_checkpoint(&ckpt).into_iter().filter(|d| Path::new(&format!("{out_dir}/{d}")).exists()).collect();
    let pending: Vec<Item> = todo.into_iter().filter(|i| !done.contains(&i.rel)).collect();
    if pending.is_empty() {
        log.info("nothing pending; all files already filtered");
        progress(Progress::Total(0));
        return Ok(Some(Outcome { n_pending: 0, n_ok: 0, n_failed: 0 }));
    }
    log.info(&format!(
        "{} files to filter ({:.1} GB input; filtered copies need about the same space in {})",
        pending.len(),
        rsum(pending.iter().filter_map(|i| i.size)) / 1e9,
        out_dir
    ));
    progress(Progress::Total(pending.len()));

    let chunk = num(&fc["chunk_size"]).filter(|c| *c >= 1.0).unwrap_or(200.0) as usize;
    let workers = num(&fc["workers"]).filter(|w| *w >= 1.0).unwrap_or(1.0) as usize;
    let tolerance = num(&fc["duration_tolerance_s"]).unwrap_or(0.5);
    let batches: Vec<&[Item]> = pending.chunks(chunk).collect();
    let mut seen = Table::read(&results).map(|t| t.rows.len()).unwrap_or(0);
    let (mut n_ok, mut n_fail) = (0usize, 0usize);
    for (b, batch) in batches.iter().enumerate() {
        let res = filter_batch(batch, &filter, &ffmpeg, workers, tolerance);
        let rows: Vec<Vec<Cell>> = res
            .iter()
            .map(|d| vec![Cell::Str(d.rel.clone()), Cell::Bool(d.ok), Cell::Bool(d.verified), Cell::Str(d.message.clone())])
            .collect();
        append_table(&results, &RESULT_COLS, &rows).map_err(|e| format!("Could not write {}: {e}", results.display()))?;
        if let Some(dir) = ckpt.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let verified: Vec<&str> = res.iter().filter(|d| d.verified).map(|d| d.rel.as_str()).collect();
        if !verified.is_empty() {
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&ckpt).map_err(|e| e.to_string())?;
            for rel in verified {
                f.write_all(format!("{rel}{EOL}").as_bytes()).map_err(|e| e.to_string())?;
            }
        }
        let ok = res.iter().filter(|d| d.verified).count();
        let failed = res.len() - ok;
        n_ok += ok;
        n_fail += failed;
        log.info(&format!("batch {}/{}: {} ok, {} failed", b + 1, batches.len(), ok, failed));
        let (rows, now) = new_rows(&results, seen);
        seen = now;
        progress(Progress::Batch { batch: b + 1, batches: batches.len(), ok, failed, rows: &rows });
    }
    log.info(&format!("filtering finished: {n_ok} ok, {n_fail} failed of {}", pending.len()));
    Ok(Some(Outcome { n_pending: pending.len(), n_ok, n_failed: n_fail }))
}

/// `filter_settings`: the "filter_info" event's settings.
pub fn filter_settings(cfg: &Config) -> Value {
    let fc = &cfg.data["filter"];
    let metadata = cfg.data["paths"]["metadata"].as_str().unwrap_or("");
    let master_path = format!("{metadata}/audio_master.csv");
    let exists = Path::new(&master_path).is_file();
    let input_gb = Table::read(Path::new(&master_path))
        .filter(|_| exists)
        .and_then(|t| {
            let (sz, qc) = (t.col("file_size_b")?, t.col("qc_pass")?);
            let bytes = rsum(t.rows.iter().filter(|r| r[qc] == "TRUE").filter_map(|r| opt_f(&r[sz])));
            Some(((bytes / 1024f64.powi(3)) * 10.0).round_ties_even() / 10.0)
        })
        .map(|x| json!(x))
        .unwrap_or(Value::Null);
    json!({
        "enabled": fc["enabled"].as_bool().unwrap_or(false),
        "highpass_hz": yaml_json(&fc["highpass_hz"], false),
        "order": yaml_json(&fc["order"], false),
        "workers": yaml_json(&fc["workers"], false),
        "chunk_size": yaml_json(&fc["chunk_size"], false),
        "output_folder": yaml_json(&cfg.data["paths"]["filtered"], false),
        "scanned": exists,
        "input_gb": input_gb,
        "already_done": checkpoint_count(&checkpoint_path(cfg)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_like_r() {
        let got = highpass_filter_string(&Yaml::Integer(250), 8).unwrap();
        assert_eq!(
            got,
            "highpass=f=250:p=2:t=q:w=0.509796:r=f64,highpass=f=250:p=2:t=q:w=0.601345:r=f64,\
             highpass=f=250:p=2:t=q:w=0.899976:r=f64,highpass=f=250:p=2:t=q:w=2.562915:r=f64"
        );
        assert_eq!(highpass_filter_string(&Yaml::Real("62.5".into()), 2).unwrap(), "highpass=f=62.5:p=2:t=q:w=0.707107:r=f64");
    }

    #[test]
    fn bad_settings_are_refused() {
        assert_eq!(highpass_filter_string(&Yaml::Integer(250), 3).unwrap_err(), "Filter order must be an even number >= 2");
        assert_eq!(highpass_filter_string(&Yaml::Integer(0), 8).unwrap_err(), "cutoff_hz must be a single positive number");
        assert!(highpass_filter_string(&Yaml::Null, 8).is_err());
    }

    #[test]
    fn paths_relative_to_the_recordings_folder() {
        assert_eq!(rel_to("D:/a/std/G1/x.wav", "D:/a/std"), "G1/x.wav");
        assert_eq!(rel_to("D:/a/std/G1/x.wav", "D:/a/std/"), "G1/x.wav");
    }
}
