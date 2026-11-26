# BirdNET in Rust: spike report

2026-11-26, Windows 11 x64. Question from the handoff (Part 2, risk 3.1): can the
Rust engine run BirdNET `.tflite` models, the global model and users' own
classifiers, with the same results as pamflow's Python runner (birdnet 0.2.16)?

## Verdict

**Go, with the TensorFlow Lite C library.** Rust loads `tensorflowlite_c.dll` and
runs both models. Raw scores match Python to within 7.4e-5, which changes no
detection on the test clips, and it is faster than Python on one thread. The
library is 4.3 MB, so Python and TensorFlow (several GB) can leave the installer.

The pure-Rust runtime (tract) is a **no-go**: it cannot load the models.

## What was tested

| | |
|---|---|
| Models | BirdNET global v2.4 (`model-fp32.tflite`, 6,522 species); a user-trained classifier (`D:\PAM_GNI\models\BirdNet_v3`, 23 species). Both have the same body (input 144,000 samples = 3 s at 48 kHz) and the same 32 operators |
| Audio | 30 s of `G0007_20260415_111458.wav` (seconds 75–105, full of parakeet calls), as 4 clips that each isolate one risk: 48 kHz mono; 32 kHz mono (resampling); stereo (channel mean); 10.5 s (padded last segment) |
| Reference | `make_reference.py` cuts each clip into segments with birdnet's own functions, saves them and runs `tf.lite.Interpreter` (TF 2.21, one thread) for the raw scores. Checked against pamflow's real runner (`pam_birdnet_predict.py`, custom model, min confidence 0.1): **same detections in all 4 clips** (42, 38, 54, 19), confidences within 1.3e-6 (the CSV's 6-decimal rounding) |
| Rust | the same saved segments fed through each runtime; largest absolute difference of the raw scores (logits) |

## Results

### Option 3: tract (pure Rust), no-go

`tract-tflite` 0.23.10 fails at load on `SPLIT_V`. Compared with its operator
registry, 9 of the 32 operators the models use are missing:
`CAST FLOOR_DIV GATHER PACK POW RANGE REVERSE_V2 RFFT2D SPLIT_V`. Most belong to
the spectrogram front end built into the model (`RFFT2D` is the FFT). Adding
them to tract would be a project of its own.

### Option 1: TensorFlow Lite C library, go

Built from TensorFlow v2.21.0 source with CMake and MSVC
(`build_tflite_c.sh`), called from Rust through 14 C functions loaded at run
time (`tflite-try/`, `libloading`). XNNPACK is on, as in Python.

| Model | Clip | Max logit difference | Rust ms/segment | Python ms/segment |
|---|---|---|---|---|
| global v2.4 | 48 kHz mono | 7.4e-5 | 55 | 62 |
| | 32 kHz mono | 3.2e-5 | 52 | 80 |
| | stereo | 3.8e-5 | 55 | 80 |
| | 10.5 s | 4.4e-5 | 51 | 84 |
| custom BirdNet_v3 | 48 kHz mono | 1.0e-5 | 52 | 76 |
| | 32 kHz mono | 6.7e-6 | 54 | 74 |
| | stereo | 7.9e-6 | 54 | 73 |
| | 10.5 s | 6.0e-6 | 55 | 76 |

- **Differences are tiny and stable** (the same on every run). They are float
  rounding from a different kernel build. A logit difference of 7.4e-5 moves a
  confidence by at most 1.9e-5. None of the 222,530 reference scores lies within
  1e-4 of the 0.1 threshold, so no detection changes on these clips. Across a
  whole project, a score sitting within ~2e-5 of the threshold can in rare cases
  fall on the other side; the golden comparison in step 5 should report how
  often that happens.
- **Speed:** one segment at a time, one thread: 51–55 ms against Python's
  62–84 ms. A batch of 8 on one thread: ~60 ms/segment, no gain. 4 threads in
  one interpreter: unsteady (47–130 ms). Speed should come from several files
  in parallel (one interpreter per worker), as birdnet's worker processes do.
- **Size and dependencies:** `tensorflowlite_c.dll` is 4.3 MB and needs only
  Windows and the Microsoft C++ runtime (`MSVCP140`, `VCRUNTIME140*`), which
  must ship with the app or be linked statically (`/MT`).
- Load time: 63 ms (global), 78 ms (custom).

### Option 2: ONNX Runtime

Not tried. Every user classifier would need converting to ONNX, which needs
Python, so it cannot handle "bring your own classifier" (handoff 3.1).

## Build notes (TensorFlow 2.21.0 on Windows)

`build_tflite_c.sh` applies three fixes; without them there is no usable DLL:

1. `-DTENSORFLOW_SOURCE_DIR`: otherwise TFLite's CMake downloads its own
   TensorFlow, and an older one (v2.19.0).
2. `tensorflow/lite/core/model_building.h`: forward-declare
   `class Tensor;`. MSVC binds `friend class Tensor;` to the wrong class
   (error C2248).
3. `tensorflow/lite/CMakeLists.txt`: don't add `TFL_STATIC_LIBRARY_BUILD`
   when building the C library as a DLL. Otherwise the DLL exports nothing.

`TfLiteVersion()` reports "2.19.0": TF 2.21's `lite/version.h` was not updated.
The code is 2.21.0 (tag `a481b102`).

## What birdnet 0.2.16 does before and after the model

Read from the package source; the Rust engine has to match it (step 5 of the
plan). The spike fed birdnet's own segments, so none of this was tested yet.

- **Reading:** soundfile, as float32 (16-bit samples / 32768); stereo becomes
  the mean of the channels.
- **Segments:** 3 s at the file's own sample rate; hop = 3 s − overlap; each
  segment resampled to 48 kHz with `scipy.signal.resample` (FFT method); the
  last one padded with zeros to 144,000 samples.
- **Bandpass:** applied only when its limits differ from the model's
  (0–15,000 Hz), i.e. never with pamflow's defaults. Otherwise a 5th-order
  Butterworth (`scipy.signal.butter` + `lfilter`).
- **Confidence:** `sigmoid(clip(logit + (sensitivity − 1) × 10, −15, 15))`.
- **top_k = None:** every species above the threshold is kept.
- Corrupt or unreadable files are skipped and listed as unprocessed.

## Next steps

1. Linux and macOS builds of the C library: not tried. The same CMake target
   builds there; Linux and macOS DLL export rules differ (the version
   scripts in `lite/c/`).
2. Decide on the C++ runtime: ship the redistributable or link statically.
3. Step 5 of the plan: `pamflow-birdnet` with decoding, an FFT resampler that
   matches scipy's `resample`, segmentation, padding, confidence, top-k and the
   detection and status CSVs; then a golden comparison of a whole project's
   detections against the Python runner.

## Files

`inspect_models.py` (inputs, outputs, operators), `make_reference.py`,
`check_reference.py`, `tract-try/`, `tflite-try/`, `build_tflite_c.sh`,
`results_tflite.txt`. Test clips and reference scores (`data/`) and the
TensorFlow source and build (`tflite/`) are kept locally and not committed.
