//! pamflow-engine: run one job and report progress as `@@PAM {json}` lines.
//!
//!   pamflow-engine <job.json>
//!
//! Drop-in for `Rscript bridge/run_job.R <job.json>` (pamflow Desktop 0.2.1):
//! same job file, same events, same exit status (0 = job finished,
//! 1 = job failed, 2 = bad job file). Jobs are moved here from R one by one;
//! a job that is not ported yet fails with a clear message so the app can
//! keep sending it to the R bridge.

use pamflow_core::config::Config;
use pamflow_core::config_edit::Change;
use pamflow_core::config_save;
use pamflow_core::events::emit;
use pamflow_core::project::{find_config, Found};
use pamflow_core::project_jobs;
use serde_json::{json, Map, Value};
use std::process::ExitCode;

/// Jobs the Rust engine runs. Everything else still goes to the R bridge.
const PORTED: &[&str] =
    &["inspect", "config_save", "config_save_as", "project_create", "example_project", "diagnostics", "scan", "scan_results"];

/// Scan the recordings (`job_scan`): stage 01 with progress as log events,
/// then the summary.
fn job_scan(cfg: &Config, job: &Value) -> Outcome {
    use pamflow_core::{scan, stage_log};
    emit("stage", json!({ "stage": "scan", "label": "Scanning recordings" }));
    let logs = cfg.data["paths"]["logs"].as_str().map(std::path::PathBuf::from);
    let sink: stage_log::Sink = Box::new(|level, stage, text| {
        emit("log", json!({ "level": level, "stage": stage, "text": text }));
    });
    let mut log = stage_log::StageLog::new(scan::STAGE, logs, sink);
    let force = job["force"].as_bool().unwrap_or(false);
    let run = stage_log::run_stage(&mut log, |log| scan::scan_audio(cfg, force, log));
    if run.result.is_none() {
        let mut msg = "Scanning the recordings failed; see the log.".to_string();
        if let Some(e) = &run.last_error {
            msg = msg.replace("see the log.", &format!("pamflow said: {e}"));
        }
        return Outcome::failed(msg);
    }
    emit("scan", scan::scan_event(cfg));
    let mut o = Outcome::ok("");
    o.extra.insert("minutes".into(), json!((run.minutes * 100.0).round() / 100.0));
    o
}

/// Write the diagnostics file (works without a project).
fn job_diagnostics(job: &Value) -> ExitCode {
    let found = find_config(job["project"].as_str(), job["config"].as_str());
    let facts = system_facts();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z").to_string();
    let text = project_jobs::diagnostics_text(job, found.as_ref(), &facts, &now);
    match project_jobs::write_diagnostics(job["target"].as_str().unwrap_or(""), &text) {
        Ok((path, bytes)) => {
            emit("diagnostics_saved", json!({ "path": path, "bytes": bytes }));
            finish(Outcome::ok(""))
        }
        Err(e) => finish(Outcome::failed(e)),
    }
}

fn system_facts() -> project_jobs::SystemFacts {
    let mem = sysinfo::System::new()
        .ok()
        .map(|mut sys| {
            sys.refresh_memory();
            sys.total_memory()
        })
        .unwrap_or(0);
    let os = sysinfo::System::long_os_version().unwrap_or_else(|_| std::env::consts::OS.to_string());
    project_jobs::SystemFacts {
        os: format!("{os} {}", std::env::consts::ARCH),
        memory_gb: (mem > 0).then(|| mem as f64 / 1024f64.powi(3)),
        time_zone: iana_time_zone::get_timezone().unwrap_or_else(|_| "unknown".into()),
    }
}

/// Report the project (`job_inspect`): the "project" event.
fn job_inspect(cfg: &Config, found: &Found) -> Outcome {
    emit("project", pamflow_core::inspect::project_event(cfg, found));
    Outcome::ok("")
}

/// After a save, read the saved file and report the project again, as
/// run_job.R does (with the job's run-only overrides).
fn inspect_after_save(job: &Value, found: &Found) -> Outcome {
    match Config::read(&found.config) {
        Ok(mut cfg) => {
            apply_overrides(&mut cfg, job);
            job_inspect(&cfg, found)
        }
        Err(e) => Outcome::failed(e.to_string()),
    }
}

/// Settings the app changes for this run only (`job.birdnet`, `job.filter`).
fn apply_overrides(cfg: &mut Config, job: &Value) {
    for section in ["birdnet", "filter"] {
        cfg.override_section(section, &job[section]);
    }
}

/// What a job ends with: the fields of the "finished" event.
struct Outcome {
    status: &'static str,
    message: String,
    extra: Map<String, Value>,
}

impl Outcome {
    fn ok(message: impl Into<String>) -> Self {
        Self { status: "ok", message: message.into(), extra: Map::new() }
    }
    fn failed(message: impl Into<String>) -> Self {
        Self { status: "failed", message: message.into(), extra: Map::new() }
    }
}

fn finish(o: Outcome) -> ExitCode {
    let mut fields = Map::new();
    fields.insert("status".into(), json!(o.status));
    // R's successful jobs end with no message field
    if !(o.status == "ok" && o.message.is_empty()) {
        fields.insert("message".into(), json!(o.message));
    }
    fields.extend(o.extra);
    emit("finished", Value::Object(fields));
    ExitCode::from(if o.status == "ok" { 0 } else { 1 })
}

fn changes_of(job: &Value) -> Result<Vec<Change>, String> {
    match &job["changes"] {
        Value::Null => Ok(Vec::new()),
        v => serde_json::from_value(v.clone()).map_err(|e| format!("bad changes in the job file: {e}")),
    }
}

fn job_config_save(job: &Value, found: &Found) -> Outcome {
    let changes = match changes_of(job) {
        Ok(c) => c,
        Err(e) => return Outcome::failed(e),
    };
    match config_save::save(found, &changes, &config_save::stamp_now()) {
        Ok(None) => Outcome::ok("Nothing to save."),
        Ok(Some(saved)) => {
            emit("config_saved", json!({ "n": changes.len(), "backup": saved.backup }));
            inspect_after_save(job, found)
        }
        Err(e) => Outcome::failed(e),
    }
}

fn job_config_save_as(job: &Value, found: &Found) -> Outcome {
    let changes = match changes_of(job) {
        Ok(c) => c,
        Err(e) => return Outcome::failed(e),
    };
    let target = job["target"].as_str().unwrap_or("");
    match config_save::save_as(found, target, &changes, &config_save::stamp_now()) {
        Ok((saved, now)) => {
            let config = pamflow_core::config_edit::norm_path(&now.config);
            if config.to_lowercase() == pamflow_core::config_edit::norm_path(&found.config).to_lowercase() {
                emit("config_saved", json!({ "n": changes.len(), "backup": saved.backup }));
            } else {
                emit("config_saved", json!({ "n": changes.len(), "backup": saved.backup, "config": config }));
            }
            inspect_after_save(job, &now)
        }
        Err(e) => Outcome::failed(e),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut job: Value = match args.as_slice() {
        [path] => match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| {
            serde_json::from_str(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
        }) {
            Ok(v) => v,
            Err(e) => {
                finish(Outcome::failed(format!("could not read the job file: {e}")));
                return ExitCode::from(2);
            }
        },
        _ => {
            finish(Outcome::failed("usage: pamflow-engine <job.json>"));
            return ExitCode::from(2);
        }
    };
    let mut name = job["job"].as_str().unwrap_or("").to_string();
    if !PORTED.contains(&name.as_str()) {
        return finish(Outcome::failed(format!("The job \"{name}\" is not available in the Rust engine yet.")));
    }

    // jobs that run before a project is looked up, as in run_job.R
    match name.as_str() {
        "diagnostics" => return job_diagnostics(&job),
        "example_project" => {
            let base = job["documents_dir"]
                .as_str()
                .filter(|d| !d.is_empty())
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(Into::into))
                .unwrap_or_else(|| ".".into());
            let dir = project_jobs::example_folder(&base);
            match project_jobs::example_project(&dir) {
                Ok(made) => {
                    let config = pamflow_core::config_edit::norm_path(&made.config);
                    emit("example_created", json!({ "root": made.root, "config": config }));
                    job["project"] = json!(config);
                }
                Err(e) => return finish(Outcome::failed(format!("Could not make the example project: {e}"))),
            }
            job["config"] = Value::Null;
            name = "inspect".into();
        }
        "project_create" => {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            match project_jobs::create_project(job["project"].as_str().unwrap_or(""), job["target"].as_str(), &today) {
                Ok(made) => {
                    let config = pamflow_core::config_edit::norm_path(&made.config);
                    emit("project_created", json!({ "config": config, "root": made.root }));
                    job["project"] = json!(config);
                }
                Err(e) => return finish(Outcome::failed(e)),
            }
            job["config"] = Value::Null;
            name = "inspect".into(); // then report the new project like any other
        }
        _ => {}
    }

    let project = job["project"].as_str();
    let Some(found) = find_config(project, job["config"].as_str()) else {
        if name == "inspect" {
            emit("no_config", pamflow_core::inspect::no_config_event(project, job["config"].as_str()));
        }
        return finish(Outcome::failed(format!(
            "No pamflow settings file found in {} (looked for config/config.yml and config.yml).",
            project.unwrap_or("")
        )));
    };
    // like run_job.R: the current settings must be readable before any job runs
    let mut cfg = match Config::read(&found.config) {
        Ok(c) => c,
        Err(e) => return finish(Outcome::failed(e.to_string())),
    };
    apply_overrides(&mut cfg, &job);
    emit(
        "started",
        json!({
            "job": name, "project": found.root, "engine": "rust",
            "pamflow": format!("rust {}", env!("CARGO_PKG_VERSION")), "pid": std::process::id(),
        }),
    );
    let outcome = match name.as_str() {
        "inspect" => job_inspect(&cfg, &found),
        "scan" => job_scan(&cfg, &job),
        "scan_results" => {
            emit("scan", pamflow_core::scan::scan_event(&cfg));
            Outcome::ok("")
        }
        "config_save" => job_config_save(&job, &found),
        "config_save_as" => job_config_save_as(&job, &found),
        other => Outcome::failed(format!("unknown job: {other}")),
    };
    finish(outcome)
}
