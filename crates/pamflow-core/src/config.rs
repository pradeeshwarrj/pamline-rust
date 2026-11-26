//! Reading a pamflow settings file (port of `pam_config_read` and
//! `pam_config_validate`, pamflow 0.6.0 `R/config.R`).
//!
//! The user's file is laid over pamflow's template defaults with R's
//! `utils::modifyList` rules: mappings merge key by key, anything else
//! replaces, and a key whose value is null is **removed** (so the template
//! default does not come back either). Folders in `paths:` that are not
//! absolute are resolved against `paths: project_root:`, itself relative to the
//! folder holding the file.

use crate::project::is_absolute;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

/// pamflow 0.6.0's `inst/extdata/config_template.yml`.
pub const TEMPLATE: &str = include_str!("../data/config_template.yml");

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigError {
    #[error("Config file not found: {0}")]
    NotFound(String),
    #[error("Could not read {path}: {message}")]
    Unreadable { path: String, message: String },
    #[error("Invalid pamflow configuration:\n{}", .0.iter().map(|p| format!("\u{2716} {p}")).collect::<Vec<_>>().join("\n"))]
    Invalid(Vec<String>),
}

/// A settings file read the way pamflow reads it.
#[derive(Debug, Clone)]
pub struct Config {
    /// The settings file.
    pub path: PathBuf,
    /// The project folder (`paths: project_root:` resolved), forward slashes.
    pub root: String,
    /// All settings: the file over the template defaults, paths resolved.
    pub data: Yaml,
}

impl Config {
    /// Read, merge over the template, resolve paths and validate.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let user = load_file(path)?;
        let mut data = parse(TEMPLATE).unwrap_or(Yaml::Hash(Default::default()));
        let user = if user.as_hash().is_some() { user } else { Yaml::Hash(Default::default()) };
        modify_list(&mut data, &user);

        let config_dir = canonical(path.parent().unwrap_or(Path::new(".")));
        let root_setting = data["paths"]["project_root"].as_str().unwrap_or(".").to_string();
        let root_dir =
            if is_absolute(&root_setting) { PathBuf::from(&root_setting) } else { config_dir.join(&root_setting) };
        let root = norm(&root_dir);

        if let Yaml::Hash(top) = &mut data {
            let paths_key = Yaml::String("paths".into());
            let paths = top.entry(paths_key).or_insert_with(|| Yaml::Hash(Default::default()));
            if let Yaml::Hash(paths) = paths {
                for (_, v) in paths.iter_mut() {
                    if let Yaml::String(p) = v {
                        if !is_absolute(p) {
                            *p = join_slash(&root, p);
                        }
                    }
                }
                paths.insert(Yaml::String("project_root".into()), Yaml::String(root.clone()));
            }
        }

        let problems = validate(&data);
        if !problems.is_empty() {
            return Err(ConfigError::Invalid(problems));
        }
        Ok(Self { path: path.to_path_buf(), root, data })
    }

    /// `section: key:` of the merged settings (`Yaml::BadValue` when absent).
    pub fn get(&self, section: &str, key: &str) -> &Yaml {
        &self.data[section][key]
    }
}

/// Parse YAML text; the first document, or Null for an empty file.
pub fn parse(text: &str) -> Result<Yaml, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    YamlLoader::load_from_str(text).map(|d| d.into_iter().next().unwrap_or(Yaml::Null)).map_err(|e| e.to_string())
}

/// Read and parse a YAML file.
pub fn load_file(path: &Path) -> Result<Yaml, ConfigError> {
    if !path.is_file() {
        return Err(ConfigError::NotFound(path.display().to_string()));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::Unreadable { path: path.display().to_string(), message: e.to_string() })?;
    parse(&text).map_err(|message| ConfigError::Unreadable { path: path.display().to_string(), message })
}

/// R's `utils::modifyList(base, val)`: merge mappings recursively; a null in
/// `val` removes the key; anything else replaces it.
pub fn modify_list(base: &mut Yaml, val: &Yaml) {
    let (Yaml::Hash(b), Yaml::Hash(v)) = (base, val) else { return };
    for (k, new) in v {
        if new.is_null() {
            b.remove(k);
            continue;
        }
        match b.get_mut(k) {
            Some(old) if old.as_hash().is_some() && new.as_hash().is_some() => modify_list(old, new),
            Some(old) => *old = new.clone(),
            None => {
                b.insert(k.clone(), new.clone());
            }
        }
    }
}

/// A number, as R sees it (integer or real).
pub fn num(y: &Yaml) -> Option<f64> {
    match y {
        Yaml::Integer(i) => Some(*i as f64),
        Yaml::Real(_) => y.as_f64(),
        _ => None,
    }
}

fn present(y: &Yaml) -> bool {
    !matches!(y, Yaml::BadValue | Yaml::Null)
}

/// `pam_config_validate`: the problems found, empty when the settings are fine.
///
/// One deliberate difference from R: a check on a number fails when the
/// value is not a number at all (R would compare it as text).
pub fn validate(cfg: &Yaml) -> Vec<String> {
    let mut problems = Vec::new();
    let mut chk = |ok: bool, msg: &str| {
        if !ok {
            problems.push(msg.to_string());
        }
    };
    let whole_ge1 = |y: &Yaml| num(y).is_some_and(|v| v >= 1.0 && v == v.round());
    let b = &cfg["birdnet"];
    if present(b) {
        let v = &b["min_confidence"];
        if present(v) {
            chk(num(v).is_some_and(|x| (0.0..=1.0).contains(&x)), "birdnet$min_confidence must be in [0, 1]");
        }
        let v = &b["workers"];
        if present(v) {
            chk(num(v).is_some_and(|x| x >= 1.0), "birdnet$workers must be >= 1");
        }
        let v = &b["overlap_s"];
        if present(v) {
            chk(num(v).is_some_and(|x| x >= 0.0), "birdnet$overlap_s must be >= 0");
        }
        let v = &b["backend"];
        if present(v) {
            chk(
                v.as_str() == Some("birdnet"),
                "birdnet$backend must be \"birdnet\" (the birdnetR engine was removed); delete the line to use the default",
            );
        }
        let v = &b["top_k"];
        if present(v) {
            chk(whole_ge1(v), "birdnet$top_k must be a whole number >= 1, or null for no limit");
        }
        for key in ["py_batch_size", "n_producers"] {
            let v = &b[key];
            if present(v) {
                chk(whole_ge1(v), &format!("birdnet${key} must be a whole number >= 1"));
            }
        }
        let v = &b["sigmoid_sensitivity"];
        if present(v) {
            chk(num(v).is_some_and(|x| x > 0.0), "birdnet$sigmoid_sensitivity must be > 0");
        }
    }
    let v = &cfg["occupancy"]["occasion_length_days"];
    if present(v) {
        chk(num(v).is_some_and(|x| x >= 1.0), "occupancy$occasion_length_days must be >= 1");
    }
    if let Some(p) = cfg["filenames"]["pattern"].as_str().filter(|p| !p.is_empty()) {
        chk(fancy_regex::Regex::new(p).is_ok(), "filenames$pattern is not a valid PCRE regex");
    }
    if let Some(tz) = cfg["filenames"]["timezone"].as_str().filter(|t| !t.is_empty()) {
        chk(tz.parse::<chrono_tz::Tz>().is_ok(), "filenames$timezone is not a valid Olson timezone");
    }
    problems
}

/// An existing folder in canonical form (no `\\?\` prefix), else made absolute.
fn canonical(p: &Path) -> PathBuf {
    let p = if p.as_os_str().is_empty() { Path::new(".") } else { p };
    match std::fs::canonicalize(p) {
        Ok(c) => {
            let s = c.to_string_lossy().to_string();
            PathBuf::from(s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s))
        }
        Err(_) => std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
    }
}

/// Like R's `normalizePath(mustWork = FALSE)`: canonical when it exists,
/// otherwise absolute with `.` and `..` left as written; forward slashes.
fn norm(p: &Path) -> String {
    let c = if p.exists() { canonical(p) } else { std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()) };
    c.to_string_lossy().replace('\\', "/")
}

/// `fs::path(root, p)`: joined with one forward slash.
fn join_slash(root: &str, p: &str) -> String {
    let p = p.replace('\\', "/");
    if p.is_empty() {
        return root.to_string();
    }
    format!("{}/{}", root.trim_end_matches('/'), p.trim_start_matches("./"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_valid() {
        let t = parse(TEMPLATE).unwrap();
        assert!(validate(&t).is_empty(), "{:?}", validate(&t));
    }

    #[test]
    fn null_removes_a_key() {
        let mut base = parse("a:\n  x: 1\n  y: 2\nb: 3\n").unwrap();
        modify_list(&mut base, &parse("a:\n  x: null\n  z: 4\nb: [1, 2]\n").unwrap());
        assert!(base["a"]["x"].is_badvalue());
        assert_eq!(base["a"]["y"].as_i64(), Some(2));
        assert_eq!(base["a"]["z"].as_i64(), Some(4));
        assert_eq!(base["b"].as_vec().map(Vec::len), Some(2));
    }

    #[test]
    fn checks() {
        let bad = parse(
            "birdnet:\n  min_confidence: 2\n  top_k: 1.5\n  workers: 0\nfilenames:\n  pattern: \"(\"\n  timezone: \"Mars/Base\"\noccupancy:\n  occasion_length_days: 0\n",
        )
        .unwrap();
        let p = validate(&bad);
        assert_eq!(p.len(), 6, "{p:?}");
        let ok = parse("birdnet:\n  workers: null\n  top_k: 5\nfilenames:\n  timezone: \"UTC\"\n").unwrap();
        assert!(validate(&ok).is_empty());
    }
}
