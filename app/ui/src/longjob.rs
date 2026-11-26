//! Long-running jobs (Standardize, later Filter, Detect, Soundscape): start,
//! follow, stop (port of `backend/longjob.svelte.ts` and `LongJobPanel.svelte`).
//!
//! The state lives in the store, not in a screen, so leaving a screen and
//! coming back keeps a running job's progress. Stop ends the engine and
//! everything it started; the engine's checkpoints let the next start
//! continue where it stopped.

use crate::components::{Badge, Button, Card, StatTile};
use crate::i18n::{t, tv};
use crate::tauri;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;

const MAX_ROWS: usize = 300;
const MAX_LOG: usize = 500;

/// A finished file in the activity list.
#[derive(Clone, Debug)]
pub struct FileRow {
    pub rel: String,
    pub ok: bool,
    pub message: String,
    pub time: String,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: String,
    pub level: String,
    pub text: String,
}

pub fn is_active(status: &str) -> bool {
    matches!(status, "starting" | "scanning" | "running" | "stopping")
}

/// The state of one long-running job.
#[derive(Clone, Copy)]
pub struct LongJob {
    pub id: RwSignal<String>,
    /// idle | starting | scanning | running | stopping | stopped | finished | failed
    pub status: RwSignal<&'static str>,
    pub total: RwSignal<Option<i64>>,
    pub already_done: RwSignal<i64>,
    pub done: RwSignal<i64>,
    pub failed: RwSignal<i64>,
    pub run_dir: RwSignal<String>,
    pub started_at: RwSignal<f64>,
    pub message: RwSignal<String>,
    pub rows: RwSignal<Vec<FileRow>>,
    pub log: RwSignal<Vec<LogLine>>,
    pub prep_done: RwSignal<i64>,
    pub prep_total: RwSignal<i64>,
}

impl LongJob {
    pub fn new() -> Self {
        LongJob {
            id: RwSignal::new(String::new()),
            status: RwSignal::new("idle"),
            total: RwSignal::new(None),
            already_done: RwSignal::new(0),
            done: RwSignal::new(0),
            failed: RwSignal::new(0),
            run_dir: RwSignal::new(String::new()),
            started_at: RwSignal::new(0.0),
            message: RwSignal::new(String::new()),
            rows: RwSignal::new(Vec::new()),
            log: RwSignal::new(Vec::new()),
            prep_done: RwSignal::new(0),
            prep_total: RwSignal::new(0),
        }
    }

    fn add_log(&self, level: &str, text: &str) {
        self.log.update(|l| {
            l.push(LogLine { time: tauri::clock(), level: level.into(), text: text.into() });
            if l.len() > MAX_LOG {
                let cut = l.len() - MAX_LOG;
                l.drain(..cut);
            }
        });
    }

    /// One event of this job.
    fn handle(&self, e: &Value) {
        let num = |k: &str| e[k].as_i64().or_else(|| e[k].as_f64().map(|f| f as i64)).unwrap_or(0);
        match e["type"].as_str().unwrap_or("") {
            "stage" => {
                let stage = e["stage"].as_str().unwrap_or("");
                if matches!(stage, "scan" | "std_inspect") {
                    self.status.set("scanning");
                }
                if matches!(stage, "detect" | "filter" | "standardize" | "soundscape") {
                    self.status.set("running");
                    self.run_dir.set(e["run_dir"].as_str().unwrap_or("").to_string());
                    self.already_done.set(num("already_done"));
                    self.started_at.set(tauri::now_ms());
                }
            }
            "prep" => {
                self.prep_done.set(num("done"));
                self.prep_total.set(num("total"));
            }
            "total" => self.total.set(Some(num("n"))),
            "files" => {
                self.done.set(num("done"));
                self.failed.set(num("failed"));
                if let Some(rows) = e["rows"].as_array() {
                    let t0 = tauri::clock();
                    let new: Vec<FileRow> = rows
                        .iter()
                        .map(|r| FileRow {
                            rel: r["rel"].as_str().unwrap_or("").to_string(),
                            ok: r["ok"].as_bool().unwrap_or(true),
                            message: r["message"].as_str().unwrap_or("").to_string(),
                            time: t0.clone(),
                        })
                        .collect();
                    self.rows.update(|list| {
                        // newest first
                        let mut next: Vec<FileRow> = new.into_iter().rev().collect();
                        next.append(list);
                        next.truncate(MAX_ROWS);
                        *list = next;
                    });
                }
            }
            "log" => self.add_log(e["level"].as_str().unwrap_or("INFO"), e["text"].as_str().unwrap_or("")),
            "text" => self.add_log("OUT", e["text"].as_str().unwrap_or("")),
            "stopping" => self.status.set("stopping"),
            "finished" => {
                self.status.set(if e["status"] == "ok" { "finished" } else { "failed" });
                self.message.set(e["message"].as_str().unwrap_or("").to_string());
            }
            "exit" => {
                if e["stopped"].as_bool().unwrap_or(false) {
                    self.status.set("stopped");
                } else if is_active(self.status.get_untracked()) {
                    self.status.set("failed");
                    if self.message.get_untracked().is_empty() {
                        let code = e["code"].as_i64().map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
                        self.message.set(format!("The analysis engine stopped unexpectedly (exit code {code})."));
                    }
                }
            }
            _ => {}
        }
    }

    /// Follow this job's events (call once). `on_event` also sees every event,
    /// e.g. to take a summary the job reports when it ends.
    pub fn listen(&self, on_event: impl Fn(&Value) + 'static) {
        let job = *self;
        spawn_local(async move {
            let _ = tauri::subscribe(move |id, ev| {
                if !id.is_empty() && id == job.id.get_untracked() {
                    job.handle(ev);
                    on_event(ev);
                }
            })
            .await;
        });
    }

    /// Start the job.
    pub fn start(&self, spec: Value) {
        let job = *self;
        spawn_local(async move {
            job.status.set("starting");
            job.total.set(None);
            job.already_done.set(0);
            job.done.set(0);
            job.failed.set(0);
            job.run_dir.set(String::new());
            job.started_at.set(0.0);
            job.message.set(String::new());
            job.rows.set(Vec::new());
            job.log.set(Vec::new());
            job.prep_done.set(0);
            job.prep_total.set(0);
            match tauri::start_job(spec).await {
                Ok(id) => job.id.set(id),
                Err(e) => {
                    job.status.set("failed");
                    job.message.set(e);
                }
            }
        });
    }

    /// Stop the job.
    pub fn stop(&self) {
        let job = *self;
        let id = job.id.get_untracked();
        if id.is_empty() || !is_active(job.status.get_untracked()) {
            return;
        }
        job.status.set("stopping");
        spawn_local(async move {
            if let Err(e) = tauri::stop_job(&id).await {
                job.message.set(e);
            }
        });
    }

    /// Minutes left, from the average time per finished file so far.
    pub fn minutes_left(&self) -> Option<i64> {
        let finished = self.done.get() + self.failed.get();
        let total = self.total.get()?;
        let started = self.started_at.get();
        if started == 0.0 || finished <= 0 {
            return None;
        }
        let per_file = (tauri::now_ms() - started) / finished as f64;
        Some((((total - finished) as f64 * per_file) / 60000.0).round().max(0.0) as i64)
    }

    pub fn minutes_elapsed(&self) -> i64 {
        let started = self.started_at.get();
        if started == 0.0 {
            0
        } else {
            ((tauri::now_ms() - started) / 60000.0).round() as i64
        }
    }
}

impl Default for LongJob {
    fn default() -> Self {
        Self::new()
    }
}

// ---- the panel ------------------------------------------------------------------------------------

fn locale(n: i64) -> String {
    crate::project::thousands(n)
}

/// Live view of a long-running job: progress with Stop, totals, and the
/// list of finished files (or the full log). `kind` picks the row labels.
#[component]
pub fn LongJobPanel(job: LongJob, kind: &'static str) -> impl IntoView {
    // refreshes the elapsed / time-left text
    let tick = RwSignal::new(0u32);
    let handle = set_interval_with_handle(move || tick.update(|t| *t += 1), std::time::Duration::from_secs(5)).ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });
    let show_log = RwSignal::new(false);
    let active = move || is_active(job.status.get());
    let finished_files = move || job.done.get() + job.failed.get();
    let preparing = move || job.status.get() == "scanning" && job.prep_total.get() > 0;
    let percent = move || {
        if preparing() {
            job.prep_done.get() as f64 / job.prep_total.get() as f64 * 100.0
        } else {
            match job.total.get() {
                Some(t) if t > 0 => finished_files() as f64 / t as f64 * 100.0,
                _ => 0.0,
            }
        }
    };
    let status_text = move || match job.status.get() {
        "starting" => t("detect.starting"),
        "scanning" => if kind == "standardize" { t("standardize.reading") } else { t("detect.scanning") },
        "running" => t("detect.running"),
        "stopping" => t("detect.stopping"),
        "stopped" => t("detect.stopped"),
        "finished" => t("detect.finished"),
        "failed" => t("detect.failedStatus"),
        _ => String::new(),
    };
    let left = move || {
        tick.get();
        job.minutes_left()
    };
    let elapsed = move || {
        tick.get();
        job.minutes_elapsed()
    };

    let hero = view! {
        <section class="hero">
            <div class="top">
                <div class="count">
                    <span class="status">{status_text}</span>
                    {move || {
                        if preparing() {
                            view! { <span class="big">{locale(job.prep_done.get())}" "<small>{tv("standardize.ofRead", &[("n", locale(job.prep_total.get()))])}</small></span> }.into_any()
                        } else if let Some(total) = job.total.get() {
                            view! { <span class="big">{finished_files()}" "<small>{tv("detect.ofFiles", &[("n", locale(total))])}</small></span> }.into_any()
                        } else {
                            view! { <span class="big"><small>{status_text()}</small></span> }.into_any()
                        }
                    }}
                </div>
                <div class="buttons">
                    {move || active().then(|| view! {
                        <Button variant="accent" disabled=Signal::derive(move || job.status.get() == "stopping") on_click=move |_| job.stop()>
                            "⏸ "{t("detect.stop")}
                        </Button>
                    })}
                    {move || {
                        let dir = job.run_dir.get();
                        (!dir.is_empty()).then(|| view! {
                            <Button variant="accent-outline" on_click=move |_| { let d = dir.clone(); spawn_local(async move { tauri::show_folder(&d).await }) }>
                                {t("detect.openFolder")}
                            </Button>
                        })
                    }}
                </div>
            </div>
            <div class="bar hero-bar" role="progressbar">
                <div class="fill" class:dim=move || !active() style=move || format!("width: {}%", percent().clamp(0.0, 100.0))></div>
            </div>
            <div class="meta">
                {move || (active() && left().is_some()).then(|| view! { <span>{tv("detect.left", &[("n", left().unwrap_or(0).to_string())])}</span> })}
                {move || (job.started_at.get() > 0.0).then(|| view! { <span>{tv("detect.elapsed", &[("n", elapsed().to_string())])}</span> })}
                {move || (job.already_done.get() > 0).then(|| view! { <span>{tv("detect.alreadyDone", &[("n", job.already_done.get().to_string())])}</span> })}
            </div>
            {move || { let m = job.message.get(); (!m.is_empty()).then(|| view! { <p class="problem">{m}</p> }) }}
        </section>
    };

    let row_badge = move |row: &FileRow| {
        if !row.ok {
            let extra = if row.message.is_empty() { String::new() } else { format!(" · {}", row.message) };
            view! { <Badge round=true palette="peach">{format!("{}{extra}", t("detect.failedFile"))}</Badge> }.into_any()
        } else if kind == "standardize" {
            let key = if row.message == "converted" { "converted" } else { "copied" };
            view! { <Badge round=true palette="mint">{t(&format!("standardize.rowDone.{key}"))}</Badge> }.into_any()
        } else {
            view! { <Badge round=true palette="mint">{t("filter.fileDone")}</Badge> }.into_any()
        }
    };

    let activity = move || {
        if show_log.get() {
            let lines: Vec<LogLine> = job.log.get().into_iter().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
            view! {
                <ul class="log mono">
                    {lines
                        .into_iter()
                        .map(|l| {
                            let class = match l.level.as_str() { "WARN" => "warn", "ERROR" => "error", _ => "" };
                            view! { <li class=class><span class="time">{l.time}</span><span class="text">{l.text}</span></li> }
                        })
                        .collect_view()}
                </ul>
            }
            .into_any()
        } else if !job.rows.get().is_empty() {
            view! {
                <ul class="rows">
                    {job.rows
                        .get()
                        .into_iter()
                        .map(|r| {
                            let badge = row_badge(&r);
                            view! { <li><span class="time">{r.time.clone()}</span><span class="file mono">{r.rel.clone()}</span>{badge}</li> }
                        })
                        .collect_view()}
                </ul>
            }
            .into_any()
        } else {
            view! { <p class="empty">{t("detect.noActivity")}</p> }.into_any()
        }
    };

    view! {
        {hero}
        <div class="grid-2">
            <StatTile plain=true value=move || job.done.get().to_string() label=t("detect.filesDone") />
            <StatTile plain=true value=move || job.failed.get().to_string() label=t("detect.filesFailed") />
        </div>
        <Card title=t("detect.activity")>
            <div><Button size="sm" on_click=move |_| show_log.update(|v| *v = !*v)>
                {move || if show_log.get() { t("detect.hideLog") } else { t("detect.showLog") }}
            </Button></div>
            {activity}
        </Card>
    }
}
