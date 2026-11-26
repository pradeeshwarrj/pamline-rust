//! Talking to the Rust side of the app (the Tauri shell). Every call to the
//! engine goes through this file, so the screens never deal with Tauri
//! directly. (Port of `src/lib/backend/tauri.ts`.)
//!
//! The shell is built with `withGlobalTauri`, so everything is reached through
//! `window.__TAURI__`; values cross the boundary as JSON text.

use futures::channel::oneshot;
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn invoke_js(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn listen_js(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

fn from_js(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return Value::Null;
    }
    js_sys::JSON::stringify(v)
        .ok()
        .and_then(|s| s.as_string())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

fn err_text(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| from_js(&e).to_string())
}

/// Is the page running inside the app (not in a plain browser)?
pub fn in_app() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &"__TAURI__".into()).map(|v| !v.is_undefined()).unwrap_or(false)
}

/// Call a Tauri command.
pub async fn invoke(cmd: &str, args: impl Serialize) -> Result<Value, String> {
    let args = serde_json::to_value(args).map_err(|e| e.to_string())?;
    invoke_js(cmd, to_js(&args)).await.map(|v| from_js(&v)).map_err(err_text)
}

// The settings file the open project uses; every job carries it, so a
// settings file kept anywhere (not only `<project>/config/config.yml`) works.
thread_local! {
    static JOB_CONFIG: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn set_job_config(path: &str) {
    JOB_CONFIG.with(|c| *c.borrow_mut() = path.to_string());
}

/// Receive every job's events until `stop` is called.
async fn listen_job_events(mut handler: impl FnMut(String, Value) + 'static) -> Result<impl FnOnce(), String> {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |msg: JsValue| {
        let payload = from_js(&js_sys::Reflect::get(&msg, &"payload".into()).unwrap_or(JsValue::NULL));
        let id = payload["id"].as_str().unwrap_or("").to_string();
        handler(id, payload["event"].clone());
    });
    let unlisten = listen_js("job-event", &closure).await.map_err(err_text)?;
    Ok(move || {
        if let Ok(f) = unlisten.dyn_into::<js_sys::Function>() {
            let _ = f.call0(&JsValue::NULL);
        }
        drop(closure);
    })
}

/// Run a job and collect its events until it ends (the "exit" event).
/// `on_event` sees each event as it arrives.
pub async fn run_job(mut spec: Value, on_event: impl Fn(&Value) + 'static) -> Result<Vec<Value>, String> {
    if spec.get("config").is_none() {
        let cfg = JOB_CONFIG.with(|c| c.borrow().clone());
        if !cfg.is_empty() {
            spec["config"] = json!(cfg);
        }
    }
    struct Shared {
        job: Option<String>,
        early: Vec<(String, Value)>,
        events: Vec<Value>,
        done: Option<oneshot::Sender<()>>,
    }
    let (tx, rx) = oneshot::channel();
    let shared = Rc::new(RefCell::new(Shared { job: None, early: Vec::new(), events: Vec::new(), done: Some(tx) }));
    let on_event = Rc::new(on_event);
    let take = {
        let (shared, on_event) = (shared.clone(), on_event.clone());
        move |id: String, ev: Value| {
            let mut s = shared.borrow_mut();
            match &s.job {
                None => s.early.push((id, ev)),
                Some(job) if *job == id => {
                    on_event(&ev);
                    let exit = ev["type"] == "exit";
                    s.events.push(ev);
                    if exit {
                        if let Some(tx) = s.done.take() {
                            let _ = tx.send(());
                        }
                    }
                }
                Some(_) => {}
            }
        }
    };
    let stop = listen_job_events(take.clone()).await?;
    let started = invoke("start_job", json!({ "spec": spec })).await;
    let id = match started {
        Ok(v) => v.as_str().unwrap_or("").to_string(),
        Err(e) => {
            stop();
            return Err(e);
        }
    };
    let early = {
        let mut s = shared.borrow_mut();
        s.job = Some(id);
        std::mem::take(&mut s.early)
    };
    for (id, ev) in early {
        take(id, ev);
    }
    let _ = rx.await;
    stop();
    let events = std::mem::take(&mut shared.borrow_mut().events);
    Ok(events)
}

/// Why a job failed, for the screen (`failureMessage` in tauri.ts).
pub fn failure_message(events: &[Value], fallback: &str) -> String {
    if let Some(m) = events.iter().find(|e| e["type"] == "finished").and_then(|e| e["message"].as_str()).filter(|m| !m.is_empty()) {
        return m.to_string();
    }
    let printed: Vec<String> = events
        .iter()
        .filter(|e| e["type"] == "text" || (e["type"] == "log" && e["level"] == "ERROR"))
        .filter_map(|e| e["text"].as_str().map(str::to_string))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail: Vec<&String> = printed.iter().rev().take(12).rev().collect();
    let code = events
        .iter()
        .find(|e| e["type"] == "exit")
        .and_then(|e| e["code"].as_i64())
        .map(|c| format!(" (exit code {c})"))
        .unwrap_or_default();
    if tail.is_empty() {
        format!("{fallback}{code}")
    } else {
        format!("{fallback}{code}\n{}", tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"))
    }
}

// The dialog and opener plugins are reached through their commands: the global
// `window.__TAURI__` only carries Tauri's own modules.

/// Ask the user for a folder. None if they cancel.
pub async fn pick_folder(title: &str) -> Option<String> {
    let opts = json!({ "options": { "directory": true, "multiple": false, "title": title } });
    invoke("plugin:dialog|open", opts).await.ok().and_then(|v| v.as_str().map(str::to_string))
}

/// Ask the user for a file. `extensions` like `["csv"]`. None if they cancel.
pub async fn pick_file(title: &str, name: &str, extensions: &[&str]) -> Option<String> {
    let opts = json!({ "options": { "directory": false, "multiple": false, "title": title,
        "filters": [{ "name": name, "extensions": extensions }] } });
    invoke("plugin:dialog|open", opts).await.ok().and_then(|v| v.as_str().map(str::to_string))
}

/// Ask where to save a file. None if they cancel.
pub async fn pick_save_path(title: &str, default_path: &str, name: &str, extensions: &[&str]) -> Option<String> {
    let opts = json!({ "options": { "title": title, "defaultPath": default_path,
        "filters": [{ "name": name, "extensions": extensions }] } });
    invoke("plugin:dialog|save", opts).await.ok().and_then(|v| v.as_str().map(str::to_string))
}

/// Show a file or folder in Explorer / Finder.
pub async fn show_folder(path: &str) {
    let _ = invoke("plugin:opener|reveal_item_in_dir", json!({ "paths": [path] })).await;
}
