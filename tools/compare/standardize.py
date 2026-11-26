"""Run std_state, std_plan and standardize through the R bridge and the Rust
engine on copies of one raw folder and compare what they report and write.

    python tools/compare/standardize.py <pamflow-desktop folder> <work folder> [--r-only]

The raw folder is hand-made: WAV and FLAC in several rates and channel counts
(copied as they are, converted, down-sampled), short memos, non-audio files,
unreadable audio (text, empty, a fake mp3), hidden files, names with and
without a grid and a time stamp, upper-case extensions and nested folders.
Several settings variants are run (folder layouts, rename strategy, prefix,
target format, isolating switches).

Compared: events (log lines, stage, total, std_state, finished; progress events
only by their last value), the plan and the log CSVs byte for byte, the files
written (names; copied files byte for byte; converted files by their decoded
audio, with a note when the bytes differ).
"""
import copy
import hashlib
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from config_save import run  # noqa: E402
from scan import tone, wav_bytes  # noqa: E402

WORK = Path(sys.argv[2])
R_ONLY = "--r-only" in sys.argv


def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", *args], check=True)


def make_raw(base: Path):
    raw = base / "data" / "raw"
    for d in ("A", "B", "C", "D", "A/sub"):
        (raw / d).mkdir(parents=True, exist_ok=True)
    (base / "config").mkdir(parents=True, exist_ok=True)
    (base / "metadata").mkdir(parents=True, exist_ok=True)
    tmp = base / "tmp"
    tmp.mkdir()

    def wav(rel, secs, rate=8000, ch=1, bits=16):
        p = raw / rel
        p.write_bytes(wav_bytes(tone(secs, rate, ch), rate, channels=ch, bits=bits))
        return p

    def flac(rel, secs, rate=8000, ch=1):
        src = tmp / "t.wav"
        src.write_bytes(wav_bytes(tone(secs, rate, ch), rate, channels=ch))
        ffmpeg("-i", str(src), "-c:a", "flac", str(raw / rel))

    wav("A/G0059_20260405_060000.wav", 10)                     # wav, to convert
    flac("A/G0059_20260405_060010.flac", 10)                   # compliant: copied
    wav("A/G0059_20260405_060020.wav", 10, rate=44100, ch=2)   # stereo 44.1 kHz
    wav("A/sub/G0059_20260405_060030.wav", 10, bits=24)        # 24-bit, nested
    flac("B/rec_one.flac", 10, ch=2)                           # stereo flac, no time stamp in the name
    wav("B/SW001_20260406_070000_x.wav", 10)
    wav("B/20260407-080000.wav", 10)                           # no grid, dashed stamp
    wav("B/recorder_260408090000.wav", 10)                     # 12-digit stamp
    wav("B/nodate_file.wav", 10)                               # time from the file's modification time
    flac("D/High96.flac", 10, rate=96000)                      # down-sampled to the target rate
    wav("D/G0001_20260405_000000.WAV", 10)                     # upper-case extension
    wav("C/short.wav", 2)                                      # memo
    flac("C/short.flac", 3)
    (raw / "C/notes.txt").write_text("field notes\n")
    (raw / "C/data.csv").write_text("a,b\n1,2\n")
    (raw / "C/noext").write_bytes(b"x")
    (raw / "C/bad.wav").write_bytes(b"not audio at all\n" * 8)
    (raw / "C/empty.wav").write_bytes(b"")
    (raw / "C/song.mp3").write_bytes(b"ID3" + b"\0" * 200)     # audio extension pamflow cannot read
    (raw / "C/broken.flac").write_bytes(b"fLaC" + b"\0" * 20)
    wav("C/.hidden_G0001_20260405_000100.wav", 10)             # hidden: ignored
    shutil.rmtree(tmp)
    (base / "config" / "config.yml").write_text(
        'paths:\n  project_root: "."\nfilenames:\n  timezone: "Asia/Kolkata"\n'
        "standardization:\n  min_duration_s: 5\n", encoding="utf-8")


VARIANTS = [
    ("default", {}),
    ("grid_date", {"organize_folders": "grid_date"}),
    ("preserve_keep", {"organize_folders": "preserve", "rename_strategy": "keep"}),
    ("date_prefix", {"organize_folders": "date", "prefix": "G0099"}),
    ("wav_flat_noisolate", {"target_format": "wav", "channels": 2, "organize_folders": "flat",
                            "isolate_non_audio": False, "isolate_short": False}),
]


# ---- normalising ------------------------------------------------------------------------

def norm_events(ev, root):
    out, last = [], {}
    for e in ev:
        t = e["type"]
        if t == "started":
            continue
        s = json.dumps(e).replace(root.as_posix(), "<ROOT>").replace(str(root).replace("\\", "\\\\"), "<ROOT>")
        s = re.sub(r"in \d+\.\d\d min", "in <N> min", s)
        e = json.loads(s)
        if t in ("prep", "files"):
            last[t] = e
            continue
        if t == "finished":
            e.pop("minutes", None)
        if t == "std_state":
            for k in ("plan", "last_run"):
                e[k].pop("updated", None)
        out.append(e)
    return out, last


def tree(root: Path, sub: str):
    base = root / sub
    return sorted(p.relative_to(root).as_posix() for p in base.rglob("*") if p.is_file()) if base.exists() else []


def md5_audio(p: Path):
    r = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", str(p), "-f", "md5", "-"], capture_output=True, text=True)
    return r.stdout.strip() or "unreadable"


def pcm_max_diff(a: Path, b: Path):
    """Largest sample difference between two decoded files (None if not comparable)."""
    import numpy as np
    out = []
    for p in (a, b):
        r = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", str(p), "-f", "s16le", "-"], capture_output=True)
        out.append(np.frombuffer(r.stdout, dtype="<i2").astype(np.int32))
    if len(out[0]) != len(out[1]) or not len(out[0]):
        return None
    return int(np.abs(out[0] - out[1]).max())


def diff_json(a, b, path=""):
    if isinstance(a, dict) and isinstance(b, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{path}{k}: only in {'R' if k in a else 'Rust'}")
            else:
                out += diff_json(a[k], b[k], f"{path}{k}.")
        return out
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            return [f"{path[:-1]}: {len(a)} / {len(b)} items"]
        return [d for x, y in zip(a, b) for d in diff_json(x, y, path)]
    if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool) and not isinstance(b, bool):
        return [] if abs(a - b) <= 1e-9 * max(1.0, abs(a)) else [f"{path[:-1]}: R={a} Rust={b}"]
    return [] if a == b else [f"{path[:-1]}: R={json.dumps(a)[:150]} Rust={json.dumps(b)[:150]}"]


def mask_text(data: bytes, root: Path):
    return data.replace(root.as_posix().encode(), b"<ROOT>")


def compare_variant(name, std, base: Path):
    roots = {}
    for eng in ("r", "rust"):
        root = WORK / f"std_{name}_{eng}"
        if root.exists():
            shutil.rmtree(root)
        shutil.copytree(base, root)
        roots[eng] = root
    problems = []
    notes = []
    steps = [("std_state", {}), ("std_plan", {}), ("standardize", {}), ("std_state", {}), ("std_plan", {})]
    for step, (job_name, _) in enumerate(steps):
        res = {}
        for eng in (["r"] if R_ONLY else ["r", "rust"]):
            job = {"job": job_name, "project": str(roots[eng])}
            if std:
                job["std"] = std
            jf = WORK / f"std_job_{eng}.json"
            jf.write_text(json.dumps(job), encoding="utf-8")
            code, ev, err = run(eng, jf)
            events, last = norm_events(ev, roots[eng])
            res[eng] = {"events": events, "last": last, "code": code}
        label = f"{name} #{step + 1} {job_name}"
        if R_ONLY:
            print(f"--- {label}: R exit {res['r']['code']}")
            for e in res["r"]["events"]:
                print("    " + json.dumps(e)[:300])
            print("    last:", json.dumps(res["r"]["last"])[:300])
            continue
        r, s = res["r"], res["rust"]
        if r["code"] != s["code"]:
            problems.append(f"{label}: exit {r['code']} / {s['code']}")
        if len(r["events"]) != len(s["events"]):
            problems.append(f"{label}: {len(r['events'])} / {len(s['events'])} events")
            for e in r["events"]:
                notes.append("R   : " + json.dumps(e)[:200])
            for e in s["events"]:
                notes.append("Rust: " + json.dumps(e)[:200])
        else:
            for i, (a, b) in enumerate(zip(r["events"], s["events"])):
                for d in diff_json(a, b, f"{a['type']}."):
                    problems.append(f"{label} event {i + 1}: {d}")
        for t in ("prep", "files"):
            a, b = r["last"].get(t), s["last"].get(t)
            if (a is None) != (b is None):
                problems.append(f"{label}: last {t} event only in {'R' if a else 'Rust'}")
            elif a is not None:
                for d in diff_json({k: v for k, v in a.items() if k != "rows"}, {k: v for k, v in b.items() if k != "rows"}, f"last {t}."):
                    problems.append(f"{label}: {d}")
    if R_ONLY:
        return 0
    # ---- files
    rr, rs = roots["r"], roots["rust"]
    for sub in ("metadata/00_standardization_plan.csv", "data/standardized/standardization_log.csv"):
        a, b = rr / sub, rs / sub
        if a.exists() != b.exists():
            problems.append(f"{sub}: only {'R' if a.exists() else 'Rust'} wrote it")
        elif a.exists():
            da, db = mask_text(a.read_bytes(), rr), mask_text(b.read_bytes(), rs)
            if da != db:
                la, lb = da.decode(errors="replace").splitlines(), db.decode(errors="replace").splitlines()
                k = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), min(len(la), len(lb)))
                problems.append(f"{sub} line {k + 1}:\n         R:    {la[k] if k < len(la) else '-'}\n         Rust: {lb[k] if k < len(lb) else '-'}")
    for sub in ("data/standardized", "data/isolated"):
        ta, tb = tree(rr, sub), tree(rs, sub)
        if ta != tb:
            problems.append(f"{sub}: only R {sorted(set(ta) - set(tb))[:5]}; only Rust {sorted(set(tb) - set(ta))[:5]}")
        same_bytes = diff_bytes = resampled = 0
        for rel in sorted(set(ta) & set(tb)):
            if rel.endswith("standardization_log.csv"):
                continue
            a, b = rr / rel, rs / rel
            if a.read_bytes() == b.read_bytes():
                same_bytes += 1
            elif rel.endswith((".flac", ".wav")) and md5_audio(a) == md5_audio(b):
                diff_bytes += 1
            elif rel.endswith((".flac", ".wav")) and (pcm_max_diff(a, b) or 99) <= 1:
                # sample-rate conversion: R's audio package and the ffmpeg program round differently by 1
                resampled += 1
            else:
                problems.append(f"{rel}: audio differs ({md5_audio(a)[:12]} / {md5_audio(b)[:12]})")
        if resampled:
            notes.append(f"{sub}: {resampled} down-sampled file(s) differ by at most 1 in the last bit of some samples")
        if diff_bytes:
            notes.append(f"{sub}: {same_bytes} files byte-identical, {diff_bytes} converted files decode identically but differ in bytes")
    print(f"{'ok  ' if not problems else 'DIFF'} {name}")
    for n in notes[:6]:
        print("       note: " + n)
    for p in problems[:25]:
        print("       " + p)
    return len(problems)


def odd_projects():
    """Folders that cannot be standardized, and an empty raw folder."""
    cases = {
        "no_raw_folder": (["audio_raw: \"data/missing\""], False),
        "folders_overlap": (["audio_raw: \"data/raw\"", "audio_standardized: \"data/raw/out\""], True),
        "isolated_inside_raw": (["audio_isolated: \"data/raw/iso\""], True),
        "empty_raw": ([], True),
    }
    bad = 0
    for name, (paths, make_raw_dir) in cases.items():
        roots = {}
        for eng in ("r", "rust"):
            root = WORK / f"std_odd_{name}_{eng}"
            if root.exists():
                shutil.rmtree(root)
            (root / "config").mkdir(parents=True)
            if make_raw_dir:
                (root / "data" / "raw").mkdir(parents=True)
            lines = ["paths:", '  project_root: "."'] + ["  " + p for p in paths]
            lines += ["filenames:", '  timezone: "Asia/Kolkata"']
            (root / "config" / "config.yml").write_text("\n".join(lines) + "\n", encoding="utf-8")
            roots[eng] = root
        problems = []
        for job_name in ("std_state", "std_plan", "standardize"):
            res = {}
            for eng in ("r", "rust"):
                jf = WORK / f"std_job_{eng}.json"
                jf.write_text(json.dumps({"job": job_name, "project": str(roots[eng])}), encoding="utf-8")
                code, ev, err = run(eng, jf)
                events, _ = norm_events(ev, roots[eng])
                res[eng] = (code, events)
            if res["r"][0] != res["rust"][0]:
                problems.append(f"{job_name}: exit {res['r'][0]} / {res['rust'][0]}")
            for d in diff_json(res["r"][1], res["rust"][1], f"{job_name}."):
                problems.append(d)
        for rel in ("metadata/00_standardization_plan.csv", "data/standardized/standardization_log.csv"):
            a, b = roots["r"] / rel, roots["rust"] / rel
            if a.exists() != b.exists() or (a.exists() and mask_text(a.read_bytes(), roots["r"]) != mask_text(b.read_bytes(), roots["rust"])):
                problems.append(f"{rel} differs")
        bad += bool(problems)
        print(f"{'ok  ' if not problems else 'DIFF'} {name}")
        for p in problems[:10]:
            print("       " + p[:300])
    return bad


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    base = WORK / "_raw_base"
    if base.exists():
        shutil.rmtree(base)
    make_raw(base)
    bad = sum(compare_variant(name, std, base) for name, std in VARIANTS)
    bad += odd_projects()
    print(f"\n{bad} difference(s)")


if __name__ == "__main__":
    main()
