# pamflow in Rust

Rewrite of pamflow Desktop (0.2.1) and the parts of the pamflow R package (0.6.0)
the app uses, so the app no longer ships R and Python. Plan: the handoff document
"rebuilding pamflow Desktop entirely in Rust" (2026-11-25).

## Decisions (2026-11-25)

- The interface is rewritten in Rust too (framework not chosen yet).
- BirdNET: the spike is done ([spikes/birdnet/REPORT.md](spikes/birdnet/REPORT.md)).
  The TensorFlow Lite C library (a 4.3 MB DLL) run from Rust matches Python's
  scores to 7.4e-5 on the global model and a custom classifier, and is faster.
  The pure-Rust runtime tract cannot load the models.
- Same project files as the R version: existing projects and half-finished runs
  keep working.
- Acoustic indices are implemented from the published papers; the GPL R packages
  (soundecology, seewave) are used only as black-box references.

## Layout

| Crate | Status |
|---|---|
| `pamflow-core` | config editor (port of `bridge/config_edit.R`); config reader with template merge, path resolution and pamflow's checks (`pam_config_read`, `pam_config_validate`); saving with check file, read-back and backups; project discovery; `@@PAM` events |
| `pamflow-engine` | `pamflow-engine <job.json>`: same job file, events and exit codes as `Rscript bridge/run_job.R`. Runs `inspect`, `config_save` and `config_save_as` (each save ends with the project report, as in R) |

Checked against R with `tools/compare/inspect.py` and `tools/compare/config_save.py`
(example project, a project without settings, refused settings, and the real
4,700-recording project): identical events and files.

Known differences from R, on purpose:
- Two saves in the same second: R refuses the second one (the backup name exists); Rust numbers the second backup (`..._2.bak`).
- A number setting holding text (`min_confidence: "abc"`) is refused; R compares it as text and lets it through.

## Tests

```
cargo test
```

The template tests read pamflow's `config_template.yml` from `../pamflow` (next to
this folder) or from `PAMFLOW_TEMPLATE`.

`crates/pamflow-core/tests/golden/config_edit/` holds edit cases whose expected
output was made by the R editor; the Rust editor must match it byte for byte.
To regenerate (with the runtime bundled in pamflow-desktop):

`tools/compare/config_save.py` runs the same save jobs through the R bridge and
the Rust engine on copies of a project and compares the saved files, backups and
events (`cargo build` first):

```
python tools/compare/config_save.py <pamflow-desktop> <scratch folder> [a real config.yml ...]
```

```
python tools/golden/config_edit_cases.py
<pamflow-desktop>/runtime/.pixi/envs/default/lib/R/bin/Rscript.exe tools/golden/config_edit.R <pamflow-desktop>
```
