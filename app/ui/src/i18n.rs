//! All on-screen text lives in `en.json` (shared with the old app). Screens
//! never contain text directly; they call `t("section.key")`, or
//! `tv("section.key", &[("n", "63".into())])` with `{n}` placeholders.

use serde_json::Value;
use std::sync::OnceLock;

fn dictionary() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(include_str!("../en.json")).unwrap_or(Value::Null))
}

/// The text for a key; a missing key shows the key itself, so it is easy to spot.
pub fn t(key: &str) -> String {
    let mut node = dictionary();
    for part in key.split('.') {
        node = &node[part];
    }
    node.as_str().map(str::to_string).unwrap_or_else(|| key.to_string())
}

/// The text with `{name}` placeholders filled in.
pub fn tv(key: &str, vars: &[(&str, String)]) -> String {
    let mut s = t(key);
    for (name, value) in vars {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}
