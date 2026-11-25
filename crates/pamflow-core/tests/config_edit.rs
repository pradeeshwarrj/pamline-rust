//! Port of pamflow-desktop `bridge/tests/test_config_edit.R`.
//!
//! The template tests read pamflow's real `config_template.yml`: set
//! `PAMFLOW_TEMPLATE` to its path, or keep the pamflow repository next to this
//! one (`../pamflow/inst/extdata/config_template.yml`). Without it they are
//! skipped with a message, as in R.

use pamflow_core::config_edit::{self as ce, Change, ConfigText, EditError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

fn cfg_text() -> Vec<String> {
    [
        "paths:",
        "  project_root: \".\"",
        "  audio_standardized: \"data/standardized\"",
        "",
        "filenames:",
        "  # G0059_YYYYMMDD_HHMMSS.flac ; timestamps are local time",
        r#"  pattern: "^(?<grid>G\\d{4})_(?<date>\\d{8})_(?<time>\\d{6})(_chunk\\d+)?\\.(flac|wav)$""#,
        "  timezone: \"Asia/Kolkata\"",
        "",
        "birdnet:",
        "  run_id: \"run01\"",
        "  min_confidence: 0.3",
        "  workers: null             # parallel worker processes, each with its own model copy.",
        "                            # null = half of the physical cores.",
        "  chunk_size: 100           # files per batch",
        "",
        "deployments:",
        "  columns: [deployment_id, recorder, grid]",
        "processing:",
        "  species_whitelist:",
        "    - Psittacula columboides",
        "    - Ocyceros griseus",
        "  min_confidence: 0.5",
        "report:",
        "  title: \"PAM # Report\"   # a hash inside quotes is not a comment",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn parse(lines: &[String]) -> Yaml {
    let docs = YamlLoader::load_from_str(&lines.join("\n")).expect("file parses as YAML");
    docs.into_iter().next().unwrap_or(Yaml::Null)
}

fn ch(section: &str, key: &str, value: Value) -> Change {
    Change { section: section.into(), key: key.into(), value }
}

fn strs(y: &Yaml) -> Vec<String> {
    match y {
        Yaml::Array(a) => a.iter().map(|v| v.as_str().unwrap().to_string()).collect(),
        Yaml::String(s) => vec![s.clone()],
        _ => vec![],
    }
}

fn ints(y: &Yaml) -> Vec<i64> {
    y.as_vec().map(|a| a.iter().map(|v| v.as_i64().unwrap()).collect()).unwrap_or_default()
}

fn num(y: &Yaml) -> f64 {
    y.as_f64().or_else(|| y.as_i64().map(|i| i as f64)).expect("a number")
}

fn keys(y: &Yaml) -> Vec<String> {
    y.as_hash().map(|h| h.keys().map(|k| k.as_str().unwrap().to_string()).collect()).unwrap_or_default()
}

fn template_path() -> Option<PathBuf> {
    let p = std::env::var_os("PAMFLOW_TEMPLATE").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pamflow/inst/extdata/config_template.yml")
    });
    p.is_file().then_some(p)
}

#[test]
fn value_changed_comment_kept() {
    let out = ce::set(cfg_text(), "birdnet", "workers", &json!(8));
    assert!(out.iter().any(|l| l == "  workers: 8  # parallel worker processes, each with its own model copy."));
    assert!(out.iter().any(|l| l.contains("# null = half of the physical cores.")));
    let p = parse(&out);
    assert_eq!(p["birdnet"]["workers"].as_i64(), Some(8));
    assert_eq!(p["birdnet"]["chunk_size"].as_i64(), Some(100));
}

#[test]
fn strings_quoted_and_escaped() {
    let v = "E:\\GNI\\Standard \"set\"";
    let out = ce::set(cfg_text(), "paths", "audio_standardized", &json!(v));
    assert_eq!(parse(&out)["paths"]["audio_standardized"].as_str(), Some(v));
}

#[test]
fn pattern_line_untouched() {
    let base = cfg_text();
    let out = ce::set(base.clone(), "filenames", "timezone", &json!("Asia/Kolkata"));
    assert_eq!(out[6], base[6]);
    let pattern = parse(&out)["filenames"]["pattern"].as_str().unwrap().to_string();
    assert_eq!(pattern, r"^(?<grid>G\d{4})_(?<date>\d{8})_(?<time>\d{6})(_chunk\d+)?\.(flac|wav)$");
}

#[test]
fn block_list_replaced_by_flow_list() {
    let out = ce::set(cfg_text(), "processing", "species_whitelist", &json!([]));
    let p = parse(&out);
    assert_eq!(p["processing"]["species_whitelist"].as_vec().map(Vec::len), Some(0));
    assert_eq!(num(&p["processing"]["min_confidence"]), 0.5);
    let out = ce::set(cfg_text(), "processing", "species_whitelist", &json!(["Gracula religiosa"]));
    assert_eq!(strs(&parse(&out)["processing"]["species_whitelist"]), ["Gracula religiosa"]);
}

#[test]
fn null_and_new_section() {
    let out = ce::set(cfg_text(), "birdnet", "chunk_size", &Value::Null);
    assert!(parse(&out)["birdnet"]["chunk_size"].is_null());
    assert!(out.iter().any(|l| l.contains("chunk_size: null  # files per batch")));
    let out = ce::set(cfg_text(), "validation", "show_confidence", &json!(true));
    assert_eq!(parse(&out)["validation"]["show_confidence"].as_bool(), Some(true));
}

#[test]
fn missing_key_added_inside_its_section() {
    let out = ce::set(cfg_text(), "birdnet", "overlap_s", &json!(1.5));
    let p = parse(&out);
    assert_eq!(num(&p["birdnet"]["overlap_s"]), 1.5);
    assert_eq!(p["birdnet"]["run_id"].as_str(), Some("run01"));
    let at = out.iter().position(|l| l == "  overlap_s: 1.5").unwrap();
    let dep = out.iter().position(|l| l == "deployments:").unwrap();
    assert!(at < dep);
}

#[test]
fn hash_inside_quotes_is_value() {
    let out = ce::set(cfg_text(), "report", "title", &json!("Nilgiris PAM"));
    assert!(out.iter().any(|l| l == "  title: \"Nilgiris PAM\"  # a hash inside quotes is not a comment"));
}

#[test]
fn only_edited_lines_differ() {
    let base = cfg_text();
    let out = ce::apply(
        base.clone(),
        &[ch("birdnet", "min_confidence", json!(0.4)), ch("report", "title", json!("X"))],
    )
    .unwrap();
    assert_eq!(out.len(), base.len());
    assert_eq!(out.iter().zip(&base).filter(|(a, b)| a != b).count(), 2);
}

#[test]
fn template_edits_keep_comments_and_other_settings() {
    let Some(path) = template_path() else {
        eprintln!("skip template test (set PAMFLOW_TEMPLATE to pamflow's config_template.yml)");
        return;
    };
    let tpl = ConfigText::read(&path).unwrap().lines;
    let edits = [
        ch("birdnet", "workers", json!(6)),
        ch("birdnet", "min_confidence", json!(0.35)),
        ch("processing", "species_whitelist", json!(["Gracula religiosa"])),
        ch("occupancy", "occasion_length_days", json!(7)),
        ch("report", "author", json!("Pradeeshwar")),
        ch("validation", "reviewer", json!("RJ")),
    ];
    let out = ce::apply(tpl.clone(), &edits).unwrap();
    let comment_of = |l: &String| l.find('#').map(|i| l[i..].to_string());
    let after: Vec<String> = out.iter().filter_map(comment_of).collect();
    for c in tpl.iter().filter_map(comment_of) {
        assert!(after.contains(&c), "comment lost: {c}");
    }
    let (p, p0) = (parse(&out), parse(&tpl));
    assert_eq!(p["birdnet"]["workers"].as_i64(), Some(6));
    assert_eq!(num(&p["birdnet"]["min_confidence"]), 0.35);
    assert_eq!(p["occupancy"]["occasion_length_days"].as_i64(), Some(7));
    assert_eq!(p["report"]["author"].as_str(), Some("Pradeeshwar"));
    assert_eq!(p["validation"]["reviewer"].as_str(), Some("RJ"));
    assert_eq!(strs(&p["processing"]["species_whitelist"]), ["Gracula religiosa"]);
    let edited: Vec<String> = edits.iter().map(|e| format!("{}.{}", e.section, e.key)).collect();
    assert_eq!(keys(&p), keys(&p0));
    for sec in keys(&p0) {
        match p0[sec.as_str()].as_hash() {
            None => assert_eq!(p[sec.as_str()], p0[sec.as_str()]),
            Some(h) => {
                for k in h.keys().map(|k| k.as_str().unwrap()) {
                    if edited.contains(&format!("{sec}.{k}")) {
                        continue;
                    }
                    assert_eq!(p[sec.as_str()][k], p0[sec.as_str()][k], "{sec}.{k} changed");
                }
            }
        }
    }
}

#[test]
fn new_settings_files() {
    let tpl = template_path().map(|p| ConfigText::read(&p).unwrap().lines).unwrap_or_else(cfg_text);
    let tmp = std::env::temp_dir().join("pamflow-core-tests");
    let proj = tmp.join("my project");
    let std_ = ce::new_config_lines(&tpl, &proj, &proj.join("config").join("config.yml"), "0.6.0", "2026-11-03");
    assert_eq!(std_[0], "# pamflow project settings");
    assert!(std_[1].contains("2026-11-03") && std_[1].contains("pamflow 0.6.0"));
    let (ps, pt) = (parse(&std_), parse(&tpl));
    assert_eq!(ps["paths"]["project_root"], pt["paths"]["project_root"], "project_root left as in the template");
    assert_eq!(ps, pt, "every setting equals the template");
    assert!(tpl.iter().all(|l| std_.contains(l)), "template lines kept as they are");

    let pin = ce::new_config_lines(&tpl, &proj, &tmp.join("shared").join("study.yml"), "", "2026-11-03");
    assert_eq!(parse(&pin)["paths"]["project_root"].as_str(), Some(ce::norm_path(&proj).as_str()));
    assert_eq!(ce::derive_root(&proj.join("config").join("x.yml")).to_lowercase(), ce::norm_path(&proj).to_lowercase());

    std::fs::create_dir_all(&tmp).unwrap();
    let f = tmp.join("written.yml");
    std::fs::write(&f, ConfigText::render(&std_, false)).unwrap();
    let bytes = std::fs::read(&f).unwrap();
    assert!(!bytes.contains(&b'\r'));
    assert_eq!(ConfigText::read(&f).unwrap().lines, std_);
}

fn sc() -> Vec<String> {
    [
        "soundscape:",
        "  enabled: true",
        "  window_mins: 10",
        "  acoustic_indices:   # which indices",
        "    ACI:",
        "    Bio:",
        "      flim: [1500, 9000]   # my range",
        "    NDSI:",
        "      anthro_range: [1000, 2000]",
        "  day_hours: [6, 17]",
        "",
        "occupancy:",
        "  run_id: \"x\"",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn soundscape_indices() {
    let o = ce::set_indices(sc(), &names(&["NDSI", "Bio", "ADI"])).unwrap();
    let p = parse(&o);
    let ai = &p["soundscape"]["acoustic_indices"];
    assert_eq!(keys(ai), ["ADI", "Bio", "NDSI"], "chosen ones only, in pamflow's order");
    assert_eq!(ints(&ai["Bio"]["flim"]), [1500, 9000]);
    assert!(o.iter().any(|l| l.contains("# my range")), "kept index keeps its settings and comment");
    assert_eq!(ai["ADI"]["bin_step"].as_i64(), Some(1000), "new index gets pamflow's defaults");
    assert!(o.iter().any(|l| l.contains("# which indices")));
    assert_eq!(ints(&p["soundscape"]["day_hours"]), [6, 17]);
    assert_eq!(p["occupancy"]["run_id"].as_str(), Some("x"));

    let added = ce::set_indices(sc()[..3].to_vec(), &names(&["M"])).unwrap();
    assert_eq!(keys(&parse(&added)["soundscape"]["acoustic_indices"]), ["M"], "added when the key is missing");
    let added = ce::set_indices(names(&["paths:", "  a: 1"]), &names(&["M", "ACI"])).unwrap();
    assert_eq!(keys(&parse(&added)["soundscape"]["acoustic_indices"]), ["ACI", "M"], "added when the section is missing");
    assert_eq!(ce::set_indices(sc(), &[]), Err(EditError::NoIndices));
    assert!(matches!(ce::set_indices(sc(), &names(&["H"])), Err(EditError::UnknownIndex(_))));
    let via_apply = ce::apply(sc(), &[ch("soundscape", "acoustic_indices", json!(["AEve", "M"]))]).unwrap();
    assert_eq!(keys(&parse(&via_apply)["soundscape"]["acoustic_indices"]), ["AEve", "M"]);
}

#[test]
fn template_more_settings_kinds() {
    let Some(path) = template_path() else {
        eprintln!("skip template test (set PAMFLOW_TEMPLATE to pamflow's config_template.yml)");
        return;
    };
    let tl = ConfigText::read(&path).unwrap().lines;
    let chg = [
        ch("processing", "species_whitelist", json!(["Psittacula columboides"])),
        ch("processing", "species_blacklist", json!(["Gallus gallus", "Pavo cristatus"])),
        ch("processing", "date_range", json!(["2026-03-26", "2026-04-20"])),
        ch("processing", "species_thresholds_file", json!("data/thr.csv")),
        ch("processing", "species_season_file", Value::Null),
        ch("birdnet", "top_k", json!(5)),
        ch("birdnet", "device", json!("GPU")),
        ch("deployments", "columns", json!(["deployment_id", "grid", "deploy_date"])),
        ch("validation", "bands", json!([0.4, 0.6, 1])),
        ch("validation", "spec_px", json!([800, 300])),
        ch("recall", "hours", json!([])),
        ch("report", "formats", json!(["html"])),
    ];
    let o = ce::apply(tl.clone(), &chg).unwrap();
    let r = parse(&o);
    assert_eq!(strs(&r["processing"]["species_whitelist"]), ["Psittacula columboides"]);
    assert_eq!(strs(&r["processing"]["species_blacklist"]), ["Gallus gallus", "Pavo cristatus"]);
    assert_eq!(strs(&r["processing"]["date_range"]), ["2026-03-26", "2026-04-20"]);
    assert_eq!(r["processing"]["species_thresholds_file"].as_str(), Some("data/thr.csv"));
    assert!(r["processing"]["species_season_file"].is_null());
    assert_eq!(r["birdnet"]["top_k"].as_i64(), Some(5));
    assert_eq!(r["birdnet"]["device"].as_str(), Some("GPU"));
    assert_eq!(strs(&r["deployments"]["columns"]), ["deployment_id", "grid", "deploy_date"]);
    assert_eq!(ints(&r["validation"]["spec_px"]), [800, 300]);
    let bands: Vec<f64> = r["validation"]["bands"].as_vec().unwrap().iter().map(num).collect();
    assert_eq!(bands, [0.4, 0.6, 1.0]);
    assert_eq!(r["recall"]["hours"].as_vec().map(Vec::len), Some(0));
    assert_eq!(strs(&r["report"]["formats"]), ["html"]);
    assert_eq!(keys(&r), keys(&parse(&tl)));
    let comment_lines = |v: &[String]| v.iter().filter(|l| l.trim_start().starts_with('#')).count();
    assert_eq!(comment_lines(&o), comment_lines(&tl), "every comment line kept");
}

/// The Rust editor must produce byte-for-byte the same file as the R editor
/// on the same edits (outputs saved from R in tests/golden/).
#[test]
fn same_output_as_r() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/config_edit");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skip: no golden files in {}", dir.display());
        return;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            let case: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            let input = ConfigText::read(&dir.join(case["input"].as_str().unwrap())).unwrap();
            let changes: Vec<Change> = serde_json::from_value(case["changes"].clone()).unwrap();
            let want = std::fs::read_to_string(dir.join(case["expected"].as_str().unwrap())).unwrap();
            let got = ConfigText::render(&ce::apply(input.lines, &changes).unwrap(), false);
            assert_eq!(got, want, "case {}", p.display());
            n += 1;
        }
    }
    assert!(n > 0, "no golden cases found");
}
