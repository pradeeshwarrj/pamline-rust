//! Finding a project's settings file and the folder it belongs to
//! (`pd_find_config`, `pd_config_root` in `bridge/protocol.R`).

use crate::config_edit::{derive_root, norm_path};
use std::path::{Path, PathBuf};
use yaml_rust2::YamlLoader;

/// A settings file and the folder the project's relative paths start from.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub config: PathBuf,
    pub root: String,
}

/// Written as an absolute path (C:/..., /home/..., ~/..., \\server\...)?
pub fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
        || p.starts_with('/')
        || p.starts_with('~')
        || p.starts_with("\\\\")
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            return PathBuf::from(home).join(rest.trim_start_matches(['/', '\\']));
        }
    }
    PathBuf::from(p)
}

/// The project folder of a settings file: `paths: project_root:` when it is
/// absolute, else the folder above `config/`, else the folder holding the file.
pub fn config_root(config: &Path) -> String {
    let pinned = std::fs::read_to_string(config)
        .ok()
        .and_then(|t| YamlLoader::load_from_str(&t).ok())
        .and_then(|docs| docs.into_iter().next())
        .and_then(|d| d["paths"]["project_root"].as_str().map(str::to_string));
    match pinned {
        Some(p) if is_absolute(&p) => norm_path(&expand_home(&p)),
        _ => derive_root(config),
    }
}

/// Find a project's settings file. `project` is a project folder (looks for
/// config/config.yml, then config.yml) or a .yml file; `config`, if given,
/// is a settings file chosen explicitly and wins.
pub fn find_config(project: Option<&str>, config: Option<&str>) -> Option<Found> {
    let project = project.filter(|p| !p.is_empty()).map(|p| PathBuf::from(norm_path(&expand_home(p))));
    let explicit = config.filter(|c| !c.is_empty()).map(|c| PathBuf::from(norm_path(&expand_home(c))));
    let is_yml = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"));
    let yml = explicit.or_else(|| project.clone().filter(|p| is_yml(p)));
    if let Some(yml) = yml {
        if !yml.is_file() {
            return None;
        }
        let root = config_root(&yml);
        return Some(Found { config: yml, root });
    }
    let project = project?;
    for candidate in ["config/config.yml", "config/config.yaml", "config.yml", "config.yaml"] {
        let path = project.join(candidate);
        if path.is_file() {
            return Some(Found { config: path, root: project.to_string_lossy().replace('\\', "/") });
        }
    }
    None
}
