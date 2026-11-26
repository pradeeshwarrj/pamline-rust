# pamflow in Rust

Rewrite of pamflow Desktop (0.2.1) and the parts of the pamflow R package (0.6.0)
the app uses, so the app no longer ships R and Python. Plan: the handoff document
"rebuilding pamflow Desktop entirely in Rust" (2026-11-25).

The reference for every comparison is the app as shipped: pamflow 0.6.0 as
installed in pamflow Desktop 0.2.1's runtime (`runtime/.pixi/envs/default`).

## Decisions (2026-11-25)

- The interface is rewritten in Rust too, with Leptos (Rust compiled to WebAssembly) inside the existing Tauri 2 shell (decided 2026-11-26). The app lives in this repository: the shell code (jobs, dialogs, close prompt, installers) is brought over from pamflow-desktop and the Svelte screens are ported one by one, Project first.
- BirdNET: the spike is done ([spikes/birdnet/REPORT.md](spikes/birdnet/REPORT.md)).
  The TensorFlow Lite C library (a 4.3 MB DLL) run from Rust matches Python's
  scores to 7.4e-5 on the global model and a custom classifier, and is faster.
  The pure-Rust runtime tract cannot load the models.
- Same project files as the R version: existing projects and half-finished runs
  keep working.
- Acoustic indices are implemented from the published papers; the GPL R packages
  (soundecology, seewave) are used only as black-box references.

## What runs in Rust

`pamflow-engine <job.json>` takes the same job file as `Rscript bridge/run_job.R`
and prints the same `@@PAM` events, with the same exit codes. Jobs ported:

| Job | Checked against R with |
|---|---|
| `inspect` | `tools/compare/inspect_job.py` |
| `config_save`, `config_save_as` | `tools/compare/config_save.py` |
| `project_create`, `example_project`, `diagnostics` | `tools/compare/project_jobs.py` |
| `scan`, `scan_results` | `tools/compare/scan.py` (see `tools/compare/SCAN_NOTES.md`) |
| `std_state`, `std_plan`, `standardize` | `tools/compare/standardize.py` (5 setting variants and 4 failure cases; converted files decode identically, see below) |

Each comparison runs both engines on their own copies of the same projects
(pamflow's example project, hand-made edge cases, and the real 4,700-recording
project) and compares files byte for byte and events field by field. Build first
(`cargo build`), then for example:

```
python tools/compare/scan.py <pamflow-desktop> <scratch folder> D:/PAM_GNI/config/config.yml
```

`pamflow-core` modules: `config_edit` (comment-keeping editor), `config` (reader:
template merge, paths, checks), `config_save`, `project`, `project_jobs`,
`inspect`, `audio_files` (listing, file-name parsing), `standardize` (Stage 00), `audio_header` (WAV like
tuneR, FLAC STREAMINFO), `scan`, `rcsv` (CSV like data.table fwrite/fread),
`rtime` (dates like R's strptime), `collate` (R's sort order), `stage_log`
(pamflow's stage log), `events`.

Known differences from R, on purpose:
- Two saves in the same second: R refuses the second one (the backup name exists); Rust numbers the second backup (`..._2.bak`).
- A number setting holding text (`min_confidence: "abc"`) is refused; R compares it as text and lets it through.
- `diagnostics` lists the Rust engine's versions instead of R and Python's.
- `standardize` converts audio with the `ffmpeg` program (on the PATH) where R uses its built-in libav. The converted audio is sample-identical on the test files, but the files differ in bytes (the encoder version tag differs: `Lavf61` in R's package, whatever ffmpeg is installed). Copied files are byte-identical. As in R's pamflow, mp3/m4a/ogg/wma/aiff are listed as unreadable (so isolated as corrupt): only WAV and FLAC headers are read.

## The app (`app/`)

Leptos (Rust compiled to WebAssembly) inside a Tauri 2 shell, ported from pamflow-desktop:

- `app/src-tauri`: the shell (job runner, engine launcher, dialogs, close prompt, first-launch setup). Jobs in the routing table run in `pamflow-engine`, the rest in the R bridge, which is still read from the `pamflow-desktop` folder next to this repository (or `PAMFLOW_DESKTOP`).
- `app/ui`: the screens. Done: the shell around them (sidebar with navigation), Project, Standardize and Scan, plus the live job panel (progress, Stop, finished files) that Filter, Detect and Soundscape will reuse. Text comes from `en.json`, the look from the old app's tokens.

Tools: `rustup target add wasm32-unknown-unknown`, `cargo install trunk tauri-cli --locked`. Run it (build the engine first, `cargo build`):

```
cd app
cargo tauri dev
```

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
