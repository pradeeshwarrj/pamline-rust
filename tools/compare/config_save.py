"""Run the same config_save / config_save_as jobs through the R bridge and the
Rust engine, each on its own copy of a project, and compare the results.

    python tools/compare/config_save.py <pamflow-desktop folder> <work folder> [real config.yml ...]

Projects: pamflow's example project (made with pam_example_project()), plus a
copy of each real config.yml given (only the settings file is copied; it gets
a fresh project folder). Compared per case: job status, the saved file byte
for byte, whether a backup was made, the config_saved event (n, and the
config path for save-as) and, for refused saves, both messages are shown.
"""
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

DESKTOP = Path(sys.argv[1])
WORK = Path(sys.argv[2])
REAL = [Path(p) for p in sys.argv[3:]]
ENV_DIR = DESKTOP / "runtime" / ".pixi" / "envs" / "default"
RSCRIPT = ENV_DIR / "lib" / "R" / "bin" / "Rscript.exe"
ENGINE = Path(__file__).resolve().parents[2] / "target" / "debug" / "pamflow-engine.exe"


def r_env():
    """The environment the app gives R: the bundled runtime only (src-tauri runtime.rs
    `apply_env`). Without this R also sees the user's own library, which may hold
    another pamflow version (the app never does)."""
    env = dict(os.environ)
    for var in ("R_HOME", "R_LIBS_SITE", "PYTHONHOME", "PYTHONPATH", "RETICULATE_PYTHON"):
        env.pop(var, None)
    env.update({"R_LIBS_USER": "NULL", "R_LIBS": "", "R_PROFILE_USER": "NULL", "R_ENVIRON_USER": "NULL"})
    extra = [ENV_DIR / "Library" / "mingw-w64" / "bin", ENV_DIR / "Library" / "bin", ENV_DIR / "Scripts",
             ENV_DIR / "lib" / "R" / "bin" / "x64"]
    env["PATH"] = os.pathsep.join(str(p) for p in extra) + os.pathsep + env.get("PATH", "")
    return env


def events(stdout):
    out = []
    for line in stdout.splitlines():
        if line.startswith("@@PAM "):
            out.append(json.loads(line[6:]))
    return out


def run(engine, job_file):
    if engine == "r":
        cmd = [str(RSCRIPT), str(DESKTOP / "bridge" / "run_job.R"), str(job_file)]
        p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", env=r_env(), timeout=600)
    else:
        # the app starts the engine with the runtime's folders first in PATH (so its ffmpeg is used)
        p = subprocess.run([str(ENGINE), str(job_file)], capture_output=True, text=True, encoding="utf-8", env=r_env(), timeout=600)
    return p.returncode, events(p.stdout), p.stderr


def make_example(dest):
    code = f'invisible(pamflow::pam_example_project("{dest.as_posix()}"))'
    subprocess.run([str(RSCRIPT), "-e", code], check=True, env=r_env(), capture_output=True)


def c(section, key, value):
    return {"section": section, "key": key, "value": value}


CASES = [
    ("numbers_and_text", "config_save", [c("birdnet", "min_confidence", 0.35), c("birdnet", "workers", 6),
                                         c("report", "title", 'Nilgiris "PAM" # 2026'), c("birdnet", "top_k", None)]),
    ("lists", "config_save", [c("processing", "species_whitelist", ["Gracula religiosa", "Pitta abbotti"]),
                              c("validation", "bands", [0.3, 0.55, 1]), c("recall", "hours", []),
                              c("validation", "spec_px", [800, 300])]),
    ("indices", "config_save", [c("soundscape", "acoustic_indices", ["NDSI", "ACI"]),
                                c("soundscape", "batch_size", 20)]),
    ("windows_path", "config_save", [c("paths", "audio_raw", "E:\\GNI\\raw data")]),
    ("nothing", "config_save", []),
    ("refused_range", "config_save", [c("birdnet", "min_confidence", 2)]),
    ("refused_timezone", "config_save", [c("filenames", "timezone", "Mars/Base")]),
    ("refused_key", "config_save", [c("birdnet", "python_env", "C:/x")]),
    ("save_as_elsewhere", "config_save_as", [c("birdnet", "run_id", "run02")]),
    ("save_as_config_dir", "config_save_as", [c("occupancy", "occasion_length_days", 7)]),
]


def project_copies(base, name):
    """Two identical copies of the project, one per engine."""
    out = {}
    for eng in ("r", "rust"):
        d = WORK / name / eng
        if d.exists():
            shutil.rmtree(d)
        shutil.copytree(base, d)
        out[eng] = d
    return out


def compare(base_name, base):
    print(f"\n=== {base_name}")
    bad = 0
    for case, job_name, changes in CASES:
        dirs = project_copies(base, f"{base_name}_{case}")
        res = {}
        for eng, d in dirs.items():
            job = {"job": job_name, "project": str(d), "changes": changes}
            if case == "save_as_elsewhere":
                job["target"] = str(d.parent / f"shared_{eng}" / "study")
            if case == "save_as_config_dir":
                job["target"] = str(d / "config" / "other.yml")
            jf = d.parent / f"job_{eng}.json"
            jf.write_text(json.dumps(job), encoding="utf-8")
            code, ev, err = run(eng, jf)
            fin = next((e for e in ev if e["type"] == "finished"), {})
            saved_ev = next((e for e in ev if e["type"] == "config_saved"), None)
            if case == "save_as_elsewhere":
                written = d.parent / f"shared_{eng}" / "study.yml"
            elif case == "save_as_config_dir":
                written = d / "config" / "other.yml"
            else:
                written = d / "config" / "config.yml"
            backups = list((written.parent / "backups").glob("*.bak")) if (written.parent / "backups").exists() else []
            res[eng] = {
                "code": code, "status": fin.get("status"), "message": fin.get("message", ""),
                "bytes": written.read_bytes() if written.exists() else None,
                "n_backups": len(backups), "saved": saved_ev, "stderr": err,
            }
        r, s = res["r"], res["rust"]
        # the saved file: the same bytes, except the pinned project_root, which names each copy's own folder
        rb = (r["bytes"] or b"").replace(str(dirs["r"].as_posix()).encode(), b"<PROJECT>")
        sb = (s["bytes"] or b"").replace(str(dirs["rust"].as_posix()).encode(), b"<PROJECT>")
        checks = {
            "status": (r["status"], s["status"]),
            "file": ("same" if rb == sb else "DIFFERENT", "same"),
            "backups": (r["n_backups"], s["n_backups"]),
            "event n": ((r["saved"] or {}).get("n"), (s["saved"] or {}).get("n")),
        }
        ok = all(a == b for a, b in checks.values())
        bad += not ok
        print(f"{'ok  ' if ok else 'DIFF'} {case:20s} status={r['status']}/{s['status']} backups={r['n_backups']}/{s['n_backups']}")
        if r["status"] != "ok" or s["status"] != "ok" or not ok:
            print(f"       R:    {r['message'][:200]!r}")
            print(f"       Rust: {s['message'][:200]!r}")
        if not ok and rb != sb:
            import difflib
            for line in list(difflib.unified_diff(rb.decode(errors="replace").splitlines(),
                                                  sb.decode(errors="replace").splitlines(), "R", "Rust", lineterm=""))[:20]:
                print("       " + line)
    return bad


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    bases = []
    ex = WORK / "_example"
    if not ex.exists():
        make_example(ex)
    bases.append(("example", ex))
    for i, cfg in enumerate(REAL):
        d = WORK / f"_real{i}"
        if d.exists():
            shutil.rmtree(d)
        (d / "config").mkdir(parents=True)
        shutil.copy(cfg, d / "config" / "config.yml")
        bases.append((f"real{i}", d))
    bad = sum(compare(n, b) for n, b in bases)
    print(f"\n{bad} case(s) differ")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
