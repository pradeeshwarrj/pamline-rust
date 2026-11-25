//! Change settings in config.yml without losing comments or layout.
//!
//! Port of `bridge/config_edit.R` (pamflow Desktop 0.2.1). Only the lines of
//! the settings being edited change; every comment, blank line and other
//! setting stays exactly as it was. Works on pamflow's two-level layout:
//!
//! ```yaml
//! section:
//!   key: value          # comment kept
//! ```
//!
//! Values come in as `serde_json::Value`, the way the interface sends them
//! in a job's `changes`. An integer JSON number behaves like an R integer, a
//! number with a decimal point like an R double.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// One setting to change: `section: key: value`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Change {
    pub section: String,
    pub key: String,
    #[serde(default)]
    pub value: Value,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("Choose at least one index.")]
    NoIndices,
    #[error("Unknown index: {0}")]
    UnknownIndex(String),
}

// ---- values ------------------------------------------------------------

/// A number the way R's `format(v, scientific = FALSE, digits = 15, trim = TRUE)` writes it.
pub fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    format_f64(n.as_f64().unwrap_or(f64::NAN))
}

fn format_f64(f: f64) -> String {
    if !f.is_finite() {
        return if f.is_nan() { "NaN".into() } else if f > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if f == 0.0 {
        return "0".into();
    }
    // 15 significant digits, then drop trailing zeros
    let magnitude = f.abs().log10().floor() as i32;
    let decimals = (14 - magnitude).clamp(0, 340) as usize;
    let mut s = format!("{:.*}", decimals, f);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => format_number(n),
        Value::String(s) => quote(s),
        // nested values do not occur in pamflow's settings; write them as text
        other => quote(&other.to_string()),
    }
}

fn flatten<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| flatten(x, out)),
        Value::Object(o) => o.values().for_each(|x| flatten(x, out)),
        Value::Null => {}
        x => out.push(x),
    }
}

/// Format a value as a YAML value on one line (`pd_yaml_value`).
pub fn yaml_value(value: &Value) -> String {
    match value {
        Value::Array(_) | Value::Object(_) => {
            let mut items = Vec::new();
            flatten(value, &mut items);
            if items.is_empty() {
                return "[]".into();
            }
            // decimals and whole numbers in one list: write 1 as 1.0, or YAML reads a mixed list
            let all_numbers = items.iter().all(|v| v.is_number());
            if all_numbers && items.len() > 1 {
                let nums: Vec<f64> = items.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect();
                if nums.iter().any(|x| x.is_finite() && x.fract() != 0.0) {
                    // R formats the vector as a whole: every decimal gets the
                    // same number of places ([0.30, 0.55, 1.0])
                    let places = nums
                        .iter()
                        .filter(|x| x.is_finite() && x.fract() != 0.0)
                        .map(|x| format_f64(*x).split_once('.').map_or(0, |(_, d)| d.len()))
                        .max()
                        .unwrap_or(1);
                    let parts: Vec<String> = nums
                        .iter()
                        .map(|x| {
                            if x.fract() == 0.0 {
                                format!("{:.1}", x)
                            } else if x.is_finite() {
                                format!("{:.*}", places, x)
                            } else {
                                format_f64(*x)
                            }
                        })
                        .collect();
                    return format!("[{}]", parts.join(", "));
                }
            }
            // a list mixing text and numbers is all text in R (unlist coerces)
            let any_text = items.iter().any(|v| v.is_string());
            let parts: Vec<String> = items
                .iter()
                .map(|v| if any_text && !v.is_string() { quote(&scalar(v)) } else { scalar(v) })
                .collect();
            format!("[{}]", parts.join(", "))
        }
        v => scalar(v),
    }
}

// ---- line helpers --------------------------------------------------------

/// Number of leading spaces (tabs do not count, as in R).
pub fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// A blank line or a comment line.
pub fn is_blank(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

fn is_whitespace_only(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// The trailing comment of the text after `key:`, or "".
/// A `#` inside quotes is part of the value; a comment `#` follows a space,
/// a tab, or starts the text.
pub fn trailing_comment(rest: &str) -> &str {
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    let (mut in_single, mut in_double) = (false, false);
    for (k, &(pos, ch)) in chars.iter().enumerate() {
        let prev = if k > 0 { Some(chars[k - 1].1) } else { None };
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single && prev != Some('\\') {
            in_double = !in_double;
        } else if ch == '#' && !in_single && !in_double && matches!(prev, None | Some(' ') | Some('\t')) {
            return &rest[pos..];
        }
    }
    ""
}

/// `<indent spaces>name:` followed by the end of line or whitespace.
/// Returns the text after the colon.
fn key_rest<'a>(line: &'a str, base: usize, key: &str) -> Option<&'a str> {
    if indent(line) != base {
        return None;
    }
    let after = line[base..].strip_prefix(key)?.strip_prefix(':')?;
    match after.chars().next() {
        None => Some(after),
        Some(c) if c.is_whitespace() => Some(after),
        _ => None,
    }
}

fn is_section_header(line: &str, section: &str) -> bool {
    match line.strip_prefix(section).and_then(|r| r.strip_prefix(':')) {
        Some(rest) => {
            let t = rest.trim_start();
            t.is_empty() || t.starts_with('#')
        }
        None => false,
    }
}

/// A section's place in the file.
struct Section {
    header: usize,
    /// Last line of the section (inclusive); equal to `header` when empty.
    end: usize,
    /// Lines of the section that are neither blank nor comments.
    content: Vec<usize>,
    /// Indent of the section's keys.
    base: usize,
}

fn find_section(lines: &[String], section: &str) -> Option<Section> {
    let header = lines.iter().position(|l| is_section_header(l, section))?;
    // the section ends before the next line that starts at column 0
    let end = lines[header + 1..]
        .iter()
        .position(|l| !is_blank(l) && indent(l) == 0)
        .map(|k| header + k)
        .unwrap_or(lines.len() - 1);
    let content: Vec<usize> = (header + 1..=end).filter(|&i| !is_blank(&lines[i])).collect();
    let base = content.iter().map(|&i| indent(&lines[i])).min().unwrap_or(2);
    Some(Section { header, end, content, base })
}

fn append_section(lines: &mut Vec<String>, new: impl IntoIterator<Item = String>) {
    if lines.last().is_some_and(|l| !l.is_empty()) {
        lines.push(String::new());
    }
    lines.extend(new);
}

fn spaces(n: usize) -> String {
    " ".repeat(n)
}

// ---- edits ---------------------------------------------------------------

/// Set `section: key:` to `value` (`pd_yaml_set`).
pub fn set(mut lines: Vec<String>, section: &str, key: &str, value: &Value) -> Vec<String> {
    let yv = yaml_value(value);
    let Some(sec) = find_section(&lines, section) else {
        append_section(&mut lines, [format!("{section}:"), format!("  {key}: {yv}")]);
        return lines;
    };
    let base = sec.base;
    let new_line = |comment: &str| {
        let mut s = format!("{}{}: {}", spaces(base), key, yv);
        if !comment.is_empty() {
            s.push_str("  ");
            s.push_str(comment);
        }
        s
    };

    let hit = (sec.header + 1..=sec.end).find(|&i| key_rest(&lines[i], base, key).is_some());
    if let Some(at) = hit {
        let after = key_rest(&lines[at], base, key).unwrap();
        // the R pattern consumes one optional whitespace character after the colon
        let rest = match after.chars().next() {
            Some(c) if c.is_whitespace() => &after[c.len_utf8()..],
            _ => after,
        };
        let comment = trailing_comment(rest).to_string();
        // A value written as an indented block below the key (e.g. a "- item"
        // list) is replaced too. Comment lines are always kept (pamflow's
        // template continues comments on the next lines); a blank line or the
        // next setting ends the old value.
        let item_prefix = format!("{}- ", spaces(base));
        let mut cont = Vec::new();
        let mut i = at + 1;
        while i <= sec.end {
            let l = &lines[i];
            if is_whitespace_only(l) {
                break;
            }
            if is_comment(l) {
                i += 1;
                continue;
            }
            if indent(l) > base || l.starts_with(&item_prefix) {
                cont.push(i);
                i += 1;
                continue;
            }
            break;
        }
        lines[at] = new_line(&comment);
        for &c in cont.iter().rev() {
            lines.remove(c);
        }
        return lines;
    }

    // key not in the section yet: add it after the section's last setting
    let last = sec.content.last().copied().unwrap_or(sec.header);
    lines.insert(last + 1, new_line(""));
    lines
}

/// What pamflow's template gives each soundscape index when it is switched on,
/// in pamflow's order.
pub const INDEX_DEFAULTS: &[(&str, &[&str])] = &[
    ("ACI", &[]),
    ("ADI", &["fmin: 0", "fmax: 24000", "bin_step: 1000"]),
    ("AEve", &[]),
    ("Bio", &["flim: [2000, 11000]"]),
    ("M", &[]),
    ("NDSI", &["anthro_range: [1000, 2000]", "bio_range: [2000, 11000]"]),
];

fn fresh_index(name: &str, at: usize) -> Vec<String> {
    let defaults = INDEX_DEFAULTS.iter().find(|(n, _)| *n == name).map(|(_, d)| *d).unwrap_or(&[]);
    let mut out = vec![format!("{}{}:", spaces(at), name)];
    out.extend(defaults.iter().map(|d| format!("{}{}", spaces(at + 2), d)));
    out
}

fn index_names(value: &Value) -> Vec<String> {
    let mut items = Vec::new();
    flatten(value, &mut items);
    items
        .into_iter()
        .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
        .collect()
}

/// Choose the soundscape indices in `soundscape: acoustic_indices:` (`pd_yaml_set_indices`).
///
/// An index that stays selected keeps its own settings and comments; a newly
/// selected one gets pamflow's default settings; the others are removed.
pub fn set_indices(mut lines: Vec<String>, indices: &[String]) -> Result<Vec<String>, EditError> {
    if indices.is_empty() {
        return Err(EditError::NoIndices);
    }
    let bad: Vec<&str> = indices
        .iter()
        .filter(|i| !INDEX_DEFAULTS.iter().any(|(n, _)| n == i))
        .map(String::as_str)
        .collect();
    if !bad.is_empty() {
        return Err(EditError::UnknownIndex(bad.join(", ")));
    }
    let wanted: Vec<&str> =
        INDEX_DEFAULTS.iter().map(|(n, _)| *n).filter(|n| indices.iter().any(|i| i == n)).collect();

    let Some(sec) = find_section(&lines, "soundscape") else {
        let mut new = vec!["soundscape:".to_string(), "  acoustic_indices:".to_string()];
        new.extend(wanted.iter().flat_map(|n| fresh_index(n, 4)));
        append_section(&mut lines, new);
        return Ok(lines);
    };
    let base = sec.base;
    let Some(at) = (sec.header + 1..=sec.end).find(|&i| key_rest(&lines[i], base, "acoustic_indices").is_some())
    else {
        let last = sec.content.last().copied().unwrap_or(sec.header);
        let mut new = vec![format!("{}acoustic_indices:", spaces(base))];
        new.extend(wanted.iter().flat_map(|n| fresh_index(n, base + 2)));
        lines.splice(last + 1..last + 1, new);
        return Ok(lines);
    };

    // the old entries: deeper-indented lines below the key (blank lines only inside)
    let mut last = at;
    let mut i = at + 1;
    while i <= sec.end {
        let l = &lines[i];
        if !is_blank(l) && indent(l) > base {
            last = i;
        } else if !is_blank(l) || (is_comment(l) && indent(l) <= base) {
            break;
        }
        i += 1;
    }
    let old: Vec<String> = lines[at + 1..=last].to_vec();
    let sub_indent = old.iter().filter(|l| !is_blank(l)).map(|l| indent(l)).min().unwrap_or(base + 2);
    let starts: Vec<usize> = (0..old.len())
        .filter(|&k| {
            let l = &old[k];
            indent(l) == sub_indent && {
                let name: String = l[sub_indent..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                !name.is_empty() && l[sub_indent + name.len()..].starts_with(':')
            }
        })
        .collect();
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let to = starts.get(k + 1).map(|n| n - 1).unwrap_or(old.len() - 1);
        let name: String = old[s].trim_start().chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        // a name listed twice: the later entry wins, as in R
        groups.retain(|(n, _)| *n != name);
        groups.push((name, old[s..=to].to_vec()));
    }
    let nested: Vec<String> = wanted
        .iter()
        .flat_map(|n| match groups.iter().find(|(g, _)| g == n) {
            Some((_, g)) => g.clone(),
            None => fresh_index(n, sub_indent),
        })
        .collect();

    let after = key_rest(&lines[at], base, "acoustic_indices").unwrap();
    let comment = trailing_comment(after.trim_start()).to_string();
    let mut head = format!("{}acoustic_indices:", spaces(base));
    if !comment.is_empty() {
        head.push_str("  ");
        head.push_str(&comment);
    }
    lines[at] = head;
    lines.splice(at + 1..=last, nested);
    Ok(lines)
}

/// Apply several changes in order (`pd_yaml_apply`).
pub fn apply(mut lines: Vec<String>, changes: &[Change]) -> Result<Vec<String>, EditError> {
    for ch in changes {
        lines = if ch.section == "soundscape" && ch.key == "acoustic_indices" {
            set_indices(lines, &index_names(&ch.value))?
        } else {
            set(lines, &ch.section, &ch.key, &ch.value)
        };
    }
    Ok(lines)
}

// ---- files ---------------------------------------------------------------

/// A settings file's lines and whether it used Windows line endings.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigText {
    pub lines: Vec<String>,
    pub crlf: bool,
}

impl ConfigText {
    /// Split file content into lines like R's `readLines` (drops a UTF-8 BOM,
    /// CR before LF, and the final line ending).
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let crlf = text.contains("\r\n");
        let mut lines: Vec<String> =
            text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        Self { lines, crlf }
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(path)?))
    }

    /// The file content: lines joined with the line ending, ending with one.
    pub fn render(lines: &[String], crlf: bool) -> String {
        let eol = if crlf { "\r\n" } else { "\n" };
        let mut s = lines.join(eol);
        s.push_str(eol);
        s
    }
}

/// Normalise a path like R's `normalizePath(mustWork = FALSE)` with forward
/// slashes; the path need not exist.
pub fn norm_path(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.to_string_lossy().replace('\\', "/");
    s.strip_prefix("//?/").map(str::to_string).unwrap_or(s)
}

/// The project folder a settings file at `target` would belong to if it did
/// not pin `project_root`: the folder above `config/`, or else the folder holding it.
pub fn derive_root(target: &Path) -> String {
    let full = PathBuf::from(norm_path(target));
    let dir = full.parent().map(Path::to_path_buf).unwrap_or_default();
    let is_config = dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("config"));
    let root = if is_config { dir.parent().map(Path::to_path_buf).unwrap_or(dir) } else { dir };
    root.to_string_lossy().replace('\\', "/")
}

/// Lines of a new settings file: pamflow's default settings with a short
/// header (`pd_new_config_lines`). If the file will not sit at
/// `<root>/config/` (or in `<root>` itself), `project_root` is pinned to the
/// absolute project folder.
pub fn new_config_lines(template: &[String], root: &Path, target: &Path, version: &str, date: &str) -> Vec<String> {
    let root_s = norm_path(root);
    let pin = derive_root(target).to_lowercase() != root_s.to_lowercase();
    let version = if version.is_empty() { String::new() } else { format!(" {version}") };
    let mut out = vec![
        "# pamflow project settings".to_string(),
        format!("# Created by pamflow Desktop on {date} from the default settings of pamflow{version}."),
        "# Change them in the app (Project step) or in any text editor; the app keeps these comments.".to_string(),
        if pin {
            "# project_root below is the project folder; other folders are relative to it.".to_string()
        } else {
            "# Folders that are not absolute are relative to project_root (\".\" = the project folder).".to_string()
        },
        String::new(),
    ];
    let body = template.to_vec();
    let body = if pin { set(body, "paths", "project_root", &Value::String(root_s)) } else { body };
    out.extend(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_like_r() {
        assert_eq!(yaml_value(&json!(0.3)), "0.3");
        assert_eq!(yaml_value(&json!(0.1 + 0.2)), "0.3");
        assert_eq!(yaml_value(&json!(8)), "8");
        assert_eq!(yaml_value(&json!(24000.0)), "24000");
        assert_eq!(yaml_value(&json!(1.5)), "1.5");
        assert_eq!(yaml_value(&json!(-0.25)), "-0.25");
        assert_eq!(yaml_value(&json!(0.000123)), "0.000123");
    }

    #[test]
    fn lists() {
        assert_eq!(yaml_value(&json!([0.4, 0.6, 1])), "[0.4, 0.6, 1.0]");
        assert_eq!(yaml_value(&json!([800, 300])), "[800, 300]");
        assert_eq!(yaml_value(&json!([])), "[]");
        assert_eq!(yaml_value(&json!(["a"])), "[\"a\"]");
        assert_eq!(yaml_value(&json!(null)), "null");
    }

    #[test]
    fn comments() {
        assert_eq!(trailing_comment("8   # x"), "# x");
        assert_eq!(trailing_comment("\"PAM # Report\"   # y"), "# y");
        assert_eq!(trailing_comment("a#b"), "");
        assert_eq!(trailing_comment("# only"), "# only");
        assert_eq!(trailing_comment("\"a \\\" # b\" # c"), "# c");
    }
}
