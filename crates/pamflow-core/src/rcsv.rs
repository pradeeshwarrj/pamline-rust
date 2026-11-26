//! CSV the way pamflow's R code writes and reads it (data.table `fwrite` /
//! `fread`), so files are byte-identical and events carry the same types.

use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

/// fwrite's line ending: CRLF on Windows.
pub const EOL: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// One cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Na,
    Str(String),
    Int(i64),
    Num(f64),
    Bool(bool),
    /// POSIXct: seconds since 1970 (UTC).
    Time(f64),
}

impl Cell {
    pub fn opt_str(v: Option<&str>) -> Cell {
        v.map(|s| Cell::Str(s.to_string())).unwrap_or(Cell::Na)
    }
    pub fn opt_num(v: Option<f64>) -> Cell {
        match v {
            Some(x) if !x.is_nan() => Cell::Num(x),
            _ => Cell::Na,
        }
    }
    pub fn opt_int(v: Option<i64>) -> Cell {
        v.map(Cell::Int).unwrap_or(Cell::Na)
    }
    pub fn opt_bool(v: Option<bool>) -> Cell {
        v.map(Cell::Bool).unwrap_or(Cell::Na)
    }
    pub fn opt_time(v: Option<f64>) -> Cell {
        match v {
            Some(x) if x.is_finite() => Cell::Time(x),
            _ => Cell::Na,
        }
    }

    /// The text fwrite writes for this cell.
    pub fn render(&self) -> String {
        match self {
            Cell::Na => String::new(),
            // fwrite quotes an empty string, to tell it from NA (written as nothing)
            Cell::Str(s) if s.is_empty() => "\"\"".into(),
            Cell::Str(s) => quote_auto(s),
            Cell::Int(i) => i.to_string(),
            Cell::Num(x) => crate::rnum::fwrite_num(*x),
            Cell::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
            Cell::Time(t) => fmt_time(*t),
        }
    }
}

/// fwrite's `quote = "auto"`: quote a field holding the separator, a quote
/// or a line break; quotes inside are doubled.
pub fn quote_auto(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// A double as fwrite writes it (see [`crate::rnum::fwrite_num`]).
pub fn fmt_num(x: f64) -> String {
    crate::rnum::fwrite_num(x)
}

/// POSIXct as fwrite writes it: UTC ISO 8601 with `Z`; a fraction of a second
/// as 3 digits when the microseconds are whole milliseconds, else 6.
pub fn fmt_time(t: f64) -> String {
    if !t.is_finite() {
        return String::new();
    }
    let xi = t.floor();
    let mut m = ((t - xi) * 10_000_000.0) as i64; // 7th digit decides
    m += m % 10;
    m /= 10;
    let dt = chrono::DateTime::from_timestamp(xi as i64, 0).unwrap_or_default();
    let mut s = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
    if m != 0 && m % 1000 == 0 {
        s.push_str(&format!(".{:03}", m / 1000));
    } else if m != 0 {
        s.push_str(&format!(".{:06}", m));
    }
    s.push('Z');
    s
}

/// One row of rendered cells.
pub fn row(cells: &[Cell]) -> String {
    let mut s = cells.iter().map(Cell::render).collect::<Vec<_>>().join(",");
    s.push_str(EOL);
    s
}

/// Write a table (header + rows), replacing the file.
pub fn write_table(path: &Path, header: &[&str], rows: &[Vec<Cell>]) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str(&header.join(","));
    out.push_str(EOL);
    for r in rows {
        out.push_str(&row(r));
    }
    std::fs::write(path, out)
}

/// Append rows; the header is written only when the file is new (fwrite append).
pub fn append_table(path: &Path, header: &[&str], rows: &[Vec<Cell>]) -> std::io::Result<()> {
    let new = !path.exists();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let mut out = String::new();
    if new {
        out.push_str(&header.join(","));
        out.push_str(EOL);
    }
    for r in rows {
        out.push_str(&row(r));
    }
    f.write_all(out.as_bytes())
}

// ---- reading ---------------------------------------------------------------------

/// A CSV read as text: header names and rows of fields (missing fields "").
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub names: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn read(path: &Path) -> Option<Table> {
        let mut r = csv::ReaderBuilder::new().flexible(true).from_path(path).ok()?;
        let names: Vec<String> = r.headers().ok()?.iter().map(str::to_string).collect();
        let rows = r
            .records()
            .flatten()
            .filter(|rec| !(rec.len() == 1 && rec[0].trim().is_empty()))
            .map(|rec| {
                let mut v: Vec<String> = rec.iter().map(str::to_string).collect();
                v.resize(names.len(), String::new());
                v
            })
            .collect();
        Some(Table { names, rows })
    }

    pub fn col(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// The column's fields, or None when there is no such column.
    pub fn column(&self, name: &str) -> Option<Vec<&str>> {
        let i = self.col(name)?;
        Some(self.rows.iter().map(|r| r[i].as_str()).collect())
    }

    /// The column as fread would type it, as JSON values (jsonlite).
    pub fn json_column(&self, i: usize) -> Vec<Value> {
        let vals: Vec<&str> = self.rows.iter().map(|r| r[i].as_str()).collect();
        let present: Vec<&str> = vals.iter().copied().filter(|v| !v.is_empty() && *v != "NA").collect();
        let is_bool = !present.is_empty() && present.iter().all(|v| matches!(*v, "TRUE" | "FALSE" | "true" | "false"));
        let is_int = !present.is_empty() && present.iter().all(|v| v.parse::<i64>().is_ok());
        let is_num = !present.is_empty() && present.iter().all(|v| v.parse::<f64>().is_ok());
        let is_time = !present.is_empty() && present.iter().all(|v| parse_iso(v).is_some());
        vals.iter()
            .map(|v| {
                let na = v.is_empty() || *v == "NA";
                if present.is_empty() {
                    // fread: an all-empty column is logical NA (a text column keeps "")
                    return Value::Null;
                }
                if is_bool {
                    return if na { Value::Null } else { json!(v.eq_ignore_ascii_case("true")) };
                }
                if is_int {
                    return if na { Value::Null } else { json!(v.parse::<i64>().unwrap()) };
                }
                if is_num {
                    return if na { Value::Null } else { json!(v.parse::<f64>().unwrap()) };
                }
                if is_time {
                    return if na {
                        Value::Null
                    } else {
                        json!(parse_iso(v).map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string()))
                    };
                }
                json!(v) // text: empty stays ""
            })
            .collect()
    }

    /// Rows as JSON objects (jsonlite `dataframe = "rows"`, NA as null).
    pub fn json_rows(&self, limit: usize) -> Value {
        let cols: Vec<Vec<Value>> = (0..self.names.len()).map(|i| self.json_column(i)).collect();
        Value::Array(
            (0..self.rows.len().min(limit))
                .map(|r| {
                    Value::Object(self.names.iter().enumerate().map(|(i, n)| (n.clone(), cols[i][r].clone())).collect())
                })
                .collect(),
        )
    }
}

fn parse_iso(v: &str) -> Option<chrono::NaiveDateTime> {
    let v = v.strip_suffix('Z')?;
    chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%.f").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_fwrite() {
        assert_eq!(fmt_num(3599.701333333333), "3599.70133333333");
        assert_eq!(fmt_num(0.298666715621948), "0.298666715621948");
        assert_eq!(fmt_num(-268_425.454_999_923_7), "-268425.454999924");
        assert_eq!(fmt_num(0.0077083333333333335), "0.00770833333333333");
        assert_eq!(fmt_num(345571372.0), "345571372");
        assert_eq!(fmt_num(76.7), "76.7");
        assert_eq!(fmt_num(10.0), "10");
        assert_eq!(fmt_num(1e-7), "1e-07");
        assert_eq!(fmt_num(1.5e20), "1.5e+20");
    }

    #[test]
    fn times_like_fwrite() {
        assert_eq!(fmt_time(1775349000.0), "2026-04-05T00:30:00Z");
        assert_eq!(fmt_time(1775700000.0 + 285.455), "2026-04-09T02:04:45.455Z");
        assert_eq!(fmt_time(1776223649.0 + 3599.701333333333), "2026-04-15T04:27:28.701333Z");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_auto("a,b"), "\"a,b\"");
        assert_eq!(quote_auto("say \"hi\", ok"), "\"say \"\"hi\"\", ok\"");
        assert_eq!(quote_auto("G  NA"), "G  NA");
    }
}
