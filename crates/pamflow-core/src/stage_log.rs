//! pamflow's stage log (`pam_log`, `pam_run_stage`): each message is a line
//! in `<logs>/<stage>.log` and a "log" event for the app.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

/// Where a log line goes besides the file: (level, stage, text).
pub type Sink<'a> = Box<dyn FnMut(&str, &str, &str) + 'a>;

pub struct StageLog<'a> {
    pub stage: String,
    log_dir: Option<PathBuf>,
    sink: Sink<'a>,
    /// R warnings raised during the stage (they are counted at the end).
    pub warnings: Vec<String>,
    /// The last ERROR message, for the job's failure message.
    pub last_error: Option<String>,
}

impl<'a> StageLog<'a> {
    pub fn new(stage: &str, log_dir: Option<PathBuf>, sink: Sink<'a>) -> Self {
        Self { stage: stage.into(), log_dir, sink, warnings: Vec::new(), last_error: None }
    }

    /// `pam_log(level, msg, stage, log_dir)`.
    pub fn log(&mut self, level: &str, msg: &str) {
        let line = format!("{} [{level}] [{}] {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), self.stage);
        if let Some(dir) = &self.log_dir {
            let _ = std::fs::create_dir_all(dir);
            if let Ok(mut f) =
                std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.log", self.stage)))
            {
                // R's cat() to a text-mode file: CRLF on Windows
                let eol = if cfg!(windows) { "\r\n" } else { "\n" };
                let _ = f.write_all(format!("{}{eol}", line.replace('\n', eol)).as_bytes());
            }
        }
        // the bridge turns each message line into an event; continuation
        // lines (without the prefix) become plain INFO lines
        let mut lines = msg.split('\n');
        let first = lines.next().unwrap_or("");
        (self.sink)(level, &self.stage, first);
        for l in lines.filter(|l| !l.trim().is_empty()) {
            (self.sink)("INFO", "", l);
        }
        if level == "ERROR" {
            self.last_error = Some(first.to_string());
        }
    }

    pub fn info(&mut self, msg: &str) {
        self.log("INFO", msg);
    }
    /// A WARN line that is not an R warning (not counted).
    pub fn warn(&mut self, msg: &str) {
        self.log("WARN", msg);
    }
    /// An R warning: logged as WARN and counted.
    pub fn warning(&mut self, msg: &str) {
        self.warnings.push(msg.to_string());
        self.log("WARN", msg);
    }
}

/// How a stage ended (`pam_run_stage`).
pub struct StageRun<T> {
    pub result: Option<T>,
    pub minutes: f64,
    pub warnings: usize,
    pub last_error: Option<String>,
}

/// `pam_run_stage`: "stage started", the work, an ERROR line if it fails,
/// then "stage finished|FAILED in X.XX min (N warnings)".
pub fn run_stage<T>(log: &mut StageLog, f: impl FnOnce(&mut StageLog) -> Result<T, String>) -> StageRun<T> {
    let t0 = Instant::now();
    log.info("stage started");
    let out = f(log);
    let result = match out {
        Ok(v) => Some(v),
        Err(e) => {
            log.log("ERROR", &e);
            None
        }
    };
    let minutes = t0.elapsed().as_secs_f64() / 60.0;
    let n = log.warnings.len();
    log.info(&format!(
        "stage {} in {:.2} min ({} warnings)",
        if result.is_some() { "finished" } else { "FAILED" },
        minutes,
        n
    ));
    StageRun { result, minutes, warnings: n, last_error: log.last_error.clone() }
}
