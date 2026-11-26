"""Run the inspect job through the R bridge and the Rust engine and compare
the events field by field.

    python tools/compare/inspect.py <pamflow-desktop folder> <work folder> [project folder ...]

Projects: pamflow's example project, a folder without settings (no_config),
a project whose settings pamflow refuses, plus each project folder given
(inspect only reads, so real projects can be used as they are). Numbers are
compared as numbers (1 == 1.0); everything else exactly.
"""
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from config_save import make_example, run  # noqa: E402  (same folder)

WORK = Path(sys.argv[2])
EXTRA = [Path(p) for p in sys.argv[3:]]


def same(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= 1e-9 * max(1.0, abs(a))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    return a == b


def diff(r, s, path=""):
    out = []
    if isinstance(r, dict) and isinstance(s, dict):
        for k in sorted(set(r) | set(s)):
            if k not in r or k not in s:
                out.append(f"{path}{k}: only in {'R' if k in r else 'Rust'} ({(r if k in r else s)[k]!r})")
            else:
                out += diff(r[k], s[k], f"{path}{k}.")
    elif not same(r, s):
        out.append(f"{path[:-1]}: R={json.dumps(r)[:120]} Rust={json.dumps(s)[:120]}")
    return out


def settings_by_key(ev):
    return {f"{x['section']}.{x['key']}": x for x in ev.get("settings", [])}


def compare(name, project):
    job = {"job": "inspect", "project": str(project)}
    jf = WORK / f"inspect_{name}.json"
    jf.write_text(json.dumps(job), encoding="utf-8")
    res = {}
    for eng in ("r", "rust"):
        code, ev, err = run(eng, jf)
        res[eng] = {e["type"]: e for e in ev if e["type"] != "started"}
        res[eng]["_code"] = code
    r, s = res["r"], res["rust"]
    problems = []
    if set(r) != set(s):
        problems.append(f"events: R={sorted(set(r) - {'_code'})} Rust={sorted(set(s) - {'_code'})}")
    for t in ("no_config", "project"):
        if t in r and t in s:
            a, b = dict(r[t]), dict(s[t])
            if t == "project":
                ra, sb = settings_by_key(a), settings_by_key(b)
                a.pop("settings", None)
                b.pop("settings", None)
                problems += diff(ra, sb, "settings.")
            problems += diff(a, b, f"{t}.")
    fr, fs = r.get("finished", {}), s.get("finished", {})
    if fr.get("status") != fs.get("status"):
        problems.append(f"finished: R={fr} Rust={fs}")
    print(f"{'ok  ' if not problems else 'DIFF'} {name}: status {fr.get('status')}/{fs.get('status')}"
          f" exit {r['_code']}/{s['_code']}")
    if fr.get("status") != "ok":
        print(f"       R:    {fr.get('message', '')[:160]!r}")
        print(f"       Rust: {fs.get('message', '')[:160]!r}")
    for p in problems:
        print("       " + p)
    return len(problems)


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    ex = WORK / "_example"
    if not ex.exists():
        make_example(ex)
    empty = WORK / "_no_config"
    empty.mkdir(exist_ok=True)
    bad = WORK / "_bad_settings"
    if bad.exists():
        shutil.rmtree(bad)
    shutil.copytree(ex, bad)
    cfg = bad / "config" / "config.yml"
    text = cfg.read_text(encoding="utf-8")
    assert 'timezone: "Asia/Kolkata"' in text, "example settings changed; pick another line to break"
    cfg.write_text(text.replace('timezone: "Asia/Kolkata"', 'timezone: "Mars/Base"', 1), encoding="utf-8")
    cases = [("example", ex), ("no_config", empty), ("bad_settings", bad)]
    cases += [(p.name, p) for p in EXTRA]
    n = sum(compare(name, p) for name, p in cases)
    print(f"\n{n} difference(s)")
    sys.exit(1 if n else 0)


if __name__ == "__main__":
    main()
