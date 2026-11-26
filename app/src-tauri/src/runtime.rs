//! Finding the bundled runtime (R + pamflow + Python/BirdNET + ffmpeg) and
//! preparing commands that run inside it, sealed off from any R or Python
//! the user may have installed themselves.
//!
//! Where the runtime is looked for, in order:
//!   1. the PAMFLOW_RUNTIME environment variable (a runtime folder)
//!   2. the installed app's runtime, unpacked on first launch (see setup.rs)
//!   3. while developing: the runtime of the pamflow-desktop folder next to
//!      this repository (<..>/pamflow-desktop/runtime/.pixi/envs/default)

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
pub struct Runtime {
    /// Runtime folder (the conda/pixi environment).
    pub prefix: PathBuf,
    /// Folder holding the R bridge scripts (run_job.R, protocol.R, ...).
    pub bridge: PathBuf,
}

impl Runtime {
    pub fn locate(app: &AppHandle) -> Result<Runtime, String> {
        let mut tried: Vec<PathBuf> = Vec::new();

        // Bridge scripts: next to the app when installed, in the project when developing.
        let bridge_candidates = [
            app.path().resource_dir().ok().map(|d| d.join("bridge")),
            std::env::var("PAMFLOW_BRIDGE").ok().map(PathBuf::from),
            desktop_root().map(|d| d.join("bridge")),
        ];
        let bridge = bridge_candidates
            .into_iter()
            .flatten()
            .find(|d| d.join("run_job.R").is_file())
            .ok_or_else(|| "The app's R bridge scripts (bridge/run_job.R) were not found.".to_string())?;

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(p) = std::env::var("PAMFLOW_RUNTIME") {
            candidates.push(PathBuf::from(p));
        }
        if let Some(p) = crate::setup::installed_prefix(app) {
            candidates.push(p);
        }
        if let Some(root) = desktop_root() {
            candidates.push(root.join("runtime").join(".pixi").join("envs").join("default"));
        }

        // R cannot read Windows "\\?\C:\..." long-path names; use plain C:\... names.
        let bridge = plain(bridge);
        for prefix in candidates.into_iter().map(plain) {
            if rscript_in(&prefix).is_file() {
                return Ok(Runtime { prefix, bridge });
            }
            tried.push(prefix);
        }
        Err(format!(
            "The analysis engine (runtime) was not found. Looked in:\n{}\n\nWhile developing, build it with `pixi install` and `pixi run setup-r` in the runtime folder.",
            tried.iter().map(|p| format!("  {}", p.display())).collect::<Vec<_>>().join("\n")
        ))
    }

    pub fn rscript(&self) -> PathBuf {
        rscript_in(&self.prefix)
    }

    /// A command that runs `Rscript <args>` inside the runtime.
    pub fn rscript_command(&self, args: &[&std::ffi::OsStr]) -> Command {
        let mut cmd = Command::new(self.rscript());
        cmd.args(args);
        self.apply_env(&mut cmd);
        cmd
    }

    /// A command that runs the Rust engine (`pamflow-engine <job.json>`).
    /// With the runtime at hand it gets the runtime's PATH (ffmpeg for the
    /// diagnostics file); without one it still runs: the engine needs no R or Python.
    pub fn engine_command(exe: &Path, runtime: Option<&Runtime>, job_file: &std::ffi::OsStr) -> Command {
        let mut cmd = Command::new(exe);
        cmd.arg(job_file);
        match runtime {
            Some(rt) => rt.apply_env(&mut cmd),
            None => platform_flags(&mut cmd),
        }
        cmd
    }

    /// The same environment `pixi run` sets up, plus isolation from the
    /// user's own R and Python settings.
    fn apply_env(&self, cmd: &mut Command) {
        let p = &self.prefix;
        let mut path_dirs: Vec<PathBuf> = if cfg!(windows) {
            vec![
                p.clone(),
                p.join("Library").join("mingw-w64").join("bin"),
                p.join("Library").join("usr").join("bin"),
                p.join("Library").join("bin"),
                p.join("Scripts"),
                p.join("bin"),
                p.join("lib").join("R").join("bin").join("x64"),
            ]
        } else {
            vec![p.join("bin")]
        };
        if let Some(existing) = std::env::var_os("PATH") {
            path_dirs.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(path_dirs) {
            cmd.env("PATH", joined);
        }
        cmd.env("CONDA_PREFIX", p);

        // Ignore the user's own R installation and settings.
        for var in ["R_HOME", "R_LIBS_SITE", "PYTHONHOME", "PYTHONPATH", "RETICULATE_PYTHON"] {
            cmd.env_remove(var);
        }
        cmd.env("R_LIBS_USER", "NULL")
            .env("R_LIBS", "")
            .env("R_PROFILE_USER", "NULL")
            .env("R_ENVIRON_USER", "NULL");

        // Certificates for any HTTPS the runtime does (e.g. model downloads).
        let cert = if cfg!(windows) {
            p.join("Library").join("ssl").join("cacert.pem")
        } else {
            p.join("ssl").join("cacert.pem")
        };
        if cert.is_file() {
            cmd.env("SSL_CERT_FILE", cert);
        }

        platform_flags(cmd);
    }
}

/// No console window on Windows; own process group elsewhere, so Stop can end
/// a job and everything it started.
fn platform_flags(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW); // no black console window
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

/// The Rust engine (`pamflow-engine`), if there is one. Looked for in order:
///   1. the PAMFLOW_ENGINE environment variable (the program; "off" turns the
///      Rust engine off, so every job goes to R)
///   2. the installed app: `engine/pamflow-engine` next to the bridge scripts
///   3. while developing: the sibling `pamline-rust` folder's build
///      (`target/release`, then `target/debug`)
pub fn locate_engine(app: &AppHandle) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "pamflow-engine.exe" } else { "pamflow-engine" };
    if let Ok(p) = std::env::var("PAMFLOW_ENGINE") {
        if p.eq_ignore_ascii_case("off") {
            return None;
        }
        let p = PathBuf::from(p);
        return p.is_file().then(|| plain(p));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("engine").join(exe));
    }
    if let Some(root) = repo_root() {
        for profile in ["release", "debug"] {
            candidates.push(root.join("target").join(profile).join(exe));
        }
    }
    candidates.into_iter().find(|p| p.is_file()).map(plain)
}

/// A path without Windows' "\\?\" prefix (no change elsewhere).
pub fn plain(p: PathBuf) -> PathBuf {
    dunce::simplified(&p).to_path_buf()
}

fn rscript_in(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.join("lib").join("R").join("bin").join("Rscript.exe")
    } else {
        prefix.join("bin").join("Rscript")
    }
}

/// This repository's folder while developing (`cargo tauri dev`); None in release builds.
fn repo_root() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        // app/src-tauri -> app -> the repository
        Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// The pamflow-desktop folder next to this repository while developing: it still
/// holds the R bridge scripts and runtime that the jobs not yet ported need.
fn desktop_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PAMFLOW_DESKTOP") {
        return Some(PathBuf::from(p));
    }
    repo_root()?.parent().map(|p| p.join("pamflow-desktop")).filter(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    /// The engine built next to this repository (skipped when there is none).
    fn dev_engine() -> Option<PathBuf> {
        let exe = if cfg!(windows) { "pamflow-engine.exe" } else { "pamflow-engine" };
        if let Ok(p) = std::env::var("PAMFLOW_ENGINE") {
            return Some(PathBuf::from(p)).filter(|p| p.is_file());
        }
        let root = repo_root()?.join("target");
        ["release", "debug"].iter().map(|p| root.join(p).join(exe)).find(|p| p.is_file())
    }

    /// Run one job through `engine_command` and return the `@@PAM` events.
    fn run(exe: &Path, dir: &Path, name: &str, job: Value) -> Vec<Value> {
        let file = dir.join(format!("{name}.json"));
        std::fs::write(&file, job.to_string()).unwrap();
        let mut cmd = Runtime::engine_command(exe, None, file.as_os_str());
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).spawn().unwrap();
        let events: Vec<Value> = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| l.strip_prefix("@@PAM ").and_then(|j| serde_json::from_str(j).ok()))
            .collect();
        assert!(child.wait().unwrap().success(), "{name} failed: {events:?}");
        events
    }

    type Value = serde_json::Value;

    /// The app's way of running the Rust engine: example project, report, scan.
    #[test]
    fn engine_runs_through_engine_command() {
        let Some(exe) = dev_engine() else {
            eprintln!("skip: no pamflow-engine build found (build ../pamline-rust, or set PAMFLOW_ENGINE)");
            return;
        };
        let dir = std::env::temp_dir().join("pamflow-desktop-engine-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ev = run(&exe, &dir, "example", serde_json::json!({ "job": "example_project", "documents_dir": dir }));
        let types: Vec<&str> = ev.iter().filter_map(|e| e["type"].as_str()).collect();
        assert_eq!(types, ["example_created", "started", "project", "finished"]);
        let config = ev[0]["config"].as_str().unwrap().to_string();
        assert_eq!(ev[2]["n_audio"], 6);

        let ev = run(&exe, &dir, "scan", serde_json::json!({ "job": "scan", "project": config }));
        let scan = ev.iter().find(|e| e["type"] == "scan").expect("a scan event");
        assert_eq!(scan["n_files"], 6);
        assert_eq!(ev.last().unwrap()["status"], "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
