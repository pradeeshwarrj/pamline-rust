//! Step 4 · Filter: high-pass filter the recordings with ffmpeg (port of
//! `FilterLive.svelte`). The run uses the live job panel.

use crate::components::{Badge, Button, Card, PageHeader, Pills};
use crate::i18n::{t, tv};
use crate::longjob::{is_active, LongJobPanel};
use crate::project::{text, thousands};
use crate::state::{load_filter_info, refresh_now, start_filter, Store};
use crate::tauri;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;

fn gb(x: f64) -> String {
    if x >= 1024.0 {
        format!("{:.1} TB", x / 1024.0)
    } else {
        format!("{x:.1} GB")
    }
}

#[component]
pub fn FilterScreen(store: Store) -> impl IntoView {
    if !store.folder.get_untracked().is_empty() {
        load_filter_info(store);
        if store.facts.get_untracked().is_none() && !store.checking.get_untracked() {
            spawn_local(async move { refresh_now(store).await });
        }
    }
    let info = move || store.filter_info.get();
    let free_gb = RwSignal::new(None::<f64>);
    Effect::new(move |_| {
        let dir = info().map(|s| text(&s["output_folder"])).unwrap_or_default();
        if !dir.is_empty() {
            spawn_local(async move { free_gb.set(tauri::free_space(&dir).await) });
        }
    });

    // worker choices: a few sizes up to one fewer than the physical cores
    let physical = move || store.fact("cores_physical").as_i64().unwrap_or(4).max(1);
    let choices = move || {
        let p = physical();
        let mut c: Vec<i64> = [2, 4, 8, 12, (p - 2).max(1)].into_iter().filter(|n| *n >= 1 && *n <= (p - 1).max(1)).collect();
        c.sort();
        c.dedup();
        c
    };
    let workers = RwSignal::new(0i64);
    Effect::new(move |_| {
        if workers.get_untracked() == 0 && store.facts.get().is_some() {
            if let Some(s) = info() {
                let want = s["workers"].as_i64().unwrap_or(4);
                if let Some(best) = choices().into_iter().min_by_key(|n| (n - want).abs()) {
                    workers.set(best);
                }
            }
        }
    });

    let active = move || is_active(store.filter_job.status.get());
    let need_gb = move || info().and_then(|s| s["input_gb"].as_f64());
    let too_small = move || matches!((free_gb.get(), need_gb()), (Some(f), Some(n)) if f < n);
    let start_label = move || {
        let done = info().and_then(|s| s["already_done"].as_i64()).unwrap_or(0);
        if store.filter_job.status.get() == "stopped" || done > 0 { t("detect.continue") } else { t("filter.start") }
    };
    let enabled = move || info().is_some_and(|s| s["enabled"].as_bool().unwrap_or(false));

    let settings_card = move || {
        if active() {
            return ().into_any();
        }
        let s = info().unwrap_or(Value::Null);
        let done = s["already_done"].as_i64().unwrap_or(0);
        let filter_text = if s["enabled"].as_bool().unwrap_or(false) {
            tv("filter.highpass", &[("hz", text(&s["highpass_hz"])), ("order", text(&s["order"]))])
        } else {
            t("filter.disabled")
        };
        let options: Vec<(String, String)> = choices().into_iter().map(|n| (n.to_string(), n.to_string())).collect();
        view! {
            <Card title=t("filter.settingsTitle") hint=t("filter.settingsHint")>
                <dl>
                    <dt>{t("filter.filter")}</dt>
                    <dd>{filter_text}</dd>
                    <dt>{t("filter.output")}</dt><dd class="mono">{text(&s["output_folder"])}</dd>
                    <dt>{t("filter.space")}</dt>
                    <dd>
                        {match need_gb() { Some(n) => tv("filter.needs", &[("size", gb(n))]), None => "–".to_string() }}
                        {move || free_gb.get().map(|f| format!(" · {}", tv("filter.free", &[("size", gb(f))])))}
                    </dd>
                    {(done > 0).then(|| view! { <dt>{t("filter.done")}</dt><dd>{tv("detect.alreadyDone", &[("n", thousands(done))])}</dd> })}
                </dl>
                {move || too_small().then(|| view! { <Badge palette="peach">{t("filter.notEnoughSpace")}</Badge> })}
                <div class="field">
                    <span class="label">{t("project.workers")}</span>
                    <Pills options=options value=Signal::derive(move || workers.get().to_string())
                        on_change=move |v: String| workers.set(v.parse().unwrap_or(4)) />
                    <span class="muted">{t("filter.workersHint")}</span>
                </div>
                <div class="actions">
                    <Button variant="primary" size="lg" disabled=Signal::derive(move || !enabled())
                        on_click=move |_| start_filter(store, workers.get_untracked())>
                        "▶ "{start_label}
                    </Button>
                    <Button variant="ghost" on_click=move |_| store.screen.set("project")>{t("filter.changeSettings")}</Button>
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
        if info().is_none() {
            return view! { <p class="muted">{t("report.loading")}</p> }.into_any();
        }
        view! {
            {settings_card}
            {move || (store.filter_job.status.get() != "idle").then(|| view! { <LongJobPanel job=store.filter_job kind="filter" /> })}
        }
        .into_any()
    };

    view! {
        <PageHeader number=4 name=t("stages.filter") title=t("filter.title") intro=t("filter.intro") />
        {body}
    }
}
