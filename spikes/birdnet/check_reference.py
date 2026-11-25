"""Check the reference logits against pamflow's own BirdNET runner.

    python -I spikes/birdnet/check_reference.py <data_dir> <model_key> <labels.txt> <runner_detections.csv> <clip> [min_conf]

Applies birdnet's confidence rule (sigmoid of the clipped logit, sensitivity 1)
to <clip>.<model_key>.logits.f32 and compares with the detections CSV written by
pam_birdnet_predict.py for the same clip.
"""
import csv
import sys
from pathlib import Path

import numpy as np

data, key, labels, det_csv, clip = Path(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
min_conf = float(sys.argv[6]) if len(sys.argv) > 6 else 0.1
names = [l.strip() for l in open(labels, encoding="utf-8") if l.strip()]
logits = np.fromfile(data / f"{clip}.{key}.logits.f32", dtype=np.float32).reshape(-1, len(names))
conf = 1.0 / (1.0 + np.exp(-np.clip(logits, -15, 15)))

mine = {}
for seg, sp in zip(*np.nonzero(conf >= min_conf)):
    mine[(seg * 3, names[sp])] = float(conf[seg, sp])
theirs = {}
with open(det_csv, encoding="utf-8") as fh:
    for row in csv.DictReader(fh):
        theirs[(int(round(float(row["start_time"]))), row["species_name"])] = float(row["confidence"])

print(f"reference: {len(mine)} detections, runner: {len(theirs)}")
only_mine = sorted(set(mine) - set(theirs))
only_theirs = sorted(set(theirs) - set(mine))
diffs = [abs(mine[k] - theirs[k]) for k in set(mine) & set(theirs)]
print("only in reference:", only_mine[:10])
print("only in runner:", only_theirs[:10])
print(f"max confidence difference on shared: {max(diffs) if diffs else float('nan'):.2e}")
