//! The open project and its settings draft (port of `backend/project.svelte.ts`
//! and `backend/settings.svelte.ts`). Everything the screens show comes from
//! what the engine reports; nothing is computed here.

use crate::tauri::{failure_message, run_job, set_job_config};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{json, Value};
use std::collections::HashMap;

const STORAGE_KEY: &str = "pamflow-project-folder";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn saved(key: &str) -> String {
    storage().and_then(|s| s.get_item(key).ok().flatten()).unwrap_or_default()
}

fn remember(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

/// All shared state, handed to the screens through context.
#[derive(Clone, Copy)]
pub struct Store {
    /// The screen shown (a step id from the sidebar).
    pub screen: RwSignal<&'static str>,
    /// Scan step: idle | loading | scanning | ready | failed.
    pub scan_status: RwSignal<&'static str>,
    /// The engine's "scan" event: what the last scan found.
    pub scan_summary: RwSignal<Option<Value>>,
    pub scan_message: RwSignal<String>,
    pub scan_log: RwSignal<Vec<String>>,
    /// What the user opened: a project folder (or a settings file, until it is read).
    pub folder: RwSignal<String>,
    /// The engine's "project" event for the open project.
    pub facts: RwSignal<Option<Value>>,
    pub checking: RwSignal<bool>,
    pub error: RwSignal<String>,
    /// A folder with no settings file: the engine's "no_config" event.
    pub no_config: RwSignal<Option<Value>>,
    /// The settings file was just made from the defaults.
    pub created: RwSignal<bool>,
    /// The open project is pamflow's example project, just made.
    pub example: RwSignal<bool>,
    /// Setting values on screen, keyed "section.key".
    pub draft: RwSignal<HashMap<String, Value>>,
    pub saving: RwSignal<bool>,
    pub save_message: RwSignal<String>,
    pub save_error: RwSignal<String>,
    pub backup: RwSignal<String>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            screen: RwSignal::new("project"),
            scan_status: RwSignal::new("idle"),
            scan_summary: RwSignal::new(None),
            scan_message: RwSignal::new(String::new()),
            scan_log: RwSignal::new(Vec::new()),
            folder: RwSignal::new(saved(STORAGE_KEY)),
            facts: RwSignal::new(None),
            checking: RwSignal::new(false),
            error: RwSignal::new(String::new()),
            no_config: RwSignal::new(None),
            created: RwSignal::new(false),
            example: RwSignal::new(false),
            draft: RwSignal::new(HashMap::new()),
            saving: RwSignal::new(false),
            save_message: RwSignal::new(String::new()),
            save_error: RwSignal::new(String::new()),
            backup: RwSignal::new(String::new()),
        }
    }

    /// A field of the project facts (null when there are none).
    pub fn fact(&self, key: &str) -> Value {
        self.facts.with(|f| f.as_ref().map(|f| f[key].clone()).unwrap_or(Value::Null))
    }

    /// The value on screen for a setting.
    pub fn get(&self, section: &str, key: &str) -> Value {
        self.draft.with(|d| d.get(&format!("{section}.{key}")).cloned().unwrap_or(Value::Null))
    }

    pub fn set(&self, section: &str, key: &str, value: Value) {
        self.draft.update(|d| {
            d.insert(format!("{section}.{key}"), value);
        });
    }

    /// Settings whose value on screen differs from the settings file.
    pub fn changed(&self) -> Vec<Value> {
        let originals = originals(self.facts.get());
        let mut out: Vec<Value> = self
            .draft
            .get()
            .into_iter()
            .filter(|(k, v)| v != originals.get(k).unwrap_or(&Value::Null))
            .map(|(k, value)| {
                let (section, key) = k.split_once('.').unwrap_or((&k, ""));
                json!({ "section": section, "key": key, "value": value })
            })
            .collect();
        out.sort_by_key(|c| (c["section"].to_string(), c["key"].to_string()));
        out
    }

    /// Put the saved values on screen (after opening a project, saving or discarding).
    pub fn reset_draft(&self) {
        self.draft.set(originals(self.facts.get_untracked()));
        self.save_error.set(String::new());
    }

    fn reset_project_state(&self) {
        self.facts.set(None);
        self.error.set(String::new());
        self.no_config.set(None);
        self.created.set(false);
        self.example.set(false);
        // results belong to the previous project
        self.scan_status.set("idle");
        self.scan_summary.set(None);
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

fn originals(facts: Option<Value>) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    if let Some(Value::Array(settings)) = facts.map(|f| f["settings"].clone()) {
        for s in settings {
            out.insert(
                format!("{}.{}", s["section"].as_str().unwrap_or(""), s["key"].as_str().unwrap_or("")),
                s["value"].clone(),
            );
        }
    }
    out
}

/// Use what the engine reported: the project, or that it has no settings file.
pub fn apply_inspect(store: Store, events: &[Value], fallback: &str) {
    let facts = events.iter().find(|e| e["type"] == "project");
    let none = events.iter().find(|e| e["type"] == "no_config");
    if let Some(facts) = facts {
        store.facts.set(Some(facts.clone()));
        store.no_config.set(None);
        set_job_config(facts["config"].as_str().unwrap_or(""));
        let folder = store.folder.get_untracked().to_lowercase();
        if folder.ends_with(".yml") || folder.ends_with(".yaml") {
            store.folder.set(facts["root"].as_str().unwrap_or("").to_string());
        }
        remember(STORAGE_KEY, &store.folder.get_untracked());
        store.reset_draft();
    } else if let Some(none) = none {
        store.facts.set(None);
        store.no_config.set(Some(none.clone()));
    } else {
        store.error.set(failure_message(events, fallback));
    }
}

async fn inspect(store: Store, spec: Value, fallback: &str) {
    store.checking.set(true);
    match run_job(spec, |_| {}).await {
        Ok(events) => apply_inspect(store, &events, fallback),
        Err(e) => store.error.set(e),
    }
    store.checking.set(false);
}

pub async fn refresh_now(store: Store) {
    let folder = store.folder.get_untracked();
    if folder.is_empty() {
        return;
    }
    store.error.set(String::new());
    inspect(store, json!({ "job": "inspect", "project": folder }), "The project could not be read.").await;
}

/// Open a project folder and ask the engine what is in it.
pub fn open_project(store: Store, folder: String) {
    store.folder.set(folder.clone());
    store.reset_project_state();
    set_job_config("");
    remember(STORAGE_KEY, &folder);
    spawn_local(async move { refresh_now(store).await });
}

/// Open a settings file kept anywhere; its project is the folder it points to.
pub fn open_config_file(store: Store, path: String) {
    store.folder.set(path);
    store.reset_project_state();
    set_job_config("");
    spawn_local(async move { refresh_now(store).await });
}

/// Start a new project: write pamflow's default settings for it, then open it.
pub fn create_project(store: Store, folder: String, target: Option<String>) {
    store.folder.set(folder.clone());
    store.reset_project_state();
    set_job_config("");
    spawn_local(async move {
        store.checking.set(true);
        let mut spec = json!({ "job": "project_create", "project": folder });
        if let Some(t) = target {
            spec["target"] = json!(t);
        }
        match run_job(spec, |_| {}).await {
            Ok(events) => {
                let made = events.iter().any(|e| e["type"] == "project_created");
                apply_inspect(store, &events, "The new project could not be created.");
                if made && store.facts.get_untracked().is_some() {
                    store.created.set(true);
                }
            }
            Err(e) => store.error.set(e),
        }
        store.checking.set(false);
    });
}

/// Make pamflow's small example project in the Documents folder and open it.
pub fn create_example(store: Store) {
    store.reset_project_state();
    set_job_config("");
    store.folder.set(String::new());
    spawn_local(async move {
        store.checking.set(true);
        match run_job(json!({ "job": "example_project", "project": "" }), |_| {}).await {
            Ok(events) => {
                let made = events.iter().find(|e| e["type"] == "example_created").cloned();
                if let Some(m) = &made {
                    store.folder.set(m["root"].as_str().unwrap_or("").to_string());
                }
                apply_inspect(store, &events, "The example project could not be made.");
                if made.is_some() && store.facts.get_untracked().is_some() {
                    store.example.set(true);
                }
            }
            Err(e) => store.error.set(e),
        }
        store.checking.set(false);
    });
}

/// Write the settings (with anything not yet saved) to the settings file, or,
/// with `target`, to a different file; the previous version is backed up.
pub fn save_settings(store: Store, target: Option<String>) {
    let changes = store.changed();
    let folder = store.folder.get_untracked();
    if folder.is_empty() || (changes.is_empty() && target.is_none()) {
        return;
    }
    spawn_local(async move {
        store.saving.set(true);
        store.save_error.set(String::new());
        store.save_message.set(String::new());
        let n = changes.len();
        let spec = match &target {
            Some(t) => json!({ "job": "config_save_as", "project": folder, "changes": changes, "target": t }),
            None => json!({ "job": "config_save", "project": folder, "changes": changes }),
        };
        match run_job(spec, |_| {}).await {
            Ok(events) => {
                let saved = events.iter().find(|e| e["type"] == "config_saved").cloned();
                let ok = events.iter().any(|e| e["type"] == "finished" && e["status"] == "ok");
                if let (Some(saved), true) = (&saved, ok) {
                    apply_inspect(store, &events, "");
                    store.created.set(false);
                    store.backup.set(saved["backup"].as_str().unwrap_or("").to_string());
                    store.save_message.set(n.to_string());
                    store.reset_draft();
                } else if saved.is_none() && ok {
                    // nothing to do
                } else {
                    store.save_error.set(failure_message(&events, "The settings could not be saved."));
                }
            }
            Err(e) => store.save_error.set(e),
        }
        store.saving.set(false);
    });
}

// ---- Scan ------------------------------------------------------------------------------------------

async fn scan_job(store: Store, job: &str, force: bool) {
    let folder = store.folder.get_untracked();
    let events = match run_job(json!({ "job": job, "project": folder, "force": force }), |_| {}).await {
        Ok(e) => e,
        Err(e) => {
            store.scan_status.set("failed");
            store.scan_message.set(e);
            return;
        }
    };
    let finished_ok = events.iter().any(|e| e["type"] == "finished" && e["status"] == "ok");
    store.scan_log.set(
        events
            .iter()
            .filter(|e| e["type"] == "log" || e["type"] == "text")
            .map(|e| e["text"].as_str().unwrap_or("").to_string())
            .collect(),
    );
    if let Some(summary) = events.iter().find(|e| e["type"] == "scan") {
        store.scan_summary.set(Some(summary.clone()));
    }
    store.scan_status.set(if finished_ok { "ready" } else { "failed" });
    store.scan_message.set(if finished_ok { String::new() } else { failure_message(&events, "Scanning failed.") });
}

/// Show what the last scan found (`scan_results`).
pub fn load_scan(store: Store) {
    if store.folder.get_untracked().is_empty() {
        return;
    }
    store.scan_status.set("loading");
    spawn_local(async move { scan_job(store, "scan_results", false).await });
}

/// Scan the recordings; the Project screen then shows the new scan.
pub fn run_scan(store: Store, force: bool) {
    store.scan_status.set("scanning");
    store.scan_message.set(String::new());
    spawn_local(async move {
        scan_job(store, "scan", force).await;
        refresh_now(store).await;
    });
}
