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
# MSVC: in model_building.h, `friend class Tensor;` inside Buffer binds to a
# different Tensor because model_builder::Tensor is not declared yet (error
# C2248 in Tensor(Buffer)). Declare it up front, next to `class Graph;`.
MB="$DIR/src/tensorflow/lite/core/model_building.h"
if ! grep -q "^class Tensor;$" "$MB"; then
  sed -i 's/^class Graph;$/class Graph;\nclass Tensor;/' "$MB"
fi
# TENSORFLOW_SOURCE_DIR: without it TFLite's CMake downloads its own pinned
# TensorFlow (v2.19.0 in the 2.21.0 source), a second copy of another version.
# Windows DLL exports: lite/CMakeLists.txt adds TFL_STATIC_LIBRARY_BUILD to
# everything when BUILD_SHARED_LIBS is off, which empties TFL_CAPI_EXPORT, so
# tensorflowlite_c.dll exports nothing. Skip it when the C library is a DLL.
LC="$DIR/src/tensorflow/lite/CMakeLists.txt"
sed -i 's/^if (NOT BUILD_SHARED_LIBS)$/if (NOT BUILD_SHARED_LIBS AND NOT TFLITE_C_BUILD_SHARED_LIBS)/' "$LC"
grep -q "NOT BUILD_SHARED_LIBS AND NOT TFLITE_C_BUILD_SHARED_LIBS" "$LC"
cmake -S "$DIR/src/tensorflow/lite/c" -B "$DIR/build" -A x64 \
  -DTENSORFLOW_SOURCE_DIR="$(cd "$DIR/src" && pwd -W)" \
  -DCMAKE_BUILD_TYPE=Release -DTFLITE_ENABLE_XNNPACK=ON
cmake --build "$DIR/build" --config Release -j 8
find "$DIR/build" -iname "tensorflowlite_c*.dll" -exec ls -la {} \;
