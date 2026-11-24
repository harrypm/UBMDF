#!/bin/sh
# Build libusbtrace.dylib (macOS, needs the Xcode Command Line Tools).
set -e
cd "$(dirname "$0")"
clang -dynamiclib -O1 -Wall -Wno-deprecated-declarations -arch "$(uname -m)" \
  -o libusbtrace.dylib usbtrace.c -framework IOKit -framework CoreFoundation
echo "built $(pwd)/libusbtrace.dylib"
