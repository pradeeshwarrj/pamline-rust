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
use serde_json::{json, Map, Value};
use std::process::ExitCode;

/// Jobs the Rust engine runs. Everything else still goes to the R bridge.
///
/// Not yet complete: after saving, run_job.R reports the project again
/// (`inspect`); until `inspect` is ported the app must keep sending these
/// jobs to R.
const PORTED: &[&str] = &["config_save", "config_save_as"];

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
    fields.insert("message".into(), json!(o.message));
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
            Outcome::ok("")
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
            Outcome::ok("")
        }
        Err(e) => Outcome::failed(e),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let job: Value = match args.as_slice() {
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
    let name = job["job"].as_str().unwrap_or("").to_string();
    if !PORTED.contains(&name.as_str()) {
        return finish(Outcome::failed(format!("The job \"{name}\" is not available in the Rust engine yet.")));
    }

    let project = job["project"].as_str();
    let Some(found) = find_config(project, job["config"].as_str()) else {
        return finish(Outcome::failed(format!(
            "No pamflow settings file found in {} (looked for config/config.yml and config.yml).",
            project.unwrap_or("")
        )));
    };
    // like run_job.R: the current settings must be readable before any job runs
    if let Err(e) = Config::read(&found.config) {
        return finish(Outcome::failed(e.to_string()));
    }
    emit(
        "started",
        json!({
            "job": name, "project": found.root, "engine": "rust",
            "pamflow": format!("rust {}", env!("CARGO_PKG_VERSION")), "pid": std::process::id(),
        }),
    );
    let outcome = match name.as_str() {
        "config_save" => job_config_save(&job, &found),
        "config_save_as" => job_config_save_as(&job, &found),
        other => Outcome::failed(format!("unknown job: {other}")),
    };
    finish(outcome)
}
