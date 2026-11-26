#!/usr/bin/env bash
# Build the TensorFlow Lite C library (tensorflowlite_c.dll) from TensorFlow's
# official source, for the BirdNET spike. Same TF version the Python reference
# used (2.21.0). Run from the workspace root; takes a while (fetches abseil,
# eigen, flatbuffers, XNNPACK ... through CMake).
set -euo pipefail
TAG=v2.21.0
DIR=spikes/birdnet/tflite
mkdir -p "$DIR"
if [ ! -d "$DIR/src/.git" ]; then
  git -c core.longpaths=true clone --depth 1 --branch "$TAG" https://github.com/tensorflow/tensorflow.git "$DIR/src"
fi
# TENSORFLOW_SOURCE_DIR: without it TFLite's CMake downloads its own pinned
# TensorFlow (v2.19.0 in the 2.21.0 source), a second copy of another version.
cmake -S "$DIR/src/tensorflow/lite/c" -B "$DIR/build" -A x64 \
  -DTENSORFLOW_SOURCE_DIR="$(cd "$DIR/src" && pwd -W)" \
  -DCMAKE_BUILD_TYPE=Release -DTFLITE_ENABLE_XNNPACK=ON
cmake --build "$DIR/build" --config Release -j 8
find "$DIR/build" -iname "tensorflowlite_c*.dll" -exec ls -la {} \;
