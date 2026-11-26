//! pamflow Desktop, interface in Rust (Leptos, compiled to WebAssembly and run
//! by the Tauri shell in `../src-tauri`). The old Svelte app is being ported
//! screen by screen; this is the Project screen first.

mod components;
mod i18n;
mod longjob;
mod project;
mod scan;
mod standardize;
mod state;
mod tauri;

use components::accent_style;
use i18n::t;
use leptos::mount::mount_to_body;
use leptos::prelude::*;
use state::Store;

/// The steps whose screens are ported; the others show "coming soon".
const READY: [&str; 3] = ["project", "standardize", "scan"];

/// The workflow A steps in order, with their pastel (see `config/stages.ts`).
const STEPS: [(&str, &str); 9] = [
    ("project", "sky"),
    ("standardize", "butter"),
    ("scan", "mint"),
    ("filter", "aqua"),
    ("detect", "lavender"),
    ("process", "peach"),
    ("validate", "rose"),
    ("occupancy", "butter"),
    ("report", "sage"),
];

const THEME_KEY: &str = "pamflow-theme";

/// Light / dark: follows the computer's own setting until the user flips the switch,
/// then the choice is remembered.
fn initial_dark() -> bool {
    let window = web_sys::window();
    let saved = window.as_ref().and_then(|w| w.local_storage().ok().flatten()).and_then(|s| s.get_item(THEME_KEY).ok().flatten());
    match saved.as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => window
            .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
            .map(|m| m.matches())
            .unwrap_or(false),
    }
}

#[component]
fn ThemeSwitch(dark: RwSignal<bool>) -> impl IntoView {
    view! {
        <button type="button" class="switch" role="switch" aria-checked=move || dark.get().to_string()
            on:click=move |_| {
                dark.update(|d| *d = !*d);
                if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
                    let _ = s.set_item(THEME_KEY, if dark.get_untracked() { "dark" } else { "light" });
                }
            }>
            <span class="label">{move || if dark.get() { t("common.darkMode") } else { t("common.lightMode") }}</span>
            <span class="track" class:on=move || dark.get()><span class="knob"></span></span>
        </button>
    }
}

#[component]
fn Sidebar(store: Store, dark: RwSignal<bool>) -> impl IntoView {
    view! {
        <nav class="sidebar">
            <div class="brand"><span class="logo">"p"</span><span class="name">{t("app.name")}</span></div>
            <span class="side-label">{t("common.workflowSteps")}</span>
            <ol class="steps">
                {STEPS
                    .iter()
                    .enumerate()
                    .map(|(i, (id, palette))| {
                        let id: &'static str = id;
                        let ready = READY.contains(&id);
                        view! {
                            <li>
                                <button type="button" class="step" class:active=move || store.screen.get() == id class:soon=!ready
                                    style=accent_style(palette)
                                    on:click=move |_| { if ready { store.screen.set(id) } }>
                                    <span class="num">{i + 1}</span>
                                    <span>{t(&format!("stages.{id}"))}</span>
                                </button>
                            </li>
                        }
                    })
                    .collect_view()}
            </ol>
            <div class="side-foot"><ThemeSwitch dark=dark /></div>
        </nav>
    }
}

#[component]
fn App() -> impl IntoView {
    let store = Store::new();
    let dark = RwSignal::new(initial_dark());
    Effect::new(move |_| {
        if let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
            let _ = root.set_attribute("data-theme", if dark.get() { "dark" } else { "light" });
        }
    });
    view! {
        <div class="app">
            <Sidebar store=store dark=dark />
            <main style=move || accent_style(STEPS.iter().find(|(id, _)| *id == store.screen.get()).map(|(_, p)| *p).unwrap_or("sky"))>
                <div class="content">
                    {move || {
                        if !tauri::in_app() {
                            return view! { <p class="muted">"Open this page through the pamflow app (cargo tauri dev)."</p> }.into_any();
                        }
                        match store.screen.get() {
                            "project" => view! { <project::ProjectScreen store=store /> }.into_any(),
                            "standardize" => view! { <standardize::StandardizeScreen store=store /> }.into_any(),
                            "scan" => view! { <scan::ScanScreen store=store /> }.into_any(),
                            _ => view! { <components::Card title=t("common.notBuiltTitle")><p class="muted">{t("common.notBuiltBody")}</p></components::Card> }.into_any(),
                        }
                    }}
                </div>
            </main>
        </div>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
