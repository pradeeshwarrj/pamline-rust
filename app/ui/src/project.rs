//! Step 1 · Project: open a pamflow project and edit its settings (port of
//! `ProjectPicker.svelte` and `ProjectLive.svelte`). This first version has the
//! picker, the facts, the recordings/sheet cards, detection, filtering and
//! cleaning settings for workflow A, and the save bar; the model card, the
//! soundscape card, "More settings" and workflow B follow.

use crate::components::{Badge, Button, Card, PageHeader, Pills, StatTile};
use crate::i18n::{t, tv};
use crate::state::{create_example, create_project, open_config_file, open_project, refresh_now, save_settings, Store};
use crate::tauri;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{json, Value};

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn slash(p: &str) -> String {
    p.replace('\\', "/")
}

/// A number with thousands separators (toLocaleString).
fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}{}", if n < 0 { "-" } else { "" }, out.chars().rev().collect::<String>())
}

// ---- setting fields -----------------------------------------------------------------------

fn text_field(store: Store, section: &'static str, key: &'static str, placeholder: &'static str) -> impl IntoView {
    view! {
        <input class="input" placeholder=placeholder
            prop:value=move || text(&store.get(section, key))
            on:input=move |ev| store.set(section, key, json!(event_target_value(&ev))) />
    }
}

fn number_field(store: Store, section: &'static str, key: &'static str, min: &'static str, max: &'static str, step: &'static str, integer: bool) -> impl IntoView {
    view! {
        <input class="input" type="number" min=min max=max step=step
            prop:value=move || match store.get(section, key) {
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            }
            on:input=move |ev| {
                let raw = event_target_value(&ev);
                if raw.trim().is_empty() {
                    return store.set(section, key, Value::Null);
                }
                if let Ok(n) = raw.trim().parse::<f64>() {
                    if n.is_finite() {
                        store.set(section, key, if integer { json!(n.round() as i64) } else { json!(n) });
                    }
                }
            } />
    }
}

fn check_field(store: Store, section: &'static str, key: &'static str, label: String, default: bool) -> impl IntoView {
    view! {
        <label class="check">
            <input type="checkbox"
                prop:checked=move || store.get(section, key).as_bool().unwrap_or(default)
                on:change=move |ev| store.set(section, key, json!(event_target_checked(&ev))) />
            {label}
        </label>
    }
}

fn field<V: IntoView + 'static>(label: String, input: V) -> impl IntoView {
    view! { <div class="field"><span class="sub">{label}</span>{input}</div> }
}

// ---- the picker ------------------------------------------------------------------------------

#[component]
fn ProjectPicker(store: Store) -> impl IntoView {
    let folder = RwSignal::new(store.folder.get_untracked());
    // follows the open project
    Effect::new(move |_| folder.set(store.folder.get()));
    let busy = Signal::derive(move || store.checking.get());
    let no_folder = Signal::derive(move || folder.get().is_empty() || store.checking.get());

    let browse = move |_| {
        spawn_local(async move {
            if let Some(p) = tauri::pick_folder(&t("project.folderTitle")).await {
                folder.set(p.clone());
                open_project(store, p);
            }
        });
    };
    let new_project = move |_| {
        spawn_local(async move {
            if let Some(p) = tauri::pick_folder(&t("project.newTitle")).await {
                open_project(store, p.clone());
                // an existing settings file is simply opened, never replaced
                while store.checking.get_untracked() || (store.facts.get_untracked().is_none() && store.no_config.get_untracked().is_none() && store.error.get_untracked().is_empty()) {
                    gloo_wait().await;
                }
                if store.no_config.get_untracked().is_some() {
                    create_project(store, p, None);
                }
            }
        });
    };
    let open_other = move |_| {
        spawn_local(async move {
            if let Some(p) = tauri::pick_file(&t("project.openOtherTitle"), &t("settings.yamlFiles"), &["yml", "yaml"]).await {
                open_config_file(store, p);
            }
        });
    };
    let create_elsewhere = move |_| {
        let Some(nc) = store.no_config.get_untracked() else { return };
        spawn_local(async move {
            let suggested = text(&nc["suggested"]);
            if let Some(p) = tauri::pick_save_path(&t("settings.saveAsTitle"), &suggested, &t("settings.yamlFiles"), &["yml", "yaml"]).await {
                create_project(store, text(&nc["folder"]), Some(p));
            }
        });
    };

    let status = move || {
        if store.checking.get() {
            return view! { <span class="muted">{t("project.checking")}</span> }.into_any();
        }
        let error = store.error.get();
        if !error.is_empty() {
            return view! { <pre class="error">{error}</pre> }.into_any();
        }
        if let Some(nc) = store.no_config.get() {
            let exists = nc["exists"].as_bool().unwrap_or(false);
            let missing = nc["missing_file"].as_str().map(str::to_string);
            if !exists && missing.is_none() {
                return view! { <pre class="error">{tv("project.folderMissing", &[("folder", text(&nc["folder"]))])}</pre> }.into_any();
            }
            let (title, body) = match &missing {
                Some(f) => (t("project.configMissingTitle"), tv("project.configMissingBody", &[("file", f.clone())])),
                None => (t("project.noConfigTitle"), tv("project.noConfigBody", &[("file", text(&nc["suggested"]))])),
            };
            let nc_folder = text(&nc["folder"]);
            return view! {
                <div class="offer">
                    <strong>{title}</strong>
                    <span class="muted">{body}</span>
                    {exists.then(|| view! {
                        <div class="row">
                            <Button variant="primary" on_click=move |_| create_project(store, nc_folder.clone(), None)>{t("project.createHere")}</Button>
                            <Button on_click=create_elsewhere>{t("project.createElsewhere")}</Button>
                        </div>
                    })}
                </div>
            }
            .into_any();
        }
        let Some(f) = store.facts.get() else { return ().into_any() };
        let config = text(&f["config"]);
        let config_for_click = config.clone();
        let config_title = config.clone();
        let scanned = f["scanned"].as_bool().unwrap_or(false);
        let audio_found = f["audio_folder_found"].as_bool().unwrap_or(false);
        let recordings_text = tv("project.recordingsN", &[("n", thousands(f["n_audio"].as_i64().unwrap_or(0)))]);
        let audio_missing_text = tv("project.audioMissing", &[("folder", text(&f["audio_folder"]))]);
        let model_found = f["model_found"].as_bool().unwrap_or(false);
        let model_missing_text = tv("project.modelMissing", &[("file", text(&f["model_file"]))]);
        view! {
            <div class="chips">
                {if store.created.get() {
                    view! { <Badge palette="sky">{t("project.configCreated")}</Badge> }.into_any()
                } else {
                    view! { <Badge palette="mint">{t("project.configFound")}</Badge> }.into_any()
                }}
                {if audio_found {
                    view! { <Badge palette="mint">{recordings_text}</Badge> }.into_any()
                } else {
                    view! { <Badge palette="peach">{audio_missing_text}</Badge> }.into_any()
                }}
                {if scanned {
                    view! { <Badge palette="mint">{t("project.scanned")}</Badge> }.into_any()
                } else {
                    view! { <Badge>{t("project.notScanned")}</Badge> }.into_any()
                }}
                {if model_found {
                    view! { <Badge palette="mint">{t("project.modelOk")}</Badge> }.into_any()
                } else {
                    view! { <Badge palette="peach">{model_missing_text}</Badge> }.into_any()
                }}
            </div>
            <div class="file">
                <span class="muted">{t("project.settingsFile")}</span>
                <span class="path mono" title=config_title>{config}</span>
                <Button variant="ghost" size="sm" on_click=move |_| { let c = config_for_click.clone(); spawn_local(async move { tauri::show_folder(&c).await }) }>
                    {t("project.showFile")}
                </Button>
            </div>
            {if store.example.get() {
                view! { <span class="muted">{t("project.exampleHint")}</span> }.into_any()
            } else if store.created.get() {
                view! { <span class="muted">{t("project.createdHint")}</span> }.into_any()
            } else {
                ().into_any()
            }}
        }
        .into_any()
    };

    view! {
        <Card title=t("project.folderTitle") hint=t("project.folderHint")>
            <div class="row">
                <input class="input" prop:value=move || folder.get()
                    on:input=move |ev| folder.set(event_target_value(&ev))
                    on:keydown=move |ev| { if ev.key() == "Enter" { open_project(store, folder.get_untracked()) } } />
                <Button on_click=browse>{t("common.browse")}</Button>
                <Button variant="primary" disabled=no_folder on_click=move |_| open_project(store, folder.get_untracked())>{t("project.open")}</Button>
            </div>
            <div class="row links">
                <Button variant="ghost" size="sm" disabled=busy on_click=new_project>{t("project.newProject")}</Button>
                <Button variant="ghost" size="sm" disabled=busy on_click=open_other>{t("project.openOther")}</Button>
                <Button variant="ghost" size="sm" disabled=busy on_click=move |_| create_example(store)>{t("project.tryExample")}</Button>
            </div>
            {status}
        </Card>
    }
}

/// A short pause between checks while a job runs.
async fn gloo_wait() {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
        let _ = tx.send(());
    });
    if let Some(w) = web_sys::window() {
        let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(wasm_bindgen::JsCast::unchecked_ref(&cb), 100);
    }
    let _ = rx.await;
}

// ---- the save bar --------------------------------------------------------------------------------

#[component]
fn SaveBar(store: Store) -> impl IntoView {
    let n_changes = move || store.changed().len();
    let saving = Signal::derive(move || store.saving.get());
    let save_as = move |_| {
        spawn_local(async move {
            let current = text(&store.fact("config"));
            if let Some(p) = tauri::pick_save_path(&t("settings.saveAsTitle"), &current, &t("settings.yamlFiles"), &["yml", "yaml"]).await {
                save_settings(store, Some(p));
            }
        });
    };
    let bar = move || {
        let n = n_changes();
        if n > 0 {
            view! {
                <div class="savebar dirty">
                    <span>{tv("settings.unsaved", &[("n", n.to_string())])}</span>
                    <div class="actions">
                        <Button variant="ghost" disabled=saving on_click=move |_| store.reset_draft()>{t("settings.discard")}</Button>
                        <Button disabled=saving on_click=save_as>{t("settings.saveAs")}</Button>
                        <Button variant="primary" disabled=saving on_click=move |_| save_settings(store, None)>
                            {move || if store.saving.get() { t("settings.saving") } else { t("settings.save") }}
                        </Button>
                    </div>
                </div>
            }
            .into_any()
        } else if !store.save_message.get().is_empty() {
            let backup = store.backup.get();
            view! {
                <div class="savebar">
                    <span>{tv("settings.saved", &[("n", store.save_message.get())])}</span>
                    <div class="actions">
                        {(!backup.is_empty()).then(|| view! {
                            <Button variant="ghost" size="sm" on_click=move |_| { let b = backup.clone(); spawn_local(async move { tauri::show_folder(&b).await }) }>
                                {t("settings.showBackup")}
                            </Button>
                        })}
                        <Button variant="ghost" size="sm" on_click=save_as>{t("settings.saveAs")}</Button>
                    </div>
                </div>
            }
            .into_any()
        } else {
            view! {
                <div class="savebar">
                    <span class="muted">{t("settings.allSaved")}</span>
                    <Button variant="ghost" size="sm" on_click=save_as>{t("settings.saveAs")}</Button>
                </div>
            }
            .into_any()
        }
    };
    view! {
        {bar}
        {move || { let e = store.save_error.get(); (!e.is_empty()).then(|| view! { <p class="problem">{e}</p> }) }}
    }
}

// ---- the screen -----------------------------------------------------------------------------------

#[component]
pub fn ProjectScreen(store: Store) -> impl IntoView {
    // open the remembered project once
    if !store.folder.get_untracked().is_empty() {
        spawn_local(async move { refresh_now(store).await });
    }

    let choose_recordings = move |_| {
        spawn_local(async move {
            if let Some(p) = tauri::pick_folder(&t("settings.recordingsFolder")).await {
                store.set("paths", "audio_standardized", json!(slash(&p)));
            }
        });
    };
    let choose_sheet = move |_| {
        spawn_local(async move {
            if let Some(p) = tauri::pick_file(&t("settings.sheet"), &t("settings.sheetFiles"), &["csv", "txt"]).await {
                store.set("paths", "deployment_sheet", json!(slash(&p)));
            }
        });
    };

    let size_label = move || match store.fact("audio_gb").as_f64() {
        None => "–".to_string(),
        Some(gb) if gb >= 1024.0 => format!("{:.1} TB", gb / 1024.0),
        Some(gb) => format!("{gb:.1} GB"),
    };
    let dash = |v: Value| v.as_i64().map(thousands).unwrap_or_else(|| "–".into());

    // worker choices: "Auto" (pamflow: half the physical cores) or a number
    let worker_options = move || {
        let physical = store.fact("cores_physical").as_i64().unwrap_or(4).max(1);
        let mut n: Vec<i64> = [2, 4, 8, 12, (physical - 2).max(1)].into_iter().filter(|n| *n <= (physical - 1).max(1)).collect();
        n.sort();
        n.dedup();
        let mut o = vec![("0".to_string(), t("settings.auto"))];
        o.extend(n.into_iter().map(|n| (n.to_string(), n.to_string())));
        o
    };
    let workers = Signal::derive(move || store.get("birdnet", "workers").as_i64().unwrap_or(0).to_string());
    let set_workers = Callback::new(move |v: String| {
        let n: i64 = v.parse().unwrap_or(0);
        store.set("birdnet", "workers", if n == 0 { Value::Null } else { json!(n) });
    });
    let input = Signal::derive(move || store.get("birdnet", "input").as_str().unwrap_or("standardized").to_string());
    let set_input = Callback::new(move |v: String| store.set("birdnet", "input", json!(v)));
    let confidence = move || store.get("birdnet", "min_confidence").as_f64().unwrap_or(0.0);

    let details = move || {
        let Some(f) = store.facts.get() else { return ().into_any() };
        let physical = f["cores_physical"].as_i64().unwrap_or(4);
        let example = f["filename_example"].clone();
        let has_example = example.is_object();
        let ex_valid = example["valid"].as_bool().unwrap_or(false);
        let ex_file = text(&example["file"]);
        let ex_grid_text = tv("settings.grid", &[("grid", text(&example["grid"]))]);
        let ex_start = text(&example["start"]);
        let unreadable = f["n_unreadable_names"].as_i64().filter(|n| *n > 0).map(|n| {
            tv("settings.unreadableNames", &[("n", n.to_string()), ("of", example["sample_size"].to_string())])
        });
        let n_audio = dash(f["n_audio"].clone());
        let n_deployments = f["n_deployments"].as_i64().map(|n| n.to_string()).unwrap_or_else(|| "–".into());
        let n_labels = dash(f["n_labels"].clone());
        let sheet_found = f["deployment_sheet_found"].as_bool().unwrap_or(false);
        let sheet_rows = tv("project.sheetRows", &[("n", n_deployments.clone())]);
        let workers_hint = tv("project.workersHint", &[("n", physical.to_string())]);
        view! {
            <div class="grid-4">
                <StatTile value=n_audio label=t("project.recordingsFound") />
                <StatTile value=size_label() label=t("project.ofAudio") palette="mint" />
                <StatTile value=n_deployments label=t("project.deploymentsInSheet") palette="lavender" />
                <StatTile value=n_labels label=t("settings.speciesInModel") palette="peach" />
            </div>

            <div class="grid-2">
                <Card title=t("settings.recordingsFolder") hint=t("project.recordingsFolderHint")>
                    <div class="row">
                        {text_field(store, "paths", "audio_standardized", "")}
                        <Button on_click=choose_recordings>{t("common.browse")}</Button>
                    </div>
                    {has_example.then(|| view! {
                        <div class="chips">
                            <Badge>{ex_file}</Badge>
                            {if ex_valid {
                                view! {
                                    <Badge palette="mint">{ex_grid_text}</Badge>
                                    <Badge palette="mint">{ex_start}</Badge>
                                }.into_any()
                            } else {
                                view! { <Badge palette="peach">{t("settings.nameNotRead")}</Badge> }.into_any()
                            }}
                        </div>
                    })}
                    {unreadable.map(|msg| view! { <Badge palette="peach">{msg}</Badge> })}
                    <span class="sub">{t("settings.timezone")}</span>
                    {text_field(store, "filenames", "timezone", "Asia/Kolkata")}
                </Card>

                <Card title=t("project.sheet") hint=t("project.sheetHint")>
                    <div class="row">
                        {text_field(store, "paths", "deployment_sheet", "")}
                        <Button on_click=choose_sheet>{t("common.browse")}</Button>
                    </div>
                    {if sheet_found {
                        view! { <span class="muted">{sheet_rows}</span> }.into_any()
                    } else {
                        view! { <Badge palette="peach">{t("settings.sheetMissing")}</Badge> }.into_any()
                    }}
                </Card>

                <Card title=t("settings.detection") hint=t("settings.detectionHint")>
                    <div class="between">
                        <span class="strong">{t("project.confidence")}</span>
                        <span class="value">{move || format!("{:.2}", confidence())}</span>
                    </div>
                    <input type="range" min="0.05" max="0.95" step="0.05"
                        prop:value=move || confidence().to_string()
                        on:input=move |ev| {
                            if let Ok(n) = event_target_value(&ev).parse::<f64>() {
                                store.set("birdnet", "min_confidence", json!(n));
                            }
                        } />
                    <span class="sub">{t("project.workers")}</span>
                    <Pills options=worker_options() value=workers on_change=set_workers />
                    <span class="muted">{workers_hint}</span>
                    <div class="pair">
                        {field(t("settings.overlap"), number_field(store, "birdnet", "overlap_s", "0", "2.9", "0.5", false))}
                        {field(t("settings.input"), view! {
                            <Pills options=vec![("standardized".to_string(), t("settings.inputStandardized")), ("filtered".to_string(), t("settings.inputFiltered"))]
                                value=input on_change=set_input />
                        })}
                    </div>
                </Card>

                <Card title=t("settings.filterTitle") hint=t("settings.filterHint")>
                    {check_field(store, "filter", "enabled", t("settings.filterOn"), false)}
                    <div class="pair">
                        {field(t("settings.highpass"), number_field(store, "filter", "highpass_hz", "0", "", "50", false))}
                        {field(t("settings.filterOrder"), number_field(store, "filter", "order", "1", "12", "1", true))}
                    </div>
                    <span class="sub">{t("settings.filteredFolder")}</span>
                    {text_field(store, "paths", "filtered", "")}
                </Card>

                <Card title=t("settings.cleaning") hint=t("settings.cleaningHint")>
                    <div class="pair">
                        {field(t("process.minConfidence"), number_field(store, "processing", "min_confidence", "0", "1", "0.05", false))}
                        {field(t("settings.mergeGap"), number_field(store, "processing", "merge_gap_s", "0", "", "1", false))}
                    </div>
                    {check_field(store, "processing", "drop_outside_deployment", t("settings.dropOutside"), true)}
                </Card>

                <Card title=t("settings.occupancy") hint=t("settings.occupancyHint")>
                    <div class="pair">
                        {field(t("occupancy.occasionLength"), number_field(store, "occupancy", "occasion_length_days", "1", "", "1", true))}
                        {field(t("occupancy.minEffort"), number_field(store, "occupancy", "min_effort_hours", "0", "", "0.5", false))}
                        {field(t("occupancy.minDetections"), number_field(store, "occupancy", "min_detections", "1", "", "1", true))}
                        {field(t("occupancy.run"), text_field(store, "occupancy", "run_id", ""))}
                    </div>
                </Card>
            </div>

            <SaveBar store=store />
        }
        .into_any()
    };

    view! {
        <PageHeader number=1 name=t("stages.project") title=t("project.title") intro=t("project.intro") />
        <ProjectPicker store=store />
        {details}
    }
}
