# pamflow in Rust

Rewrite of pamflow Desktop (0.2.1) and the parts of the pamflow R package (0.6.0)
the app uses, so the app no longer ships R and Python. Plan: the handoff document
"rebuilding pamflow Desktop entirely in Rust" (2026-11-25).

## Decisions (2026-11-25)

- The interface is rewritten in Rust too (framework not chosen yet).
- BirdNET: spike first (pure-Rust runtime vs. the TensorFlow Lite C library) on
  the global model and one custom classifier, compared with Python birdnet 0.2.16.
- Same project files as the R version: existing projects and half-finished runs
  keep working.
- Acoustic indices are implemented from the published papers; the GPL R packages
  (soundecology, seewave) are used only as black-box references.

## Layout

| Crate | Status |
|---|---|
| `pamflow-core` | config editor (port of `bridge/config_edit.R`), project discovery, `@@PAM` events |
| `pamflow-engine` | `pamflow-engine <job.json>`: same job file, events and exit codes as `Rscript bridge/run_job.R`; no job ported yet |

## Tests

```
cargo test
```

The template tests read pamflow's `config_template.yml` from `../pamflow` (next to
this folder) or from `PAMFLOW_TEMPLATE`.

`crates/pamflow-core/tests/golden/config_edit/` holds edit cases whose expected
output was made by the R editor; the Rust editor must match it byte for byte.
To regenerate (with the runtime bundled in pamflow-desktop):

```
python tools/golden/config_edit_cases.py
<pamflow-desktop>/runtime/.pixi/envs/default/lib/R/bin/Rscript.exe tools/golden/config_edit.R <pamflow-desktop>
```
