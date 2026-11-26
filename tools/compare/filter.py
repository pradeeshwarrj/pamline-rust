"""Run filter_info and filter through the R bridge and the Rust engine on copies
of the same projects and compare everything they write and report.

    python tools/compare/filter.py <pamflow-desktop folder> <work folder> [real recordings folder] [--r-only] [--only NAME]

Projects (scanned once by R, then copied for each engine):
  example     pamflow's example project (6 short tones, 2 grids), batches of 4
  edge        hand-made awkward files from scan.py (24-bit, float, stereo, FLAC, 44.1 kHz,
              broken files that fail the scan checks), 3 workers, batches of 5
  real        20 s clips cut from three recordings of the real project (48 kHz, mono, 16-bit)
  no_master   not scanned: filter scans first
  disabled    filter: enabled: false
  bad_ffmpeg  ffmpeg_path that does not exist
  odd_order   filter order 3 (must be even)

Steps per project: filter_info, filter, filter again (nothing pending), one filtered
file deleted then filter (only that one is redone), filter_info. Compared: events (timings
and the volatile parts masked), 02_filter_results.csv, 02_filter.done, logs/02_filter.log
(masked), and the filtered files byte for byte.
"""
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from config_save import make_example, run  # noqa: E402
from scan import make_edge  # noqa: E402

WORK = Path(sys.argv[2])
ARGS = sys.argv[3:]
R_ONLY = "--r-only" in ARGS
ONLY = ARGS[ARGS.index("--only") + 1] if "--only" in ARGS else None
REAL = next((Path(a) for a in ARGS if not a.startswith("--") and Path(a).is_dir()), None)
SHEET = ("deployment_id,recorder,grid,habitat,deployer,latitude,longitude,deploy_date,deploy_time,"
         "retrieve_date,retrieve_time\n1,SW1,1,Forest,Asha,7,93,01-04-2026,00:00,30-04-2026,23:00\n")


def set_filter(project: Path, **settings):
    """Add a `filter:` section to the project's settings file."""
    cfg = project / "config" / "config.yml"
    text = cfg.read_text(encoding="utf-8")
    block = "filter:\n" + "".join(f"  {k}: {v}\n" for k, v in settings.items())
    text = re.sub(r"(?m)^filter:\n(  .*\n)*", "", text)
    cfg.write_text(text.rstrip("\n") + "\n" + block, encoding="utf-8")


def r_job(project: Path, name: str):
    jf = WORK / "filt_r_job.json"
    jf.write_text(json.dumps({"job": name, "project": str(project)}), encoding="utf-8")
    return run("r", jf)


def build(name: str, base: Path):
    if name == "example":
        make_example(base)
        set_filter(base, workers=1, chunk_size=4)
    elif name == "edge":
        make_edge(base)
        set_filter(base, workers=3, chunk_size=5)
    elif name == "real":
        std = base / "data" / "standardized"
        (base / "config").mkdir(parents=True)
        (base / "metadata").mkdir(parents=True)
        (base / "metadata" / "deployments.csv").write_text(SHEET, encoding="utf-8")
        (base / "config" / "config.yml").write_text(
            'paths:\n  project_root: "."\nfilenames:\n  timezone: "Asia/Kolkata"\n', encoding="utf-8")
        for i, grid in enumerate(sorted(p for p in REAL.iterdir() if p.is_dir())[:3]):
            src = sorted(grid.glob("*.wav"))[0]
            dest = std / grid.name / src.name
            dest.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-ss", str(600 + i * 60), "-t", "20", "-i", str(src),
                            "-c", "copy", str(dest)], check=True)
        set_filter(base, workers=2, chunk_size=2)
    else:
        make_example(base)
        if name == "no_master":
            set_filter(base, workers=1, chunk_size=3)
        elif name == "disabled":
            set_filter(base, enabled="false")
        elif name == "bad_ffmpeg":
            set_filter(base, ffmpeg_path='"no_such_ffmpeg_program"')
        elif name == "odd_order":
            set_filter(base, order=3)


def mask(text: str, root: Path) -> str:
    text = text.replace(root.as_posix(), "<ROOT>").replace(str(root).replace("\\", "\\\\"), "<ROOT>")
    text = re.sub(r"^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d ", "<TIME> ", text, flags=re.M)
    text = re.sub(r"in \d+\.\d\d min", "in <N> min", text)
    # R's system2() now and then returns nothing for `ffmpeg -version` and logs "using NA" (not reproducible)
    return text.replace("using NA", "using ffmpeg version 8.1.2 Copyright (c) 2000-2026 the FFmpeg developers")


def norm_events(ev, root):
    out = []
    for e in ev:
        if e["type"] == "started":
            continue
        e = json.loads(mask(json.dumps(e), root))
        if e["type"] == "finished":
            e.pop("minutes", None)
        if e["type"] == "scan":
            e.pop("updated", None)
        out.append(e)
    return out


def diff_json(a, b, path=""):
    if isinstance(a, dict) and isinstance(b, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{path}{k}: only in {'R' if k in a else 'Rust'} ({json.dumps((a if k in a else b)[k])[:80]})")
            else:
                out += diff_json(a[k], b[k], f"{path}{k}.")
        return out
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            return [f"{path[:-1]}: {len(a)} / {len(b)} items"]
        return [d for x, y in zip(a, b) for d in diff_json(x, y, path)]
    if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool) and not isinstance(b, bool):
        return [] if abs(a - b) <= 1e-9 * max(1.0, abs(a)) else [f"{path[:-1]}: R={a} Rust={b}"]
    return [] if a == b else [f"{path[:-1]}: R={json.dumps(a)[:140]} Rust={json.dumps(b)[:140]}"]


def files_under(root: Path):
    return {p.relative_to(root).as_posix(): p for p in sorted(root.rglob("*")) if p.is_file()} if root.exists() else {}


def compare(name):
    base = WORK / f"filt_base_{name}"
    if base.exists():
        shutil.rmtree(base)
    build(name, base)
    roots = {}
    for eng in ("r", "rust"):
        root = WORK / f"filt_{name}_{eng}"
        if root.exists():
            shutil.rmtree(root)
        shutil.copytree(base, root)
        # the example project pins project_root to the folder it was made in: point the copy at itself
        cfg = root / "config" / "config.yml"
        cfg.write_text(re.sub(r'(?m)^(\s*project_root:\s*)"[^"]*"', lambda m: f'{m.group(1)}"{root.as_posix()}"',
                              cfg.read_text(encoding="utf-8"), count=1), encoding="utf-8")
        roots[eng] = root
        if name != "no_master":
            code, ev, err = r_job(root, "scan")  # both copies are scanned by R, so they start the same
            if code != 0:
                print(f"      (R scan exit {code}: {mask(err, root)[:200]})")
    problems = []
    steps = ["filter_info", "filter", "filter", "delete", "filter", "filter_info"]
    for i, step in enumerate(steps):
        res = {}
        for eng in (["r"] if R_ONLY else ["r", "rust"]):
            root = roots[eng]
            if step == "delete":
                files = sorted((root / "data" / "filtered").rglob("*.*"))
                files = [f for f in files if f.is_file()]
                if files:
                    files[0].unlink()
                res[eng] = None
                continue
            jf = WORK / f"filt_job_{eng}.json"
            jf.write_text(json.dumps({"job": step, "project": str(root)}), encoding="utf-8")
            code, ev, err = run(eng, jf)
            res[eng] = {"events": norm_events(ev, root), "code": code}
        if step == "delete":
            continue
        label = f"#{i + 1} {step}"
        if R_ONLY:
            print(f"--- {name} {label}: exit {res['r']['code']}")
            for e in res["r"]["events"]:
                print("    " + json.dumps(e)[:330])
            continue
        r, s = res["r"], res["rust"]
        if r["code"] != s["code"]:
            problems.append(f"{label}: exit {r['code']} / {s['code']}")
        if len(r["events"]) != len(s["events"]):
            problems.append(f"{label}: {len(r['events'])} / {len(s['events'])} events")
            problems += [f"   R:    {json.dumps(e)[:200]}" for e in r["events"][:12]]
            problems += [f"   Rust: {json.dumps(e)[:200]}" for e in s["events"][:12]]
        else:
            for k, (a, b) in enumerate(zip(r["events"], s["events"])):
                for d in diff_json(a, b, f"{a['type']}."):
                    problems.append(f"{label} event {k + 1}: {d}")
    if R_ONLY:
        return 0
    rr, rs = roots["r"], roots["rust"]
    for rel in ("metadata/02_filter_results.csv", "metadata/02_filter.done", "logs/02_filter.log", "metadata/audio_master.csv"):
        a, b = rr / rel, rs / rel
        if a.exists() != b.exists():
            problems.append(f"{rel}: only {'R' if a.exists() else 'Rust'} wrote it")
        elif a.exists():
            da = mask(a.read_text(encoding="utf-8"), rr) if rel.endswith(("log", "csv", "done")) else ""
            db = mask(b.read_text(encoding="utf-8"), rs) if rel.endswith(("log", "csv", "done")) else ""
            if rel.endswith("audio_master.csv"):
                da, db = da.replace("\r", ""), db.replace("\r", "")
            if da != db:
                la, lb = da.splitlines(), db.splitlines()
                k = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), min(len(la), len(lb)))
                problems.append(f"{rel} line {k + 1}:\n         R:    {la[k] if k < len(la) else '-'}\n         Rust: {lb[k] if k < len(lb) else '-'}")
    fa, fb = files_under(rr / "data" / "filtered"), files_under(rs / "data" / "filtered")
    if set(fa) != set(fb):
        problems.append(f"filtered files: only R {sorted(set(fa) - set(fb))[:4]}; only Rust {sorted(set(fb) - set(fa))[:4]}")
    same = diff = 0
    for rel in sorted(set(fa) & set(fb)):
        if fa[rel].read_bytes() == fb[rel].read_bytes():
            same += 1
        else:
            diff += 1
            problems.append(f"filtered file differs: {rel} ({fa[rel].stat().st_size} / {fb[rel].stat().st_size} bytes)")
    print(f"{'ok  ' if not problems else 'DIFF'} {name}: {same} filtered files byte-identical" + (f", {diff} differ" if diff else ""))
    for p in problems[:30]:
        print("       " + p)
    return len(problems)


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    names = ["example", "edge", "no_master", "disabled", "bad_ffmpeg", "odd_order"]
    if REAL:
        names.insert(2, "real")
    bad = sum(compare(n) for n in names if not ONLY or n == ONLY)
    print(f"\n{bad} difference(s)")


if __name__ == "__main__":
    main()
