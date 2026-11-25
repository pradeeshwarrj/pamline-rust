"""Print inputs, outputs and the operator set of .tflite models.

Run with the Python bundled in pamflow Desktop (it has TensorFlow 2.21):
    python spikes/birdnet/inspect_models.py model1.tflite [model2.tflite ...]
"""
import collections
import io
import sys
import contextlib

import tensorflow as tf


def ops_of(path):
    """Operator names and counts, from TFLite's own model analyzer."""
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        tf.lite.experimental.Analyzer.analyze(model_path=path)
    text = buf.getvalue()
    counts = collections.Counter()
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("Op#"):
            # "Op#12 CONV_2D(T#..., ...)"
            name = line.split(" ", 1)[1].split("(", 1)[0]
            counts[name] += 1
    return counts, text


for path in sys.argv[1:]:
    print("=" * 70)
    print(path)
    interp = tf.lite.Interpreter(model_path=path)
    for d in interp.get_input_details():
        print("  input ", d["index"], d["name"], d["shape"].tolist(), d["dtype"].__name__)
    for d in interp.get_output_details():
        print("  output", d["index"], d["name"], d["shape"].tolist(), d["dtype"].__name__)
    counts, text = ops_of(path)
    print("  subgraphs:", text.count("Subgraph#"))
    for name, n in sorted(counts.items()):
        print(f"    {name:32s} {n}")
