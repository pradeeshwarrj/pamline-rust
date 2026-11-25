# Make the expected outputs for crates/pamflow-core/tests/golden/config_edit
# with the R config editor of pamflow Desktop. Run from the workspace root with
# the bundled Rscript:
#   Rscript tools/golden/config_edit.R <pamflow-desktop folder>
# The edit cases (*.json) and inputs come from tools/golden/config_edit_cases.py.
args <- commandArgs(trailingOnly = TRUE)
desktop <- if (length(args)) args[1] else "../pamflow-desktop"
source(file.path(desktop, "bridge", "protocol.R"))
source(file.path(desktop, "bridge", "config_edit.R"))
dir <- "crates/pamflow-core/tests/golden/config_edit"
for (f in list.files(dir, pattern = "\\.json$", full.names = TRUE)) {
  case <- jsonlite::fromJSON(f, simplifyVector = FALSE)   # as run_job.R reads `changes`
  lines <- readLines(file.path(dir, case$input), warn = FALSE, encoding = "UTF-8")
  out <- pd_yaml_apply(lines, case$changes)
  pd_write_lines(out, file.path(dir, case$expected), crlf = FALSE)
  cat("wrote", case$expected, "\n")
}
