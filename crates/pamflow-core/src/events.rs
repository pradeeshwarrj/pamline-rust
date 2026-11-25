//! Job events: one JSON object per line on stdout, prefixed with `@@PAM `.
//!
//! Same contract as `bridge/protocol.R` (`pd_emit`), so the existing
//! interface can read a Rust job exactly like an R one. Every job ends with
//! `{"type":"finished","status":"ok"|"failed","message":...}`.

use serde_json::{Map, Value};
use std::io::Write;

pub const PREFIX: &str = "@@PAM ";

/// The text of one event line (without the line ending).
pub fn line(kind: &str, fields: Value) -> String {
    let mut obj = Map::new();
    obj.insert("type".into(), Value::String(kind.into()));
    if let Value::Object(f) = fields {
        for (k, v) in f {
            if k != "type" {
                obj.insert(k, v);
            }
        }
    }
    format!("{PREFIX}{}", Value::Object(obj))
}

/// Write one event to `out` and flush, so the interface sees it at once.
pub fn emit_to(out: &mut impl Write, kind: &str, fields: Value) -> std::io::Result<()> {
    writeln!(out, "{}", line(kind, fields))?;
    out.flush()
}

/// Write one event to stdout. A closed stdout (the app went away) is ignored.
pub fn emit(kind: &str, fields: Value) {
    let _ = emit_to(&mut std::io::stdout().lock(), kind, fields);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_line() {
        assert_eq!(line("total", json!({"n": 63})), r#"@@PAM {"type":"total","n":63}"#);
        assert_eq!(
            line("finished", json!({"status": "ok", "message": null})),
            r#"@@PAM {"type":"finished","status":"ok","message":null}"#
        );
    }
}
