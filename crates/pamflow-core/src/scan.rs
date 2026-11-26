//! Stage 01: scan the recordings (port of pamflow 0.6.0 `pam_scan_audio`,
//! `pam_audio_info`, `pam_add_qc`, `pam_read_deployments`,
//! `pam_check_deployments`, `pam_match_deployments`, `pam_audio_summary`, as
//! installed in pamflow Desktop 0.2.1). Writes, byte for byte as R does:
//! `metadata/audio_headers.csv` (cache), `metadata/audio_master.csv`,
//! `outputs/audio_summary_per_grid.csv`, `outputs/deployment_issues.csv`.
//! See tools/compare/SCAN_NOTES.md for the behaviour this reproduces.

use crate::audio_files::{list_audio_r, parse_filenames};
use crate::audio_header::audio_info;
use crate::config::{num, Config};
use crate::rcsv::{append_table, write_table, Cell, Table};
use crate::stage_log::StageLog;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use yaml_rust2::Yaml;

pub const STAGE: &str = "01_scan";

/// R's three-valued logic (`TRUE`, `FALSE`, `NA`).
type Tri = Option<bool>;

fn and(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn not(a: Tri) -> Tri {
    a.map(|v| !v)
}

/// One cached header row.
#[derive(Debug, Clone, Default)]
struct HeaderRow {
    file_size_b: Option<f64>,
    sample_rate: Option<i64>,
    channels: Option<i64>,
    bits: Option<i64>,
    duration_s: Option<f64>,
    readable: Tri,
}

/// One row of audio_master.csv.
#[derive(Debug, Clone, Default)]
pub struct MasterRow {
    pub file_path: String,
    pub file_name: String,
    pub grid: Option<String>,
    pub start_time: Option<f64>,
    pub valid_name: bool,
    h: HeaderRow,
    pub end_time: Option<f64>,
    pub truncated: Tri,
    pub modal_duration_s: Option<f64>,
    pub short: Tri,
    pub duplicate: bool,
    pub gap_s: Option<f64>,
    pub gap_before: bool,
    pub overlap_before: bool,
    pub qc_pass: Tri,
    pub deployment: Option<usize>,
}

/// One deployment-sheet row after cleaning.
#[derive(Debug, Clone)]
pub struct Deployment {
    pub fields: HashMap<String, Option<String>>,
    pub grid: String,
    pub deploy_start: Option<f64>,
    pub retrieve_end: Option<f64>,
    pub recovery_time_missing: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl Deployment {
    fn id(&self) -> Option<&str> {
        self.fields.get("deployment_id").and_then(|v| v.as_deref())
    }
    fn field(&self, k: &str) -> Option<&str> {
        self.fields.get(k).and_then(|v| v.as_deref())
    }
}

fn s(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Real(r) => Some(r.clone()),
        Yaml::Boolean(b) => Some(if *b { "TRUE" } else { "FALSE" }.into()),
        _ => None,
    }
}

/// What a scan produced.
pub struct ScanResult {
    pub master: Vec<MasterRow>,
}

const HEADER_COLS: [&str; 7] = ["file_path", "file_size_b", "sample_rate", "channels", "bits", "duration_s", "readable"];

fn parse_tri(v: &str) -> Tri {
    match v {
        "TRUE" | "true" | "T" => Some(true),
        "FALSE" | "false" | "F" => Some(false),
        _ => None,
    }
}

fn parse_f(v: &str) -> Option<f64> {
    if v.is_empty() || v == "NA" {
        None
    } else {
        v.parse().ok()
    }
}

fn parse_i(v: &str) -> Option<i64> {
    parse_f(v).map(|x| x as i64)
}

/// Read the header cache: rows in file order, first occurrence per path.
fn read_header_cache(path: &Path) -> Vec<(String, HeaderRow)> {
    let Some(t) = Table::read(path) else { return Vec::new() };
    let idx = |n: &str| t.col(n);
    let (fp, sz, sr, ch, bi, du, rd) =
        (idx("file_path"), idx("file_size_b"), idx("sample_rate"), idx("channels"), idx("bits"), idx("duration_s"), idx("readable"));
    let Some(fp) = fp else { return Vec::new() };
    let get = |r: &Vec<String>, i: Option<usize>| i.map(|i| r[i].clone()).unwrap_or_default();
    t.rows
        .iter()
        .map(|r| {
            (
                r[fp].clone(),
                HeaderRow {
                    file_size_b: parse_f(&get(r, sz)),
                    sample_rate: parse_i(&get(r, sr)),
                    channels: parse_i(&get(r, ch)),
                    bits: parse_i(&get(r, bi)),
                    duration_s: parse_f(&get(r, du)),
                    readable: parse_tri(&get(r, rd)),
                },
            )
        })
        .collect()
}

fn header_cells(path: &str, info: &crate::audio_header::AudioInfo, size: Option<f64>) -> Vec<Cell> {
    vec![
        Cell::Str(path.to_string()),
        Cell::opt_num(size),
        Cell::opt_int(info.sample_rate),
        Cell::opt_int(info.channels),
        Cell::opt_int(info.bits),
        Cell::opt_num(info.duration_s),
        Cell::Bool(info.readable),
    ]
}

/// `sprintf("%04d", as.integer(x))`: R's as.integer of text (via a number,
/// truncated); NA as "  NA".
fn grid_number(x: Option<&str>) -> String {
    let v = x.and_then(r_as_numeric).filter(|v| v.is_finite() && v.abs() < 2_147_483_648.0).map(|v| v.trunc() as i64);
    match v {
        Some(n) if n < 0 => format!("-{:03}", -n),
        Some(n) => format!("{n:04}"),
        None => "  NA".into(),
    }
}

/// R's `as.numeric(<text>)`: surrounding blanks allowed, hex, Inf; else NA.
fn r_as_numeric(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() || t == "NA" {
        return None;
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(h, 16).ok().map(|v| v as f64);
    }
    match t {
        "Inf" | "inf" => return Some(f64::INFINITY),
        "-Inf" | "-inf" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let v: f64 = t.parse().ok()?;
    Some(v)
}

/// `pam_read_deployments`: the sheet, cleaned; plus R's warnings.
pub fn read_deployments(cfg: &Config) -> Result<(Vec<Deployment>, Vec<String>), String> {
    let path = cfg.data["paths"]["deployment_sheet"].as_str().unwrap_or("").to_string();
    if !Path::new(&path).is_file() {
        return Err(format!("Deployment sheet not found: {path}"));
    }
    let d = &cfg.data["deployments"];
    let cols: Vec<String> = match &d["columns"] {
        Yaml::Array(a) => a.iter().filter_map(s).collect(),
        other => s(other).into_iter().collect(),
    };
    let has_header = d["has_header"].as_bool().unwrap_or(false);
    let mut r = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(&path)
        .map_err(|e| format!("Could not read the deployment sheet: {e}"))?;
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    for (k, rec) in r.records().enumerate() {
        let Ok(rec) = rec else { continue };
        if k == 0 && has_header {
            continue;
        }
        if rec.iter().all(|f| f.trim().is_empty()) {
            continue; // blank line
        }
        rows.push(
            rec.iter()
                .map(|f| {
                    let t = f.trim();
                    if t.is_empty() || t == "NA" {
                        None
                    } else {
                        Some(t.to_string())
                    }
                })
                .collect(),
        );
    }
    let ncol = rows.iter().map(Vec::len).max().unwrap_or(0);
    if ncol < cols.len() {
        return Err(format!("Deployment sheet has {} columns; config expects {}", ncol, cols.len()));
    }
    let tz_name = cfg.data["filenames"]["timezone"].as_str().unwrap_or("UTC");
    let tz: chrono_tz::Tz = tz_name.parse().map_err(|_| format!("unknown time zone: {tz_name}"))?;
    let prefix = s(&d["grid_prefix"]).unwrap_or_default();
    let date_format = s(&d["date_format"]).unwrap_or_else(|| "%d-%m-%Y".into());
    let time_format = s(&d["time_format"]).unwrap_or_else(|| "%H:%M".into());
    let default_time = s(&d["default_recovery_time"]);
    let clean_date = |x: Option<&str>| x.map(|v| v.trim().replace(['.', '/'], "-"));
    let clean_time = |x: Option<&str>| {
        x.map(|v| {
            let v = v.trim().replace('.', ":");
            // "HH:MM:SS" -> "HH:MM"
            let parts: Vec<&str> = v.split(':').collect();
            let digits = |p: &str, lo: usize, hi: usize| p.len() >= lo && p.len() <= hi && p.chars().all(|c| c.is_ascii_digit());
            if parts.len() == 3 && digits(parts[0], 1, 2) && digits(parts[1], 2, 2) && digits(parts[2], 2, 2) {
                format!("{}:{}", parts[0], parts[1])
            } else {
                v
            }
        })
    };
    let mut warnings = Vec::new();
    let mut out = Vec::new();
    let mut bad_year_ids: Vec<String> = Vec::new();
    let mut lat_bad = false;
    let mut lon_bad = false;
    for row in rows {
        let mut fields = HashMap::new();
        for (i, c) in cols.iter().enumerate() {
            fields.insert(c.clone(), row.get(i).cloned().flatten());
        }
        let f = |k: &str| fields.get(k).cloned().flatten();
        let grid = format!("{prefix}{}", grid_number(f("grid").as_deref()));
        let deploy_day = clean_date(f("deploy_date").as_deref()).and_then(|v| crate::rtime::parse_date(&v, &date_format));
        let mut retrieve_day =
            clean_date(f("retrieve_date").as_deref()).and_then(|v| crate::rtime::parse_date(&v, &date_format));
        if let (Some(rd), Some(dd)) = (retrieve_day, deploy_day) {
            use chrono::Datelike;
            if rd.year() < 2000 {
                retrieve_day = chrono::NaiveDate::from_ymd_opt(dd.year(), rd.month(), rd.day());
                bad_year_ids.push(f("deployment_id").unwrap_or_else(|| "NA".into()));
            }
        }
        let mut retrieve_time = f("retrieve_time");
        let recovery_time_missing = retrieve_time.is_none() && retrieve_day.is_some();
        if recovery_time_missing {
            retrieve_time = default_time.clone();
        }
        let fmt = format!("%Y-%m-%d {time_format}");
        let at = |day: Option<chrono::NaiveDate>, time: Option<String>| -> Option<f64> {
            let text = format!("{} {}", day?.format("%Y-%m-%d"), clean_time(time.as_deref())?);
            crate::rtime::parse_local_time(&text, &fmt, &tz)
        };
        let deploy_start = at(deploy_day, f("deploy_time"));
        let retrieve_end = at(retrieve_day, retrieve_time.clone());
        let num_field = |k: &str, bad: &mut bool| -> Option<f64> {
            let v = f(k)?;
            let n = r_as_numeric(&v);
            if n.is_none() {
                *bad = true;
            }
            n
        };
        let latitude = num_field("latitude", &mut lat_bad);
        let longitude = num_field("longitude", &mut lon_bad);
        if let Some(t) = &retrieve_time {
            fields.insert("retrieve_time".into(), Some(t.clone()));
        }
        out.push(Deployment { fields, grid, deploy_start, retrieve_end, recovery_time_missing, latitude, longitude });
    }
    if !bad_year_ids.is_empty() {
        warnings.push(format!("Recovery year repaired using deployment year for rows: {}", bad_year_ids.join(", ")));
    }
    // as.numeric on each column warns once per column with failures
    for bad in [lat_bad, lon_bad] {
        if bad {
            warnings.push("NAs introduced by coercion".into());
        }
    }
    Ok((out, warnings))
}

/// `order()` keys with R's NA-first, C-locale text order.
fn cmp_opt_str(a: &Option<String>, b: &Option<String>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Less,
        (_, None) => Ordering::Greater,
        (Some(x), Some(y)) => x.as_bytes().cmp(y.as_bytes()),
    }
}

fn cmp_opt_f(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Less,
        (_, None) => Ordering::Greater,
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
    }
}

/// `pam_check_deployments`: (deployment_id, grid, issue) rows.
pub fn check_deployments(dep: &[Deployment]) -> Vec<(Option<String>, String, &'static str)> {
    let mut order: Vec<usize> = (0..dep.len()).collect();
    order.sort_by(|&a, &b| {
        dep[a].grid.as_bytes().cmp(dep[b].grid.as_bytes()).then_with(|| cmp_opt_f(dep[a].deploy_start, dep[b].deploy_start))
    });
    let mut overlaps = vec![false; dep.len()];
    for w in order.windows(2) {
        let (p, c) = (&dep[w[0]], &dep[w[1]]);
        if p.grid == c.grid {
            if let (Some(ds), Some(re)) = (c.deploy_start, p.retrieve_end) {
                overlaps[w[1]] = ds < re;
            }
        }
    }
    let mut out = Vec::new();
    let mut flag = |pick: &dyn Fn(usize) -> bool, text: &'static str| {
        for &i in &order {
            if pick(i) {
                out.push((dep[i].id().map(str::to_string), dep[i].grid.clone(), text));
            }
        }
    };
    flag(&|i| dep[i].deploy_start.is_none(), "deployment start missing or unparseable");
    flag(&|i| dep[i].retrieve_end.is_none(), "recovery date missing or unparseable (never matched to audio)");
    flag(
        &|i| matches!((dep[i].retrieve_end, dep[i].deploy_start), (Some(r), Some(d)) if r < d),
        "recovery before deployment",
    );
    flag(&|i| dep[i].recovery_time_missing, "recovery time blank; end of day assumed");
    flag(&|i| overlaps[i], "overlaps another deployment of the same grid");
    out
}

fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

/// Run the scan (`pam_scan_audio`). Errors are pamflow's messages.
pub fn scan_audio(cfg: &Config, force: bool, log: &mut StageLog) -> Result<ScanResult, String> {
    let paths = &cfg.data["paths"];
    let p = |k: &str| PathBuf::from(paths[k].as_str().unwrap_or(""));
    let (metadata, outputs) = (p("metadata"), p("outputs"));
    let _ = std::fs::create_dir_all(&metadata);
    let _ = std::fs::create_dir_all(&outputs);
    let audio_dir = paths["audio_standardized"].as_str().unwrap_or("").to_string();
    if !Path::new(&audio_dir).is_dir() {
        return Err(format!("Audio folder not found: {audio_dir}"));
    }
    let files = list_audio_r(&audio_dir);
    if files.is_empty() {
        return Err("No .wav/.flac files found in the audio folder".into());
    }
    log.info(&format!("found {} audio files (wav/flac)", files.len()));

    // ---- header cache
    let hdr_path = metadata.join("audio_headers.csv");
    if force && hdr_path.exists() {
        let _ = std::fs::remove_file(&hdr_path);
    }
    let cached: std::collections::HashSet<String> = read_header_cache(&hdr_path).into_iter().map(|(p, _)| p).collect();
    let mut seen = std::collections::HashSet::new();
    let todo: Vec<&String> = files.iter().filter(|f| !cached.contains(*f) && seen.insert(f.as_str())).collect();
    let chunk = num(&cfg.data["qc"]["header_chunk_size"]).filter(|c| *c >= 1.0).unwrap_or(5000.0) as usize;
    for part in todo.chunks(chunk) {
        let rows: Vec<Vec<Cell>> = part
            .iter()
            .map(|f| {
                let (info, warnings) = audio_info(Path::new(f.as_str()));
                for w in warnings {
                    log.warning(&w);
                }
                let size = std::fs::metadata(f.as_str()).ok().map(|m| m.len() as f64);
                header_cells(f, &info, size)
            })
            .collect();
        append_table(&hdr_path, &HEADER_COLS, &rows).map_err(|e| format!("Could not write {}: {e}", hdr_path.display()))?;
    }
    log.info(&format!("read headers for {} new files", todo.len()));
    let mut headers: HashMap<String, HeaderRow> = HashMap::new();
    for (path, h) in read_header_cache(&hdr_path) {
        headers.entry(path).or_insert(h);
    }

    // ---- names, headers, QC
    let pattern = cfg.data["filenames"]["pattern"].as_str().unwrap_or("");
    let tz = cfg.data["filenames"]["timezone"].as_str().unwrap_or("UTC");
    let parsed = parse_filenames(&files, pattern, tz).map_err(|e| e.to_string())?;
    let mut master: Vec<MasterRow> = files
        .iter()
        .zip(parsed)
        .map(|(f, pn)| {
            let valid = pn.valid();
            MasterRow {
                file_path: f.clone(),
                file_name: pn.file_name,
                grid: pn.grid,
                start_time: pn.start_utc.map(|t| t as f64),
                valid_name: valid,
                h: headers.get(f).cloned().unwrap_or_default(),
                ..Default::default()
            }
        })
        .collect();
    let tol = num(&cfg.data["qc"]["gap_tolerance_s"]);
    for m in master.iter_mut() {
        m.end_time = match (m.start_time, m.h.duration_s) {
            (Some(s), Some(d)) => Some(s + d),
            _ => None,
        };
        let expected = match (m.h.bits, m.h.duration_s, m.h.sample_rate, m.h.channels) {
            (Some(b), Some(d), Some(r), Some(c)) => Some(44.0 + d * r as f64 * c as f64 * b as f64 / 8.0),
            (Some(_), _, _, _) => None, // NA arithmetic
            _ => None,
        };
        let small = match (m.h.file_size_b, expected) {
            (Some(sz), Some(e)) => Some(sz < 0.99 * e),
            _ => None,
        };
        m.truncated = and(and(m.h.readable, Some(expected.is_some())), if expected.is_some() { small } else { Some(true) });
    }
    // modal duration per grid (most common rounded duration; ties: the smallest)
    let mut modal: HashMap<Option<String>, Option<f64>> = HashMap::new();
    {
        let mut by_grid: HashMap<Option<String>, Vec<f64>> = HashMap::new();
        for m in &master {
            let e = by_grid.entry(m.grid.clone()).or_default();
            if let Some(d) = m.h.duration_s {
                e.push(round_half_even(d));
            }
        }
        for (g, mut v) in by_grid {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let mut best: Option<(f64, usize)> = None;
            let mut i = 0;
            while i < v.len() {
                let mut j = i;
                while j < v.len() && v[j] == v[i] {
                    j += 1;
                }
                if best.is_none_or(|(_, n)| j - i > n) {
                    best = Some((v[i], j - i));
                }
                i = j;
            }
            modal.insert(g, best.map(|b| b.0));
        }
    }
    let mut dup_n: HashMap<(Option<String>, Option<u64>), usize> = HashMap::new();
    for m in &master {
        *dup_n.entry((m.grid.clone(), m.start_time.map(f64::to_bits))).or_default() += 1;
    }
    for m in master.iter_mut() {
        m.modal_duration_s = modal.get(&m.grid).copied().flatten();
        let shorter = match (m.h.duration_s, m.modal_duration_s) {
            (Some(d), Some(md)) => Some(d < 0.99 * md),
            _ => None,
        };
        m.short = and(m.h.readable, shorter);
        m.duplicate = m.valid_name && dup_n[&(m.grid.clone(), m.start_time.map(f64::to_bits))] > 1;
    }
    // setorder(grid, start_time): C locale, NA first, stable
    master.sort_by(|a, b| cmp_opt_str(&a.grid, &b.grid).then_with(|| cmp_opt_f(a.start_time, b.start_time)));
    let mut prev_end: HashMap<Option<String>, Option<f64>> = HashMap::new();
    for m in master.iter_mut() {
        if !m.valid_name {
            continue;
        }
        m.gap_s = match prev_end.get(&m.grid) {
            Some(Some(pe)) => m.start_time.map(|s| s - pe),
            _ => None,
        };
        prev_end.insert(m.grid.clone(), m.end_time);
    }
    for m in master.iter_mut() {
        m.gap_before = matches!((m.gap_s, tol), (Some(g), Some(t)) if g > t);
        m.overlap_before = matches!((m.gap_s, tol), (Some(g), Some(t)) if g < -t);
        m.qc_pass = and(and(and(Some(m.valid_name), m.h.readable), not(m.truncated)), Some(!m.duplicate));
    }

    // ---- deployments
    let (dep, warnings) = read_deployments(cfg)?;
    for w in warnings {
        log.warning(&w);
    }
    let issues = check_deployments(&dep);
    let issue_rows: Vec<Vec<Cell>> = issues
        .iter()
        .map(|(id, g, t)| vec![Cell::opt_str(id.as_deref()), Cell::Str(g.clone()), Cell::Str(t.to_string())])
        .collect();
    write_table(&outputs.join("deployment_issues.csv"), &["deployment_id", "grid", "issue"], &issue_rows)
        .map_err(|e| e.to_string())?;
    if !issues.is_empty() {
        log.warn(&format!("{} deployment issues, see outputs/deployment_issues.csv", issues.len()));
    }
    for m in master.iter_mut() {
        let (Some(g), Some(st)) = (&m.grid, m.start_time) else { continue };
        // the first deployment in the sheet's order that covers the start
        m.deployment = dep.iter().position(|d| {
            &d.grid == g && d.deploy_start.is_some_and(|ds| ds <= st) && d.retrieve_end.is_some_and(|re| re >= st)
        });
    }

    // ---- write
    let master_rows: Vec<Vec<Cell>> = master
        .iter()
        .map(|m| {
            let d = m.deployment.map(|i| &dep[i]);
            vec![
                Cell::Str(m.file_path.clone()),
                Cell::Str(m.file_name.clone()),
                Cell::opt_str(m.grid.as_deref()),
                Cell::opt_time(m.start_time),
                Cell::Bool(m.valid_name),
                Cell::opt_num(m.h.file_size_b),
                Cell::opt_int(m.h.sample_rate),
                Cell::opt_int(m.h.channels),
                Cell::opt_int(m.h.bits),
                Cell::opt_num(m.h.duration_s),
                Cell::opt_bool(m.h.readable),
                Cell::opt_time(m.end_time),
                Cell::opt_bool(m.truncated),
                Cell::opt_num(m.modal_duration_s),
                Cell::opt_bool(m.short),
                Cell::Bool(m.duplicate),
                Cell::opt_num(m.gap_s),
                Cell::Bool(m.gap_before),
                Cell::Bool(m.overlap_before),
                Cell::opt_bool(m.qc_pass),
                Cell::opt_str(d.and_then(|d| d.id())),
                Cell::opt_str(d.and_then(|d| d.field("recorder"))),
                Cell::opt_num(d.and_then(|d| d.latitude)),
                Cell::opt_num(d.and_then(|d| d.longitude)),
                Cell::Bool(d.is_some()),
            ]
        })
        .collect();
    write_table(
        &metadata.join("audio_master.csv"),
        &[
            "file_path", "file_name", "grid", "start_time", "valid_name", "file_size_b", "sample_rate", "channels",
            "bits", "duration_s", "readable", "end_time", "truncated", "modal_duration_s", "short", "duplicate",
            "gap_s", "gap_before", "overlap_before", "qc_pass", "deployment_id", "recorder", "latitude", "longitude",
            "in_deployment",
        ],
        &master_rows,
    )
    .map_err(|e| e.to_string())?;

    // ---- per grid, in the sorted order of first appearance
    let mut groups: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for (i, m) in master.iter().enumerate() {
        match groups.iter_mut().find(|(g, _)| *g == m.grid) {
            Some((_, v)) => v.push(i),
            None => groups.push((m.grid.clone(), vec![i])),
        }
    }
    let sum_bool = |v: &[usize], f: &dyn Fn(&MasterRow) -> Tri| -> Option<i64> {
        let mut n = 0;
        for &i in v {
            match f(&master[i]) {
                Some(true) => n += 1,
                Some(false) => {}
                None => return None,
            }
        }
        Some(n)
    };
    let grid_rows: Vec<Vec<Cell>> = groups
        .iter()
        .map(|(g, v)| {
            let durs: Vec<f64> = v.iter().filter_map(|&i| master[i].h.duration_s).collect();
            let starts: Vec<f64> = v.iter().filter_map(|&i| master[i].start_time).collect();
            vec![
                Cell::opt_str(g.as_deref()),
                Cell::Int(v.len() as i64),
                Cell::Num(crate::rnum::rsum(durs.iter().copied()) / 3600.0),
                Cell::opt_num(median(durs.clone()).map(|m| m / 60.0)),
                Cell::opt_time(starts.iter().copied().reduce(f64::min)),
                Cell::opt_time(starts.iter().copied().reduce(f64::max)),
                Cell::opt_int(sum_bool(v, &|m| Some(m.gap_before))),
                Cell::opt_int(sum_bool(v, &|m| Some(m.duplicate))),
                Cell::opt_int(sum_bool(v, &|m| not(m.h.readable))),
                Cell::opt_int(sum_bool(v, &|m| Some(m.deployment.is_none()))),
            ]
        })
        .collect();
    write_table(
        &outputs.join("audio_summary_per_grid.csv"),
        &["grid", "n_files", "hours", "median_file_min", "first", "last", "n_gaps", "n_duplicate", "n_unreadable",
            "n_outside_deployment"],
        &grid_rows,
    )
    .map_err(|e| e.to_string())?;

    // ---- the summary line
    let all: Vec<usize> = (0..master.len()).collect();
    let na = |v: Option<i64>| v.map(|n| n.to_string()).unwrap_or_else(|| "NA".into());
    let total_hours: f64 = crate::rnum::rsum(master.iter().filter_map(|m| m.h.duration_s)) / 3600.0;
    let n_grids = groups.len();
    let truncated_n = master.iter().filter(|m| m.truncated == Some(true)).count();
    log.info(&format!(
        "{} files, {} grids, {:.1} h | invalid names {}, unreadable {}, truncated {}, duplicates {}, gaps {}, outside deployment {}",
        master.len(),
        n_grids,
        total_hours,
        master.iter().filter(|m| !m.valid_name).count(),
        na(sum_bool(&all, &|m| not(m.h.readable))),
        truncated_n,
        master.iter().filter(|m| m.duplicate).count(),
        master.iter().filter(|m| m.gap_before).count(),
        master.iter().filter(|m| m.deployment.is_none()).count(),
    ));
    Ok(ScanResult { master })
}

/// The "scan" event: a summary of the last scan, read back from the files it
/// wrote (`emit_scan` in run_job.R).
pub fn scan_event(cfg: &Config) -> serde_json::Value {
    use serde_json::json;
    let paths = &cfg.data["paths"];
    let path_of = |k: &str| paths[k].as_str().unwrap_or("").to_string();
    let audio_folder = path_of("audio_standardized");
    let master_path = PathBuf::from(path_of("metadata")).join("audio_master.csv");
    let Some(m) = Table::read(&master_path).filter(|_| master_path.is_file()) else {
        return json!({ "available": false, "audio_folder": audio_folder });
    };
    let n = m.rows.len();
    // col(name, default): the column, or the default for every row
    let col = |name: &str, default: &str| -> Vec<String> {
        m.column(name).map(|c| c.iter().map(|s| s.to_string()).collect()).unwrap_or_else(|| vec![default.into(); n])
    };
    let is_true = |v: &str| v == "TRUE" || v == "true";
    let count = |name: &str| col(name, "FALSE").iter().filter(|v| is_true(v)).count();
    let not_true = |name: &str| col(name, "TRUE").iter().filter(|v| !is_true(v)).count();
    let grids: std::collections::HashSet<String> = col("grid", "").into_iter().collect();
    let total: f64 = crate::rnum::rsum(
        col("duration_s", "0").iter().filter_map(|v| parse_f(v)).filter(|d| *d < 1e7).map(|d| d.max(0.0)),
    );
    let round1 = |x: f64| format!("{:.1}", x).parse::<f64>().unwrap_or(x);
    let outputs = PathBuf::from(path_of("outputs"));
    let per_grid = match Table::read(&outputs.join("audio_summary_per_grid.csv")) {
        Some(mut t) if !t.rows.is_empty() => {
            for name in ["hours", "median_file_min"] {
                if let Some(i) = t.col(name) {
                    for r in t.rows.iter_mut() {
                        if let Some(x) = parse_f(&r[i]) {
                            r[i] = crate::rnum::fwrite_num(round1(x));
                        }
                    }
                }
            }
            t.json_rows(usize::MAX)
        }
        _ => json!([]),
    };
    let issues = match Table::read(&outputs.join("deployment_issues.csv")) {
        Some(t) => t.json_rows(50),
        None => json!([]),
    };
    let updated = std::fs::metadata(&master_path)
        .and_then(|md| md.modified())
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    json!({
        "available": true,
        "updated": updated,
        "audio_folder": audio_folder,
        "metadata_folder": path_of("metadata"),
        "n_files": n,
        "n_grids": grids.len(),
        "total_hours": round1(total / 3600.0),
        "n_qc_pass": count("qc_pass"),
        "issues": {
            "invalid_name": not_true("valid_name"),
            "unreadable": not_true("readable"),
            "truncated": count("truncated"),
            "short": count("short"),
            "duplicate": count("duplicate"),
            "gaps": count("gap_before"),
            "outside_deployment": not_true("in_deployment"),
        },
        "per_grid": per_grid,
        "deployment_issues": issues,
    })
}
