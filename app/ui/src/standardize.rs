//! Step 2 · Standardize: triage of the raw recordings (port of
//! `StandardizeLive.svelte`). Settings are saved to config.yml; "Check" is a
//! dry run, "Standardize" writes the files. Raw files are never changed.

use crate::components::{Badge, Button, Card, PageHeader, Pills, StatTile};
use crate::i18n::{t, tv};
use crate::longjob::{is_active, LongJobPanel};
use crate::project::{check_field, field, number_field, slash, text, text_field, thousands, SaveBar};
use crate::state::{check_raw, load_std_state, refresh_now, start_standardize, Store};
use crate::tauri;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{json, Value};

const SAMPLE_RATES: [i64; 6] = [16000, 22050, 24000, 32000, 44100, 48000];
const LAYOUTS: [&str; 5] = ["grid", "grid_date", "date", "preserve", "flat"];

fn n(v: &Value) -> String {
    thousands(v.as_i64().unwrap_or(0))
}

fn gb(x: f64) -> String {
    if x >= 1024.0 {
        format!("{:.1} TB", x / 1024.0)
    } else {
        format!("{x:.1} GB")
    }
}

/// jsonlite writes a one-element vector as a plain string: accept both.
fn problem_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

/// The last two parts of a path ("G0059/file.flac").
fn tail2(p: &str) -> String {
    let parts: Vec<&str> = p.split(['/', '\\']).collect();
    parts[parts.len().saturating_sub(2)..].join("/")
}

fn path_row(store: Store, id: &'static str, key: &'static str, title: String) -> impl IntoView {
    let _ = id;
    let browse_title = title.clone();
    view! {
        <div class="row">
            {text_field(store, "paths", key, "")}
            <Button on_click=move |_| {
                let title = browse_title.clone();
                spawn_local(async move {
                    if let Some(p) = tauri::pick_folder(&title).await {
                        store.set("paths", key, json!(slash(&p)));
                    }
                });
            }>{t("common.browse")}</Button>
        </div>
    }
}

#[component]
pub fn StandardizeScreen(store: Store) -> impl IntoView {
    if !store.folder.get_untracked().is_empty() {
        if store.std_state.get_untracked().is_none() {
            load_std_state(store);
        }
        if store.facts.get_untracked().is_none() && !store.checking.get_untracked() {
            spawn_local(async move { refresh_now(store).await });
        }
    }
    // After saving settings the plan may no longer match: read the state again.
    Effect::new(move |prev: Option<String>| {
        let key = store.facts.with(|f| f.as_ref().map(|f| f["settings"].to_string()).unwrap_or_default());
        if let Some(p) = &prev {
            if !key.is_empty() && *p != key && !store.folder.get_untracked().is_empty() {
                load_std_state(store);
            }
        }
        key
    });

    let free_gb = RwSignal::new(None::<f64>);
    Effect::new(move |_| {
        let dir = store.std_state.with(|s| s.as_ref().map(|s| text(&s["settings"]["standardized_dir"])).unwrap_or_default());
        if !dir.is_empty() {
            spawn_local(async move { free_gb.set(tauri::free_space(&dir).await) });
        }
    });

    let show_problems = RwSignal::new(false);
    let show_log = RwSignal::new(false);

    let st = move || store.std_state.get();
    let plan = move || st().map(|s| s["plan"].clone()).unwrap_or(Value::Null);
    let last = move || st().map(|s| s["last_run"].clone()).unwrap_or(Value::Null);
    let unsaved = move || !store.changed().is_empty();
    let running = move || is_active(store.std_job.status.get());
    let plan_ready = move || plan()["available"].as_bool().unwrap_or(false);
    let can_check = move || {
        st().is_some_and(|s| problem_list(&s["problems"]).is_empty()) && !unsaved() && !store.std_checking.get() && !running()
    };
    let can_run = move || can_check() && plan_ready() && plan()["n_files"].as_i64().unwrap_or(0) > 0;

    // settings pills and selects bound to the draft
    let setting = move |key: &'static str, default: &'static str| {
        Signal::derive(move || match store.get("standardization", key) {
            Value::String(s) if !s.is_empty() => s,
            Value::Number(n) => n.to_string(),
            _ => default.to_string(),
        })
    };
    let format = setting("target_format", "flac");
    let channels = setting("channels", "1");
    let naming = setting("rename_strategy", "auto");
    let layout = setting("organize_folders", "grid");
    let rate = setting("target_hz", "48000");

    let top = move || {
        view! {
            <div class="skip">
                <span>{t("standardize.skip")}</span>
                <Button variant="ghost" size="sm" on_click=move |_| store.screen.set("scan")>{t("standardize.skipButton")}</Button>
            </div>
        }
    };

    let settings_cards = move || {
        if running() {
            return ().into_any();
        }
        let raw_note = match st() {
            Some(s) if !s["raw_found"].as_bool().unwrap_or(false) => Some(t("standardize.rawMissing")),
            Some(s) if !s["n_raw_files"].is_null() => Some(tv("standardize.rawCount", &[("n", n(&s["n_raw_files"]))])),
            _ => None,
        };
        view! {
            <Card title=t("standardize.foldersTitle") hint=t("standardize.foldersHint")>
                <span class="sub">{t("standardize.raw")}</span>
                {path_row(store, "s-raw", "audio_raw", t("standardize.raw"))}
                {raw_note.map(|m| view! { <span class="muted">{m}</span> })}
                <div class="pair">
                    <div class="field">
                        <span class="sub">{t("standardize.out")}</span>
                        {path_row(store, "s-std", "audio_standardized", t("standardize.out"))}
                    </div>
                    <div class="field">
                        <span class="sub">{t("standardize.isolated")}</span>
                        {path_row(store, "s-iso", "audio_isolated", t("standardize.isolated"))}
                    </div>
                </div>
            </Card>

            <div class="grid-2">
                <Card title=t("standardize.audioTitle") hint=t("standardize.audioHint")>
                    <span class="sub">{t("standardize.format")}</span>
                    <Pills options=vec![("flac".to_string(), t("standardize.flac")), ("wav".to_string(), "WAV".to_string())]
                        value=format on_change=move |v: String| store.set("standardization", "target_format", json!(v)) />
                    <span class="sub">{t("standardize.rate")}</span>
                    <select class="input" on:change=move |ev| store.set("standardization", "target_hz", json!(event_target_value(&ev).parse::<i64>().unwrap_or(48000)))>
                        {SAMPLE_RATES
                            .iter()
                            .map(|r| {
                                let r = *r;
                                view! { <option value=r.to_string() selected=move || rate.get() == r.to_string()>{format!("{} kHz", r as f64 / 1000.0)}</option> }
                            })
                            .collect_view()}
                    </select>
                    <span class="muted">{t("standardize.rateHint")}</span>
                    <span class="sub">{t("standardize.channels")}</span>
                    <Pills options=vec![("1".to_string(), t("standardize.mono")), ("2".to_string(), t("standardize.stereo"))]
                        value=channels on_change=move |v: String| store.set("standardization", "channels", json!(v.parse::<i64>().unwrap_or(1))) />
                </Card>

                <Card title=t("standardize.triageTitle") hint=t("standardize.triageHint")>
                    <div class="pair">
                        {field(t("standardize.minDuration"), number_field(store, "standardization", "min_duration_s", "0", "", "10", false))}
                        {field(t("standardize.layout"), view! {
                            <select class="input" on:change=move |ev| store.set("standardization", "organize_folders", json!(event_target_value(&ev)))>
                                {LAYOUTS
                                    .iter()
                                    .map(|l| {
                                        let l = *l;
                                        view! { <option value=l selected=move || layout.get() == l>{t(&format!("standardize.layouts.{l}"))}</option> }
                                    })
                                    .collect_view()}
                            </select>
                        })}
                    </div>
                    <span class="sub">{t("standardize.naming")}</span>
                    <Pills options=vec![("auto".to_string(), t("standardize.namingAuto")), ("keep".to_string(), t("standardize.namingKeep"))]
                        value=naming on_change=move |v: String| store.set("standardization", "rename_strategy", json!(v)) />
                    <span class="sub">{t("standardize.prefix")}</span>
                    {text_field(store, "standardization", "prefix", "")}
                    <span class="muted">{t("standardize.prefixHint")}</span>
                    {check_field(store, "standardization", "isolate_short", t("standardize.isolateShort"), true)}
                    {check_field(store, "standardization", "isolate_non_audio", t("standardize.isolateNonAudio"), true)}
                </Card>
            </div>

            <SaveBar store=store />
        }
        .into_any()
    };

    let problems = move || {
        st().map(|s| problem_list(&s["problems"]))
            .unwrap_or_default()
            .into_iter()
            .map(|p| view! { <p class="problem">{p}</p> })
            .collect_view()
    };

    let check_card = move || {
        if running() {
            return ().into_any();
        }
        let p = plan();
        let ready = p["available"].as_bool().unwrap_or(false);
        view! {
            <Card title=t("standardize.checkTitle") hint=t("standardize.checkHint")>
                <div class="actions">
                    <Button variant=if ready { "secondary" } else { "primary" } size="lg" disabled=Signal::derive(move || !can_check())
                        on_click=move |_| check_raw(store)>
                        {move || if store.std_checking.get() { t("standardize.checking") } else if plan_ready() { t("standardize.checkAgain") } else { t("standardize.check") }}
                    </Button>
                    {ready.then(|| view! { <span class="muted">{tv("standardize.checked", &[("when", text(&p["updated"]))])}</span> })}
                    {move || unsaved().then(|| view! { <Badge palette="peach">{t("standardize.saveFirst")}</Badge> })}
                </div>
                {move || store.std_checking.get().then(|| {
                    let total = store.std_prep_total.get();
                    let pct = if total > 0 { store.std_prep_done.get() as f64 / total as f64 * 100.0 } else { 0.0 };
                    view! {
                        <div class="bar"><div class="fill" style=format!("width: {}%", pct.clamp(0.0, 100.0))></div></div>
                        <span class="muted">{tv("standardize.readCount", &[("done", thousands(store.std_prep_done.get())), ("total", thousands(total))])}</span>
                    }
                })}
                {move || {
                    let m = store.std_message.get();
                    (!m.is_empty()).then(|| view! {
                        <p class="problem">{m}</p>
                        {(!store.std_log.get().is_empty()).then(|| view! {
                            <div><Button size="sm" on_click=move |_| show_log.update(|v| *v = !*v)>
                                {move || if show_log.get() { t("detect.hideLog") } else { t("detect.showLog") }}
                            </Button></div>
                            {move || show_log.get().then(|| view! { <pre class="log">{store.std_log.get().join("\n")}</pre> })}
                        })}
                    })
                }}
            </Card>
        }
        .into_any()
    };

    let plan_cards = move || {
        if running() || !plan_ready() {
            return ().into_any();
        }
        let p = plan();
        let examples: Vec<Value> = p["examples"].as_array().cloned().unwrap_or_default();
        let per_grid: Vec<Value> = p["per_grid"].as_array().cloned().unwrap_or_default();
        let problems: Vec<Value> = p["problems"].as_array().cloned().unwrap_or_default();
        let n_problems = problems.len();
        let std_dir = st().map(|s| text(&s["settings"]["standardized_dir"])).unwrap_or_default();
        let raw_gb = p["raw_gb"].as_f64().unwrap_or(0.0);
        let too_small = move || free_gb.get().is_some_and(|f| f < raw_gb);
        view! {
            <div class="grid-4">
                <StatTile value=n(&p["n_recordings"]) label=t("standardize.recordings") palette="mint" />
                <StatTile value=n(&p["n_memos"]) label=t("standardize.memos") palette="butter" />
                <StatTile value=n(&p["n_non_audio"]) label=t("standardize.nonAudio") palette="lavender" />
                <StatTile value=n(&p["n_corrupt"]) label=t("standardize.corrupt") palette="peach" />
            </div>

            <Card title=t("standardize.planTitle") hint=tv("standardize.planHint", &[("convert", n(&p["n_convert"])), ("copy", n(&p["n_copy"]))])>
                {(!examples.is_empty()).then(|| view! {
                    <div class="table-wrap">
                        <table>
                            <thead><tr><th>{t("standardize.from")}</th><th></th><th>{t("standardize.to")}</th></tr></thead>
                            <tbody>
                                {examples
                                    .iter()
                                    .map(|e| view! { <tr><td class="mono">{text(&e["from"])}</td><td class="arrow">"→"</td><td class="mono">{tail2(&text(&e["to"]))}</td></tr> })
                                    .collect_view()}
                            </tbody>
                        </table>
                    </div>
                })}
                {(!per_grid.is_empty()).then(|| view! {
                    <div class="chips">
                        {per_grid.iter().map(|g| view! { <Badge>{format!("{} · {}", text(&g["grid"]), n(&g["files"]))}</Badge> }).collect_view()}
                    </div>
                })}
                {(n_problems > 0).then(|| view! {
                    <div><Button size="sm" on_click=move |_| show_problems.update(|v| *v = !*v)>
                        {move || if show_problems.get() { t("standardize.hideSetAside") } else { tv("standardize.showSetAside", &[("n", thousands(n_problems as i64))]) }}
                    </Button></div>
                    {move || show_problems.get().then(|| view! {
                        <div class="table-wrap">
                            <table>
                                <thead><tr><th>{t("standardize.file")}</th><th>{t("standardize.what")}</th><th>{t("standardize.why")}</th></tr></thead>
                                <tbody>
                                    {problems
                                        .iter()
                                        .map(|r| view! { <tr><td class="mono">{text(&r["file"])}</td><td>{t(&format!("standardize.categories.{}", text(&r["category"])))}</td><td>{text(&r["message"])}</td></tr> })
                                        .collect_view()}
                                </tbody>
                            </table>
                        </div>
                    })}
                })}
            </Card>

            <Card title=t("standardize.runTitle") hint=t("standardize.runHint")>
                <dl>
                    <dt>{t("filter.space")}</dt>
                    <dd>
                        {tv("standardize.needs", &[("size", gb(raw_gb))])}
                        {move || free_gb.get().map(|f| format!(" · {}", tv("filter.free", &[("size", gb(f))])))}
                    </dd>
                    <dt>{t("standardize.out")}</dt><dd class="mono">{std_dir}</dd>
                </dl>
                {move || too_small().then(|| view! { <Badge palette="peach">{t("standardize.notEnoughSpace")}</Badge> })}
                <div class="actions">
                    <Button variant="primary" size="lg" disabled=Signal::derive(move || !can_run()) on_click=move |_| start_standardize(store)>
                        "▶ "{tv("standardize.start", &[("n", n(&p["n_recordings"]))])}
                    </Button>
                </div>
                <span class="muted">{t("standardize.noResume")}</span>
            </Card>
        }
        .into_any()
    };

    let last_card = move || {
        let l = last();
        if running() || !l["available"].as_bool().unwrap_or(false) {
            return ().into_any();
        }
        let isolated = l["n_memos"].as_i64().unwrap_or(0) + l["n_non_audio"].as_i64().unwrap_or(0) + l["n_corrupt"].as_i64().unwrap_or(0);
        let summary = tv("standardize.lastSummary", &[("rec", n(&l["n_recordings"])), ("iso", thousands(isolated)), ("failed", n(&l["n_failed"]))]);
        let hint = tv("standardize.lastHint", &[("when", text(&l["updated"]))]);
        let (out_dir, iso_dir) = st().map(|s| (text(&s["settings"]["standardized_dir"]), text(&s["settings"]["isolated_dir"]))).unwrap_or_default();
        view! {
            <Card title=t("standardize.lastTitle") hint=hint>
                <p class="summary">{summary}</p>
                <div class="actions">
                    <Button on_click=move |_| { let d = out_dir.clone(); spawn_local(async move { tauri::show_folder(&d).await }) }>{t("standardize.openOut")}</Button>
                    <Button on_click=move |_| { let d = iso_dir.clone(); spawn_local(async move { tauri::show_folder(&d).await }) }>{t("standardize.openIsolated")}</Button>
                    <Button variant="primary" on_click=move |_| store.screen.set("scan")>{t("standardize.next")}</Button>
                </div>
            </Card>
        }
        .into_any()
    };

    let body = move || {
        if store.folder.get().is_empty() {
            return view! {
                <Card title=t("detect.chooseProject")>
                    <div><Button variant="primary" on_click=move |_| store.screen.set("project")>{t("detect.goProject")}</Button></div>
                </Card>
            }
            .into_any();
        }
        view! {
            {top()}
            {settings_cards}
            {problems}
            {check_card}
            {plan_cards}
            {move || (store.std_job.status.get() != "idle").then(|| view! { <LongJobPanel job=store.std_job kind="standardize" /> })}
            {last_card}
        }
        .into_any()
    };

    view! {
        <PageHeader number=2 name=t("stages.standardize") title=t("standardize.title") intro=t("standardize.intro") />
        {body}
    }
}
