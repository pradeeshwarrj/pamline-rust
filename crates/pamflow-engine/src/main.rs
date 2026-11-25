//! pamflow-engine: run one job and report progress as `@@PAM {json}` lines.
//!
//!   pamflow-engine <job.json>
//!
//! Drop-in for `Rscript bridge/run_job.R <job.json>` (pamflow Desktop 0.2.1):
//! same job file, same events, same exit status (0 = job finished,
//! 1 = job failed, 2 = bad job file). Jobs are moved here from R one by one;
//! a job that is not ported yet fails with a clear message so the app can
//! keep sending it to the R bridge.

use pamflow_core::events::emit;
use pamflow_core::project::find_config;
use serde_json::{json, Value};
use std::process::ExitCode;

/// Jobs the Rust engine runs. Everything else still goes to the R bridge.
const PORTED: &[&str] = &[];

fn finished(status: &str, message: &str) {
    emit("finished", json!({ "status": status, "message": message }));
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let job: Value = match args.as_slice() {
        [path] => match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| {
            serde_json::from_str(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
        }) {
            Ok(v) => v,
            Err(e) => {
                finished("failed", &format!("could not read the job file: {e}"));
                return ExitCode::from(2);
            }
        },
        _ => {
            finished("failed", "usage: pamflow-engine <job.json>");
            return ExitCode::from(2);
        }
    };
    let name = job["job"].as_str().unwrap_or("").to_string();
    if !PORTED.contains(&name.as_str()) {
        finished("failed", &format!("The job \"{name}\" is not available in the Rust engine yet."));
        return ExitCode::from(1);
    }
    emit("started", json!({ "job": name, "engine": "rust", "version": env!("CARGO_PKG_VERSION") }));
    let _found = find_config(job["project"].as_str(), job["config"].as_str());
    finished("failed", &format!("The job \"{name}\" has no handler."));
    ExitCode::from(1)
}
