//! Saving settings: refused saves change nothing, backups, line endings.
//! (Same-output-as-R checks: tools/compare/config_save.py.)

use pamflow_core::config::TEMPLATE;
use pamflow_core::config_edit::Change;
use pamflow_core::config_save::{check_allowed, save, save_as};
use pamflow_core::project::find_config;
use serde_json::{json, Value};
use std::path::PathBuf;

fn project(name: &str, crlf: bool) -> PathBuf {
    let dir = std::env::temp_dir().join("pamflow-core-save-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    let text = TEMPLATE.replace("\r\n", "\n");
    let text = if crlf { text.replace('\n', "\r\n") } else { text };
    std::fs::write(dir.join("config").join("config.yml"), text).unwrap();
    dir
}

fn ch(section: &str, key: &str, value: Value) -> Change {
    Change { section: section.into(), key: key.into(), value }
}

#[test]
fn refused_save_changes_nothing() {
    let dir = project("refused", false);
    let found = find_config(dir.to_str(), None).unwrap();
    let before = std::fs::read(&found.config).unwrap();
    let err = save(&found, &[ch("birdnet", "min_confidence", json!(2))], "20260101-000000").unwrap_err();
    assert!(err.contains("birdnet$min_confidence must be in [0, 1]"), "{err}");
    assert_eq!(std::fs::read(&found.config).unwrap(), before);
    assert!(!dir.join("config").join("backups").exists(), "no backup for a refused save");
    let leftovers: Vec<_> = std::fs::read_dir(dir.join("config")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 1, "check file removed: {leftovers:?}");
}

#[test]
fn not_editable_is_refused() {
    assert!(check_allowed(&[ch("birdnet", "python_env", json!("x"))]).is_err());
    assert!(check_allowed(&[ch("paths", "project_root", json!("x")), ch("birdnet", "workers", json!(2))]).is_ok());
}

#[test]
fn backups_and_two_saves_in_one_second() {
    let dir = project("backups", false);
    let found = find_config(dir.to_str(), None).unwrap();
    let first = save(&found, &[ch("birdnet", "workers", json!(4))], "20260101-120000").unwrap().unwrap();
    let second = save(&found, &[ch("birdnet", "workers", json!(5))], "20260101-120000").unwrap().unwrap();
    assert!(first.backup.ends_with("backups/config.yml.20260101-120000.bak"), "{}", first.backup);
    assert!(second.backup.ends_with("backups/config.yml.20260101-120000_2.bak"), "{}", second.backup);
    let old = std::fs::read_to_string(&second.backup).unwrap();
    assert!(old.contains("workers: 4"));
    assert!(std::fs::read_to_string(&found.config).unwrap().contains("workers: 5"));
    assert_eq!(save(&found, &[], "x").unwrap(), None, "nothing to save");
}

#[test]
fn windows_line_endings_are_kept() {
    let dir = project("crlf", true);
    let found = find_config(dir.to_str(), None).unwrap();
    save(&found, &[ch("report", "author", json!("RJ"))], "20260101-000000").unwrap();
    let text = std::fs::read_to_string(&found.config).unwrap();
    assert!(text.contains("author: \"RJ\""));
    assert_eq!(text.matches('\n').count(), text.matches("\r\n").count(), "every line ends with CRLF");
}

#[test]
fn save_as_pins_project_root_only_outside_config() {
    let dir = project("save_as", false);
    let found = find_config(dir.to_str(), None).unwrap();
    let elsewhere = dir.parent().unwrap().join("save_as_shared").join("study");
    let (saved, now) = save_as(&found, elsewhere.to_str().unwrap(), &[], "20260101-000000").unwrap();
    assert!(saved.config.to_string_lossy().ends_with("study.yml"));
    let text = std::fs::read_to_string(&saved.config).unwrap();
    assert!(text.contains(&format!("project_root: \"{}\"", found.root)), "pinned");
    assert_eq!(now.root.to_lowercase(), found.root.to_lowercase(), "still the same project");

    let inside = dir.join("config").join("other.yml");
    let (saved, _) = save_as(&found, inside.to_str().unwrap(), &[], "20260101-000000").unwrap();
    assert!(!std::fs::read_to_string(&saved.config).unwrap().contains(&found.root), "not pinned inside config/");
}
