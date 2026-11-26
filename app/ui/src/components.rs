//! Shared building blocks (port of `src/lib/components/*.svelte`): the same
//! class names as the old app, styled in `assets/components.css`.

use crate::i18n::tv;
use leptos::prelude::*;

/// CSS that makes a block use a step's pastel as its accent.
pub fn accent_style(palette: &str) -> String {
    format!("--accent-tint: var(--{palette}-tint); --accent-ink: var(--{palette}-ink);")
}

/// A white (or dark) card with an optional title and one-line hint.
#[component]
pub fn Card(#[prop(into, optional)] title: String, #[prop(into, optional)] hint: String, children: Children) -> impl IntoView {
    let head = (!title.is_empty() || !hint.is_empty()).then(|| {
        view! {
            <div class="head">
                <div class="titles">
                    {(!title.is_empty()).then(|| view! { <span class="title">{title.clone()}</span> })}
                    {(!hint.is_empty()).then(|| view! { <span class="hint">{hint.clone()}</span> })}
                </div>
            </div>
        }
    });
    view! { <section class="card">{head}{children()}</section> }
}

/// One button style for the whole app: primary | secondary | ghost | accent.
#[component]
pub fn Button(
    #[prop(default = "secondary")] variant: &'static str,
    #[prop(default = "md")] size: &'static str,
    #[prop(default = Signal::stored(false), into)] disabled: Signal<bool>,
    #[prop(into)] on_click: Callback<()>,
    children: Children,
) -> impl IntoView {
    view! {
        <button type="button" class=format!("btn {variant} {size}") disabled=move || disabled.get() on:click=move |_| on_click.run(())>
            {children()}
        </button>
    }
}

/// A big number with a label, in a step's pastel.
#[component]
pub fn StatTile(#[prop(into)] value: String, #[prop(into)] label: String, #[prop(optional)] palette: Option<&'static str>) -> impl IntoView {
    view! {
        <div class="tile" style=palette.map(accent_style)>
            <div class="value">{value}</div>
            <div class="label">{label}</div>
        </div>
    }
}

/// Small coloured label; neutral grey with no palette.
#[component]
pub fn Badge(#[prop(optional)] palette: Option<&'static str>, children: Children) -> impl IntoView {
    view! { <span class=if palette.is_some() { "badge" } else { "badge neutral" } style=palette.map(accent_style)>{children()}</span> }
}

/// Pick one option from a few, shown as a segmented pill row.
#[component]
pub fn Pills(options: Vec<(String, String)>, #[prop(into)] value: Signal<String>, #[prop(into)] on_change: Callback<String>) -> impl IntoView {
    view! {
        <div class="pills" role="radiogroup">
            {options
                .into_iter()
                .map(|(v, label)| {
                    let v2 = v.clone();
                    view! {
                        <button type="button" role="radio" class:on=move || value.get() == v on:click=move |_| on_change.run(v2.clone())>
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Top of every screen: step chip, title, optional intro.
#[component]
pub fn PageHeader(number: usize, #[prop(into)] name: String, #[prop(into)] title: String, #[prop(into)] intro: String) -> impl IntoView {
    view! {
        <header class="page-header">
            <div class="text">
                <span class="chip">{tv("common.step", &[("n", number.to_string()), ("name", name)])}</span>
                <h1>{title}</h1>
                <p>{intro}</p>
            </div>
        </header>
    }
}
