"""Python reference for the BirdNET spike.

Run with the Python bundled in pamflow Desktop (birdnet 0.2.16, TensorFlow 2.21):
    python -I spikes/birdnet/make_reference.py <source.wav> <start_s> <out_dir> <model.tflite>...

1. Cuts four test clips from a real recording, each isolating one risk:
     a_48k_mono.wav      48 kHz mono, 30 s       -> model only (no resampling)
     b_32k_mono.wav      32 kHz mono, 30 s       -> FFT resampler
     c_48k_stereo.wav    48 kHz stereo, 30 s     -> channel mean
     d_48k_10.5s.wav     48 kHz mono, 10.5 s     -> padded last segment
2. For every clip: the 3 s segments exactly as birdnet makes them (birdnet's own
   functions), saved as raw little-endian float32 (<clip>.segments.f32,
   n x 144000).
3. For every model and clip: the raw model output (logits, before the sigmoid)
   with tf.lite.Interpreter, one thread (<clip>.<model>.logits.f32, n x n_species),
   plus timing.
Writes manifest.json describing everything.
"""
import json
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

from birdnet.acoustic.inference.core.producer import (
    get_file_segments_with_overlap,
)
from birdnet.utils.helper import fillup_with_silence

SEG_S = 3.0
SR = 48000
SEG_N = int(SEG_S * SR)


def make_clips(source, start_s, out):
    info = sf.info(source)
    sr = info.samplerate
    assert sr == SR, f"expected a 48 kHz source, got {sr}"
    x, _ = sf.read(source, start=int(start_s * sr), stop=int((start_s + 30) * sr), dtype="int16")
    clips = {}
    p = out / "a_48k_mono.wav"
    sf.write(p, x, SR, subtype="PCM_16")
    clips[p.name] = p
    # test input only: any decent resampler will do
    y = resample_poly(x.astype(np.float64), 2, 3)
    p = out / "b_32k_mono.wav"
    sf.write(p, np.clip(np.round(y), -32768, 32767).astype(np.int16), 32000, subtype="PCM_16")
    clips[p.name] = p
    # stereo: right channel delayed by 1 ms and quieter, so the mean differs from either side
    right = np.concatenate([np.zeros(48, dtype=np.int16), (x[:-48] * 0.5).astype(np.int16)])
    p = out / "c_48k_stereo.wav"
    sf.write(p, np.stack([x, right], axis=1), SR, subtype="PCM_16")
    clips[p.name] = p
    p = out / "d_48k_10.5s.wav"
    sf.write(p, x[: int(10.5 * SR)], SR, subtype="PCM_16")
    clips[p.name] = p
    return clips


def segments_of(path):
    info = sf.info(path)
    segs = get_file_segments_with_overlap(
        Path(path),
        audio_n_samples=info.frames,
        sample_rate=info.samplerate,
        segment_duration_s=SEG_S,
        overlap_duration_s=0.0,
        speed=1.0,
        target_sample_rate=SR,
    )
    return np.stack([fillup_with_silence(s, SEG_N) for s in segs]).astype(np.float32)


def run_model(model_path, segments):
    import tensorflow as tf

    interp = tf.lite.Interpreter(model_path=str(model_path), num_threads=1)
    interp.allocate_tensors()
    i_in = interp.get_input_details()[0]["index"]
    i_out = interp.get_output_details()[0]["index"]
    outs = []
    t0 = time.perf_counter()
    for seg in segments:
        interp.set_tensor(i_in, seg[None, :])
        interp.invoke()
        outs.append(interp.get_tensor(i_out)[0].copy())
    dt = (time.perf_counter() - t0) / len(segments)
    return np.stack(outs).astype(np.float32), dt


def main():
    source, start_s, out = sys.argv[1], float(sys.argv[2]), Path(sys.argv[3])
    models = [Path(m) for m in sys.argv[4:]]
    out.mkdir(parents=True, exist_ok=True)
    clips = make_clips(source, start_s, out)
    manifest = {"source": source, "start_s": start_s, "clips": {}, "models": {}}
    for name, path in clips.items():
        segs = segments_of(path)
        segs.tofile(out / f"{name}.segments.f32")
        info = sf.info(path)
        manifest["clips"][name] = {
            "samplerate": info.samplerate, "channels": info.channels,
            "frames": info.frames, "segments": int(segs.shape[0]),
        }
        print(f"{name}: {segs.shape[0]} segments")
    for m in models:
        key = m.stem
        manifest["models"][key] = {"path": str(m), "per_segment_s": {}}
        for name in clips:
            segs = np.fromfile(out / f"{name}.segments.f32", dtype=np.float32).reshape(-1, SEG_N)
            logits, dt = run_model(m, segs)
            logits.tofile(out / f"{name}.{key}.logits.f32")
            manifest["models"][key]["n_species"] = int(logits.shape[1])
            manifest["models"][key]["per_segment_s"][name] = dt
            print(f"{key} on {name}: {dt * 1000:.1f} ms/segment, max logit {logits.max():.3f}")
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1))


if __name__ == "__main__":
    main()
