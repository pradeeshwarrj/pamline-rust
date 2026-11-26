//! Step 3 · Scan: read and check the recordings (port of `ScanLive.svelte`).

use crate::components::{Badge, Button, Card, PageHeader, StatTile};
use crate::i18n::{t, tv};
use crate::state::{load_scan, refresh_now, run_scan, Store};
use crate::tauri;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;

/// A number the way `toLocaleString` writes it: thousands separators, up to 3 decimals.
fn locale(x: f64) -> String {
    let mut s = format!("{:.3}", x.abs());
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((s.clone(), String::new()));
    let mut out = String::new();
    for (i, c) in int.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}{}{}", if x < 0.0 { "-" } else { "" }, out.chars().rev().collect::<String>(), frac)
}

fn count(v: &Value) -> String {
    locale(v.as_f64().unwrap_or(0.0))
}

/// A table cell: dates as "2026-04-05 17:30", nothing for null.
fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) if s.len() >= 16 && s.as_bytes()[4] == b'-' && s.as_bytes()[10] == b'T' => s[..16].replace('T', " "),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A table of JSON rows; the columns are the keys of the first row.
fn table(rows: Vec<Value>) -> impl IntoView {
    let cols: Vec<String> = rows.first().and_then(|r| r.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default();
    view! {
        <div class="table-wrap">
            <table>
                <thead>
                    <tr>{cols.iter().map(|c| view! { <th>{c.replace('_', " ")}</th> }).collect_view()}</tr>
                </thead>
                <tbody>
                    {rows
                        .iter()
                        .map(|row| view! { <tr>{cols.iter().map(|c| view! { <td>{cell(&row[c.as_str()])}</td> }).collect_view()}</tr> })
                        .collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
pub fn ScanScreen(store: Store) -> impl IntoView {
    // show the last scan, and make sure the project facts are there
    if !store.folder.get_untracked().is_empty() {
        if store.scan_status.get_untracked() == "idle" {
            load_scan(store);
        }
        if store.facts.get_untracked().is_none() && !store.checking.get_untracked() {
            spawn_local(async move { refresh_now(store).await });
        }
    }
    let force = RwSignal::new(false);
    let show_log = RwSignal::new(false);
    let scanning = move || store.scan_status.get() == "scanning";
    let busy = Signal::derive(move || store.scan_status.get() == "scanning");
    let summary = move || store.scan_summary.get().filter(|s| s["available"].as_bool().unwrap_or(false));

    let body = move || {
        if store.folder.get().is_empty() {
            return view! {
                <Card title=t("detect.chooseProject")>
                    <div><Button variant="primary" on_click=move |_| store.screen.set("project")>{t("detect.goProject")}</Button></div>
                </Card>
            }
            .into_any();
        }
        let s = summary();
        let available = s.is_some();
        let folder_hint = s.as_ref().and_then(|s| s["audio_folder"].as_str()).map(|f| tv("scan.folder", &[("folder", f.to_string())])).unwrap_or_default();
        let metadata = s.as_ref().and_then(|s| s["metadata_folder"].as_str().map(str::to_string));
        let updated = s.as_ref().map(|s| tv("scan.updated", &[("when", s["updated"].as_str().unwrap_or("").to_string())]));
        let run_card = view! {
            <Card title=t("scan.runTitle") hint=folder_hint>
                <label class="check"><input type="checkbox" prop:checked=move || force.get() on:change=move |ev| force.set(event_target_checked(&ev)) /> {t("scan.force")}</label>
                <div class="actions">
                    <Button variant="primary" size="lg" disabled=busy on_click=move |_| run_scan(store, force.get_untracked())>
                        {move || if scanning() { t("scan.scanning") } else if summary().is_some() { t("scan.again") } else { t("scan.run") }}
                    </Button>
                    {available.then(|| {
                        let metadata = metadata.clone();
                        view! {
                            <Button on_click=move |_| { if let Some(m) = metadata.clone() { spawn_local(async move { tauri::show_folder(&m).await }) } }>{t("detect.openFolder")}</Button>
                            <span class="muted">{updated.clone()}</span>
                        }
                    })}
                </div>
                {move || scanning().then(|| view! { <p class="muted">{t("scan.scanningHint")}</p> })}
                {move || {
                    let msg = store.scan_message.get();
                    (!msg.is_empty()).then(|| {
                        let has_log = !store.scan_log.get().is_empty();
                        view! {
                            <p class="problem">{msg}</p>
                            {has_log.then(|| view! {
                                <div><Button size="sm" on_click=move |_| show_log.update(|v| *v = !*v)>
                                    {move || if show_log.get() { t("detect.hideLog") } else { t("detect.showLog") }}
                                </Button></div>
                                {move || show_log.get().then(|| view! { <pre class="log">{store.scan_log.get().join("\n")}</pre> })}
                            })}
                        }
                    })
                }}
            </Card>
        };

        let results = match s {
            Some(s) => {
                let issues: Vec<(String, i64)> = s["issues"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0))).collect())
                    .unwrap_or_default();
                let per_grid = s["per_grid"].as_array().cloned().unwrap_or_default();
                let dep_issues = s["deployment_issues"].as_array().cloned().unwrap_or_default();
                view! {
                    <div class="grid-4">
                        <StatTile value=count(&s["n_files"]) label=t("scan.files") />
                        <StatTile value=count(&s["n_grids"]) label=t("scan.grids") palette="sky" />
                        <StatTile value=count(&s["total_hours"]) label=t("scan.hours") palette="lavender" />
                        <StatTile value=count(&s["n_qc_pass"]) label=t("scan.pass") palette="sage" />
                    </div>
                    <Card title=t("scan.checksTitle") hint=t("scan.checksHint")>
                        <ul class="issues">
                            {issues
                                .into_iter()
                                .map(|(id, n)| {
                                    view! {
                                        <li class:ok=n == 0>
                                            <span class="name">{t(&format!("scan.issues.{id}"))}</span>
                                            <span class="hint">{t(&format!("scan.issueHints.{id}"))}</span>
                                            {if n == 0 {
                                                view! { <Badge palette="mint">{t("scan.none")}</Badge> }.into_any()
                                            } else {
                                                view! { <Badge palette="peach">{locale(n as f64)}</Badge> }.into_any()
                                            }}
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    </Card>
                    {(!per_grid.is_empty()).then(|| view! { <Card title=t("scan.gridsTitle") hint=t("scan.gridsHint")>{table(per_grid)}</Card> })}
                    {(!dep_issues.is_empty()).then(|| view! { <Card title=t("scan.deploymentTitle") hint=t("scan.deploymentHint")>{table(dep_issues)}</Card> })}
                }
                .into_any()
            }
            None if store.scan_status.get() == "ready" => view! { <Card title=t("scan.noneYet") hint=t("scan.noneYetHint")>{()}</Card> }.into_any(),
            None => ().into_any(),
        };
        view! { {run_card}{results} }.into_any()
    };

    view! {
        <PageHeader number=3 name=t("stages.scan") title=t("scan.title") intro=t("scan.intro") />
        {body}
    }
}
