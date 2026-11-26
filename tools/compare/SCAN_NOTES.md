# Scan port: what R does (findings so far, 2026-11-26)

Reference: pamflow 0.6.0 as installed in the app runtime (functions dumped with
`Rscript` from `asNamespace("pamflow")`), tuneR 1.4.7, data.table 1.18.6.1, av 0.9.6.
Edge project and comparisons: `tools/compare/scan.py` (`--r-only` shows R's events).

## Flow (`pam_scan_audio`, wrapped by `pam_run_stage("01_scan")`)
1. list recordings (`list.files`, pattern `\.(wav|flac|WAV|FLAC)$`), sorted in the
   **Windows locale's order**, not bytes: `deep/er/...` sorts before `G0001/...`
   (case-insensitive; punctuation < digits < letters). Hidden files excluded.
2. header cache `metadata/audio_headers.csv` (file_path, file_size_b, sample_rate,
   channels, bits, duration_s, readable): `force` deletes it; new files appended in
   chunks of `qc.header_chunk_size`; then **re-read from the CSV** (durations come back
   rounded to 15 digits and feed every later sum).
3. parse names (pattern is case-sensitive: `.WAV` / `.FLAC` names are invalid, grid NA).
4. merge headers (file order kept); `pam_add_qc`:
   end = start + duration; expected_bytes = 44 + dur*rate*ch*bits/8 (NA if bits NA, so
   FLAC never truncated); truncated = readable & size < 0.99*expected;
   modal_duration_s per grid = most common `round(duration)` (round half to even,
   smallest value wins ties; NA grid is its own group); short = readable & dur < 0.99*modal;
   duplicate = valid & same (grid, start) > 1; **setorder(grid, start)**: C-locale,
   NA first, stable; gap_s (valid rows, per grid) = start - previous end (plain f64
   arithmetic, e.g. 0.298666715621948); gap_before > tol, overlap_before < -tol;
   qc_pass = valid & readable & !truncated & !duplicate.
5. deployments (`pam_read_deployments`): fread, no header, skip 1 if has_header, all
   text, "" and "NA" are NA, trim; columns by position from `deployments.columns`;
   grid = prefix + sprintf("%04d", as.integer(x)) -> non-number gives `G  NA`;
   dates: `.` and `/` -> `-`, strptime-like (trailing text ignored); times: `.` -> `:`,
   seconds dropped; recovery year < 2000 -> deployment year (WARN "Recovery year
   repaired using deployment year for rows: D3"); blank recovery time -> default and
   flagged; latitude/longitude as.numeric (WARN "NAs introduced by coercion").
6. `pam_check_deployments` -> `outputs/deployment_issues.csv`, sorted (grid, start),
   five issue kinds in a fixed order (see R output in scan.py run); WARN "<n> deployment
   issues, see outputs/deployment_issues.csv".
7. match: per recording, deployments of its grid with start <= time <= end;
   **several match -> the first in the sheet's row order** (verified: D3b listed first
   wins over D3 although D3 starts earlier).
8. `audio_summary_per_grid.csv` in sorted-master order; first/last empty when all NA.
9. log lines (also "log" events): stage started; found N audio files (wav/flac);
   read headers for N new files; [WARNs]; summary line; "stage finished in X.XX min
   (N warnings)" (N counts R warnings only, not the deployment-issues line).

## Readers
- WAV: tuneR `readWave(header = TRUE)`: samples = data_length / (channels*bits/8)
  (a huge data length gives 268435.455 s: kept, truncated TRUE); chunk search limits
  (max 5 skips), formats 0/1/3/0xFFFE, bits 8/16/24/32/64; "seems to be corrupted"
  warning when rate*align != bytes/s or channels*bytes != align.
- FLAC: av (ffmpeg) duration; bits always NA; broken FLAC -> unreadable.
- unreadable rows: rate/channels/bits/duration empty, readable FALSE.

## Writing (data.table fwrite)
- logical TRUE/FALSE, NA -> empty, numbers 15 significant digits trimmed.
- POSIXct: UTC ISO `2026-04-15T03:27:29Z`; fraction m = int(frac*1e7); m += m%10;
  m /= 10; m%1000==0 -> 3 digits (`.455`), else 6 (`.701333`).
- LF line endings? check (files above looked LF).

## scan event (`emit_scan`)
per_grid rows: hours and median_file_min rounded to 1; first/last as
`"2026-04-05T00:30:00"` (no Z) or null; NA grid as `""`; deployment_issues: first 50
rows; total_hours ignores durations >= 1e7 and negatives; counts use `%in% TRUE`.
