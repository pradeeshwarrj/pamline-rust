"""Run example_project, project_create and diagnostics through the R bridge and
the Rust engine and compare what they make.

    python tools/compare/project_jobs.py <pamflow-desktop folder> <work folder>

Each engine works in its own folder; paths are masked before comparing.
example_project: every file byte for byte, the events and the project report.
project_create: the new settings file (default place and elsewhere), the events,
the report, and the refusal when the file exists. diagnostics: event and file
(the contents differ by design: R lists R/Python versions, Rust its own).
"""
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from config_save import run  # noqa: E402
from inspect_job import diff, settings_by_key  # noqa: E402

WORK = Path(sys.argv[2])
bad = 0


def fresh(p):
    if p.exists():
        shutil.rmtree(p)
    p.mkdir(parents=True)
    return p


def run_both(job_for):
    out = {}
    for eng in ("r", "rust"):
        jf = WORK / f"pj_job_{eng}.json"
        jf.write_text(json.dumps(job_for(eng)), encoding="utf-8")
        code, ev, err = run(eng, jf)
        out[eng] = {"code": code, "events": ev, "err": err}
    return out


def by_type(ev):
    return {e["type"]: e for e in ev if e["type"] != "started"}


def masked(obj, folder):
    s = json.dumps(obj)
    for f in (folder.as_posix(), str(folder).replace("\\", "\\\\")):
        s = s.replace(f, "<DIR>")
    return json.loads(s)


def report(name, problems, r_fin, s_fin):
    global bad
    bad += bool(problems)
    print(f"{'ok  ' if not problems else 'DIFF'} {name}: status {r_fin.get('status')}/{s_fin.get('status')}")
    if r_fin.get("status") != "ok" or s_fin.get("status") != "ok":
        print(f"       R:    {r_fin.get('message', '')[:150]!r}")
        print(f"       Rust: {s_fin.get('message', '')[:150]!r}")
    for p in problems:
        print("       " + p)


def compare_events(res, dirs, skip_types=()):
    r = masked(by_type(res["r"]["events"]), dirs["r"])
    s = masked(by_type(res["rust"]["events"]), dirs["rust"])
    problems = []
    if set(r) != set(s):
        problems.append(f"events R={sorted(r)} Rust={sorted(s)}")
    for t in set(r) & set(s) - {"finished"} - set(skip_types):
        a, b = dict(r[t]), dict(s[t])
        if t == "project":
            problems += diff(settings_by_key(a), settings_by_key(b), "settings.")
            a.pop("settings")
            b.pop("settings")
        problems += diff(a, b, f"{t}.")
    if r.get("finished", {}).get("status") != s.get("finished", {}).get("status"):
        problems.append("finished status differs")
    return problems, r.get("finished", {}), s.get("finished", {})


def files_under(root):
    return {p.relative_to(root).as_posix(): p for p in root.rglob("*") if p.is_file()}


def example():
    dirs = {e: fresh(WORK / f"pj_docs_{e}") for e in ("r", "rust")}
    res = run_both(lambda e: {"job": "example_project", "documents_dir": str(dirs[e])})
    problems, rf, sf = compare_events(res, dirs)
    fr, fs = files_under(dirs["r"]), files_under(dirs["rust"])
    if set(fr) != set(fs):
        problems.append(f"files only in R: {sorted(set(fr) - set(fs))}; only in Rust: {sorted(set(fs) - set(fr))}")
    for rel in sorted(set(fr) & set(fs)):
        a = fr[rel].read_bytes().replace(dirs["r"].as_posix().encode(), b"<DIR>")
        b = fs[rel].read_bytes().replace(dirs["rust"].as_posix().encode(), b"<DIR>")
        if a != b:
            problems.append(f"file differs: {rel} ({len(a)} / {len(b)} bytes)")
    report(f"example_project ({len(fr)} files)", problems, rf, sf)
    # a second one goes to "pamflow example 2"
    res = run_both(lambda e: {"job": "example_project", "documents_dir": str(dirs[e])})
    problems, rf, sf = compare_events(res, dirs)
    made = [d.name for d in sorted(dirs["rust"].iterdir())]
    if made != [d.name for d in sorted(dirs["r"].iterdir())]:
        problems.append(f"folders differ: {made}")
    report(f"example_project again -> {made}", problems, rf, sf)


def create(name, target_for):
    dirs = {e: fresh(WORK / f"pj_create_{name}_{e}") for e in ("r", "rust")}
    job = lambda e: {"job": "project_create", "project": str(dirs[e] / "My Project"),
                     **({"target": target_for(dirs[e])} if target_for else {})}
    res = run_both(job)
    problems, rf, sf = compare_events(res, dirs)
    fr, fs = files_under(dirs["r"]), files_under(dirs["rust"])
    if set(fr) != set(fs):
        problems.append(f"files only in R: {sorted(set(fr) - set(fs))}; only in Rust: {sorted(set(fs) - set(fr))}")
    for rel in sorted(set(fr) & set(fs)):
        a = fr[rel].read_text(encoding="utf-8").replace(dirs["r"].as_posix(), "<DIR>").splitlines()
        b = fs[rel].read_text(encoding="utf-8").replace(dirs["rust"].as_posix(), "<DIR>").splitlines()
        for i, (x, y) in enumerate(zip(a, b)):
            if x != y:
                problems.append(f"{rel} line {i + 1}: R={x!r} Rust={y!r}")
        if len(a) != len(b):
            problems.append(f"{rel}: {len(a)} / {len(b)} lines")
    report(f"project_create {name}", problems, rf, sf)
    # again: the file exists now
    res = run_both(job)
    problems, rf, sf = compare_events(res, dirs)
    r_msg = rf.get("message", "").replace(dirs["r"].as_posix(), "<DIR>")
    s_msg = sf.get("message", "").replace(dirs["rust"].as_posix(), "<DIR>")
    if r_msg != s_msg:
        problems.append(f"message R={r_msg!r} Rust={s_msg!r}")
    report(f"project_create {name}, file exists", problems, rf, sf)


def diagnostics():
    dirs = {e: fresh(WORK / f"pj_diag_{e}") for e in ("r", "rust")}
    ex = WORK / "_example"
    res = run_both(lambda e: {"job": "diagnostics", "project": str(ex), "app_version": "0.2.1",
                              "target": str(dirs[e] / "diag.txt")})
    problems, rf, sf = compare_events(res, dirs, skip_types=("diagnostics_saved",))
    for e in ("r", "rust"):
        ev = by_type(res[e]["events"]).get("diagnostics_saved", {})
        if not ev or ev.get("bytes", 0) <= 0:
            problems.append(f"{e}: no diagnostics_saved event")
    report("diagnostics", problems, rf, sf)
    print("       Rust file starts:")
    for line in (dirs["rust"] / "diag.txt").read_text(encoding="utf-8").splitlines()[:24]:
        print("         | " + line)


WORK.mkdir(parents=True, exist_ok=True)
example()
create("default place", None)
create("elsewhere", lambda d: str(d / "shared" / "study"))
diagnostics()
print(f"\n{bad} case(s) differ")
