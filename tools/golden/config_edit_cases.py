"""Write the config-editor golden cases (inputs + edit lists).

Run from the workspace root:  python tools/golden/config_edit_cases.py
then make the expected outputs with tools/golden/config_edit.R.
template.yml is pamflow's inst/extdata/config_template.yml (copied as is).
"""
import json
import os

G = "crates/pamflow-core/tests/golden/config_edit"
os.makedirs(G, exist_ok=True)

cfg = [
    "paths:",
    '  project_root: "."',
    '  audio_standardized: "data/standardized"',
    "",
    "filenames:",
    "  # G0059_YYYYMMDD_HHMMSS.flac ; timestamps are local time",
    r'  pattern: "^(?<grid>G\\d{4})_(?<date>\\d{8})_(?<time>\\d{6})(_chunk\\d+)?\\.(flac|wav)$"',
    '  timezone: "Asia/Kolkata"',
    "",
    "birdnet:",
    '  run_id: "run01"',
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
    '  title: "PAM # Report"   # a hash inside quotes is not a comment',
]
sc = [
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
    '  run_id: "x"',
]
for name, lines in (("cfg.yml", cfg), ("sc.yml", sc)):
    with open(os.path.join(G, name), "w", newline="\n", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


def c(s, k, v):
    return {"section": s, "key": k, "value": v}


cases = {
    "cfg_scalars": ("cfg.yml", [
        c("birdnet", "workers", 8), c("birdnet", "chunk_size", None),
        c("paths", "audio_standardized", 'E:\\GNI\\Standard "set"'),
        c("report", "title", "Nilgiris PAM"), c("birdnet", "overlap_s", 1.5),
        c("birdnet", "min_confidence", 0.1 + 0.2)]),
    "cfg_lists": ("cfg.yml", [
        c("processing", "species_whitelist", []), c("deployments", "columns", ["a", "b"]),
        c("validation", "bands", [0.4, 0.6, 1]), c("validation", "spec_px", [800, 300]),
        c("validation", "show_confidence", True)]),
    "cfg_one_item": ("cfg.yml", [
        c("processing", "species_whitelist", ["Gracula religiosa"]), c("newsec", "x", 0.000123)]),
    "sc_indices": ("sc.yml", [c("soundscape", "acoustic_indices", ["NDSI", "Bio", "ADI"])]),
    "sc_indices2": ("sc.yml", [
        c("soundscape", "acoustic_indices", ["M"]), c("soundscape", "window_mins", None)]),
    "template_main": ("template.yml", [
        c("birdnet", "workers", 6), c("birdnet", "min_confidence", 0.35),
        c("processing", "species_whitelist", ["Gracula religiosa"]),
        c("occupancy", "occasion_length_days", 7), c("report", "author", "Pradeeshwar"),
        c("validation", "reviewer", "RJ")]),
    "template_more": ("template.yml", [
        c("processing", "species_whitelist", ["Psittacula columboides"]),
        c("processing", "species_blacklist", ["Gallus gallus", "Pavo cristatus"]),
        c("processing", "date_range", ["2026-03-26", "2026-04-20"]),
        c("processing", "species_thresholds_file", "data/thr.csv"),
        c("processing", "species_season_file", None), c("birdnet", "top_k", 5),
        c("birdnet", "device", "GPU"),
        c("deployments", "columns", ["deployment_id", "grid", "deploy_date"]),
        c("validation", "bands", [0.4, 0.6, 1]), c("validation", "spec_px", [800, 300]),
        c("recall", "hours", []), c("report", "formats", ["html"])]),
    "template_indices": ("template.yml", [
        c("soundscape", "acoustic_indices", ["ACI", "NDSI"]), c("soundscape", "batch_size", 20),
        c("paths", "project_root", "D:/PAM_GNI")]),
}
for name, (inp, changes) in cases.items():
    with open(os.path.join(G, name + ".json"), "w", encoding="utf-8") as f:
        json.dump({"input": inp, "expected": name + ".expected.yml", "changes": changes}, f, indent=1)
print(len(cases), "cases")
