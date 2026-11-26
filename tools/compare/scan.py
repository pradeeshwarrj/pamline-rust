"""Run the scan and scan_results jobs through the R bridge and the Rust engine
on copies of three projects and compare everything they write and report.

    python tools/compare/scan.py <pamflow-desktop folder> <work folder> [real config.yml] [--only NAME] [--r-only]

Projects (each engine gets its own fresh copy; recordings are only read):
  example  pamflow's example project (6 tones, 2 grids)
  edge     hand-made awkward files: bad names, corrupt and truncated WAVs,
           chunks before fmt/data, 24-bit, stereo, extensible, float, FLAC,
           duplicates, gaps, recordings outside deployments, and a deployment
           sheet with a 2-digit/bad year, blank recovery time, overlaps,
           unparseable rows and a bad latitude
  real     the given config (e.g. D:/PAM_GNI/config/config.yml) with
           project_root moved to the copy, so metadata/, outputs/ and logs/ are
           written there; recordings are read where they are

Compared: audio_headers.csv, audio_master.csv, audio_summary_per_grid.csv,
deployment_issues.csv (byte for byte), the logs/01_scan.log messages
(timestamps and run times masked), and the events (log, stage, scan, finished).
Then a second scan (cache reused) and a forced one.
"""
import json
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from config_save import make_example, run  # noqa: E402

DESKTOP = Path(sys.argv[1])
WORK = Path(sys.argv[2])
REST = sys.argv[3:]
ONLY = REST[REST.index("--only") + 1] if "--only" in REST else None
R_ONLY = "--r-only" in REST
REAL = [a for a in REST if a.endswith(".yml")]


# ---- the edge-case project ---------------------------------------------------

def wav_bytes(samples, rate, channels=1, bits=16, fmt_tag=1, extensible=False, before_fmt=b"", before_data=b"",
              data_len=None, truncate_to=None):
    if bits == 16:
        data = np.asarray(samples, dtype="<i2").tobytes()
    elif bits == 24:
        ints = np.asarray(samples, dtype="<i4")
        data = b"".join(int(v & 0xFFFFFF).to_bytes(3, "little") for v in ints)
    elif bits == 32 and fmt_tag == 3:
        data = np.asarray(samples, dtype="<f4").tobytes()
    else:
        data = np.asarray(samples, dtype="<i4").tobytes()
    block = channels * bits // 8
    if extensible:
        fmt = struct.pack("<HHIIHHHHI", 0xFFFE, channels, rate, rate * block, block, bits, 22, bits, 3 if channels == 2 else 1)
        fmt += bytes([1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71])
    else:
        fmt = struct.pack("<HHIIHH", fmt_tag, channels, rate, rate * block, block, bits)
    body = b"WAVE" + before_fmt + b"fmt " + struct.pack("<I", len(fmt)) + fmt + before_data
    body += b"data" + struct.pack("<I", len(data) if data_len is None else data_len) + data
    out = b"RIFF" + struct.pack("<I", len(body)) + body
    return out[:truncate_to] if truncate_to else out


def tone(seconds, rate, channels=1, freq=440.0, amp=8000):
    t = np.arange(int(seconds * rate)) / rate
    x = np.round(amp * np.sin(2 * np.pi * freq * t)).astype(np.int64)
    return np.repeat(x, channels) if channels > 1 else x


def chunk(tag, payload):
    return tag + struct.pack("<I", len(payload)) + payload


def make_edge(root: Path):
    std = root / "data" / "standardized"
    for d in ("G0001", "G0002", "G0003", "misc", "deep/er"):
        (std / d).mkdir(parents=True, exist_ok=True)
    (root / "metadata").mkdir(parents=True)
    (root / "config").mkdir(parents=True)
    rate = 8000
    files = {
        # grid 1: a run of 10 s files with a gap and an overlap
        "G0001/G0001_20260405_060000.wav": wav_bytes(tone(10, rate), rate),
        "G0001/G0001_20260405_060010.wav": wav_bytes(tone(10, rate), rate),
        "G0001/G0001_20260405_060500.wav": wav_bytes(tone(10, rate), rate),       # gap of ~280 s
        "G0001/G0001_20260405_060505.wav": wav_bytes(tone(10, rate), rate),       # overlaps the previous
        "G0001/G0001_20260405_061000.wav": wav_bytes(tone(4, rate), rate),        # short
        "G0001/G0001_20260405_061100.wav": wav_bytes(tone(10, rate), rate, truncate_to=60000),   # truncated
        "G0001/G0001_20260405_061200.WAV": wav_bytes(tone(10.5, rate), rate),     # upper-case extension
        # grid 2: other formats
        "G0002/G0002_20260406_070000.wav": wav_bytes(tone(10, rate, 2), rate, channels=2),
        "G0002/G0002_20260406_070010.wav": wav_bytes(tone(10, rate), rate, bits=24),
        "G0002/G0002_20260406_070020.wav": wav_bytes(tone(10, rate) / 32768.0, rate, bits=32, fmt_tag=3),
        "G0002/G0002_20260406_070030.wav": wav_bytes(tone(10, rate), rate, extensible=True),
        "G0002/G0002_20260406_070040.wav": wav_bytes(tone(10, rate), rate,
                                                     before_fmt=chunk(b"JUNK", b"\0" * 28),
                                                     before_data=chunk(b"LIST", b"INFOISFT\x05\0\0\0test\0\0")),
        "G0002/G0002_20260406_070050.wav": wav_bytes(tone(10, rate), rate, data_len=0xFFFFFFF0),   # data size too big
        "G0002/G0002_20260406_070100.wav": wav_bytes(tone(10, 44100), 44100),
        # grid 3: same start time twice, and a file outside every deployment
        "G0003/G0003_20260407_080000.wav": wav_bytes(tone(10, rate), rate),
        "deep/er/G0003_20260407_080000.wav": wav_bytes(tone(10, rate, freq=880), rate),
        "G0003/G0003_20260420_080000.wav": wav_bytes(tone(10, rate), rate),
        "G0003/G0003_20260408_080000.wav": wav_bytes(tone(10, rate), rate),       # inside both D3 and D3b
        # names the pattern cannot read, and broken files
        "misc/recording_001.wav": wav_bytes(tone(10, rate), rate),
        "misc/G0001_20261340_000000.wav": wav_bytes(tone(10, rate), rate),        # month 13
        "misc/G0002_20260406_250000.wav": wav_bytes(tone(10, rate), rate),        # hour 25
        "misc/G0001_20260405_070000.wav": b"not audio at all, just text pretending to be a wav\n" * 5,
        "misc/G0001_20260405_070100.wav": b"",                                     # empty
        "misc/G0001_20260405_070200.wav": wav_bytes(tone(10, rate), rate)[:30],   # header cut off
        "misc/.hidden_G0001_20260405_070300.wav": wav_bytes(tone(10, rate), rate),
        "misc/notes.txt": b"not a recording",
    }
    for rel, data in files.items():
        p = std / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
    # FLAC files made with ffmpeg from WAVs
    for rel, secs, ch in (("G0003/G0003_20260407_090000.flac", 10, 1), ("G0003/G0003_20260407_090010.FLAC", 7.25, 2)):
        tmp = root / "tmp.wav"
        tmp.write_bytes(wav_bytes(tone(secs, rate, ch), rate, channels=ch))
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(tmp), "-c:a", "flac", str(std / rel)], check=True)
        tmp.unlink()
    (std / "G0003" / "G0003_20260407_100000.flac").write_bytes(b"fLaC" + b"\0" * 20)   # broken FLAC
    sheet = [
        "deployment_id,recorder,grid,habitat,deployer,latitude,longitude,deploy_date,deploy_time,retrieve_date,retrieve_time",
        "D1,SW1,1,Forest,Asha,11.41,76.70,05-04-2026,05:30,06-04-2026,18:00",
        "D2,SW2,2,Forest,Ravi,11.38,76.73,06/04/2026,06.00,07.04.2026,",          # other separators, blank time
        "D3b,SW4,3,Shola,Asha,11.42,76.72,07-04-2026,12:00,09-04-2026,20:00",     # overlaps D3; listed first
        "D3,SW3,3,Shola,Asha,11.40,76.71,07-04-2026,07:00:00,08-04-0206,20:00",   # seconds, recovery year 0206
        "D4,SW5,4,Tea,Ravi,north,76.70,31-02-2026,06:00,02-03-2026,18:00",        # bad latitude, impossible date
        "D5,SW6,abc,Tea,Ravi,11.1,76.1,,,,",                                       # grid not a number, no dates
        '"D6","SW7"," 1 ","Forest, edge","Asha",11.41,76.70,10-04-2026,06:00,05-04-2026,18:00',   # recovery before deployment
        "",
    ]
    (root / "metadata" / "deployments.csv").write_text("\n".join(sheet) + "\n", encoding="utf-8")
    (root / "config" / "config.yml").write_text(
        "paths:\n  project_root: \".\"\nfilenames:\n  timezone: \"Asia/Kolkata\"\n"
        "qc:\n  gap_tolerance_s: 120\n  header_chunk_size: 7\n", encoding="utf-8")


# ---- running and comparing ----------------------------------------------------------

def make_real(root: Path, cfg_path: Path):
    (root / "config").mkdir(parents=True)
    (root / "metadata").mkdir(parents=True)
    text = cfg_path.read_text(encoding="utf-8")
    text = re.sub(r'(?m)^(\s*project_root:\s*)"[^"]*"', lambda m: f'{m.group(1)}"{root.as_posix()}"', text, count=1)
    (root / "config" / "config.yml").write_text(text, encoding="utf-8")
    src_root = cfg_path.parent.parent
    shutil.copy(src_root / "metadata" / "deployments.csv", root / "metadata" / "deployments.csv")


def mask_log(text, root):
    text = text.replace(root.as_posix(), "<ROOT>")
    text = re.sub(r"^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d ", "<TIME> ", text, flags=re.M)
    return re.sub(r"in \d+\.\d\d min", "in <N> min", text)


def events_of(ev, root):
    out = []
    for e in ev:
        if e["type"] in ("started",):
            continue
        s = json.dumps(e).replace(root.as_posix(), "<ROOT>")
        s = re.sub(r"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d ", "<TIME> ", s)
        s = re.sub(r"in \d+\.\d\d min", "in <N> min", s)
        e = json.loads(s)
        if e["type"] == "finished":
            e.pop("minutes", None)
        if e["type"] == "scan":
            e.pop("updated", None)
        out.append(e)
    return out


OUTPUTS = ["metadata/audio_headers.csv", "metadata/audio_master.csv",
           "outputs/audio_summary_per_grid.csv", "outputs/deployment_issues.csv"]


def snapshot(root):
    snap = {}
    for rel in OUTPUTS:
        p = root / rel
        snap[rel] = p.read_bytes().replace(root.as_posix().encode(), b"<ROOT>") if p.exists() else None
    log = root / "logs" / "01_scan.log"
    snap["logs/01_scan.log"] = mask_log(log.read_text(encoding="utf-8"), root).encode() if log.exists() else None
    return snap


def first_diff(a, b, name):
    if a == b:
        return None
    if a is None or b is None:
        return f"{name}: only {'R' if b is None else 'Rust'} wrote it"
    la, lb = a.decode(errors="replace").splitlines(), b.decode(errors="replace").splitlines()
    for i, (x, y) in enumerate(zip(la, lb)):
        if x != y:
            return f"{name} line {i + 1}:\n         R:    {x[:220]}\n         Rust: {y[:220]}"
    return f"{name}: {len(la)} / {len(lb)} lines"


def compare(name, build):
    roots = {}
    for eng in ("r", "rust"):
        root = WORK / f"scan_{name}_{eng}"
        if root.exists():
            shutil.rmtree(root)
        build(root)
        roots[eng] = root
    bad = 0
    for step, job_extra in (("first scan", {}), ("again (cache)", {}), ("forced", {"force": True}), ("results", None)):
        res = {}
        for eng in (["r"] if R_ONLY else ["r", "rust"]):
            job = {"job": "scan_results" if job_extra is None else "scan", "project": str(roots[eng]), **(job_extra or {})}
            jf = WORK / f"scan_job_{eng}.json"
            jf.write_text(json.dumps(job), encoding="utf-8")
            code, ev, err = run(eng, jf)
            res[eng] = {"events": events_of(ev, roots[eng]), "snap": snapshot(roots[eng]), "code": code, "err": err}
        if R_ONLY:
            print(f"--- {name} {step}: R exit {res['r']['code']}")
            for e in res["r"]["events"]:
                print("    " + json.dumps(e)[:400])
            continue
        problems = [d for k in res["r"]["snap"] if (d := first_diff(res["r"]["snap"][k], res["rust"]["snap"][k], k))]
        er, es = res["r"]["events"], res["rust"]["events"]
        if er != es:
            for i, (a, b) in enumerate(zip(er, es)):
                if a != b:
                    problems.append(f"event {i + 1}:\n         R:    {json.dumps(a)[:600]}\n         Rust: {json.dumps(b)[:600]}")
                    break
            else:
                problems.append(f"events: {len(er)} / {len(es)}")
        bad += bool(problems)
        print(f"{'ok  ' if not problems else 'DIFF'} {name}: {step} (exit {res['r']['code']}/{res['rust']['code']})")
        for p in problems:
            print("       " + p)
    return bad


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    projects = {"example": lambda r: make_example(r), "edge": make_edge}
    for cfg in REAL:
        projects["real"] = lambda r, c=Path(cfg): make_real(r, c)
    bad = 0
    for name, build in projects.items():
        if ONLY and name != ONLY:
            continue
        bad += compare(name, build)
    print(f"\n{bad} step(s) differ")


if __name__ == "__main__":
    main()
