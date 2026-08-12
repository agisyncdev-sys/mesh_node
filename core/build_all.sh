#!/bin/bash
# ==============================================================================
# Automated Cross-Compilation & Packaging Pipeline for Mesh Core Engine
# ==============================================================================
set -e

# Directories
WORKSPACE_ROOT=".."
FLUTTER_UI_DIR="$WORKSPACE_ROOT/ui"
TARGET_DIR="target"

echo "=== STARTING CROSS-COMPILATION PIPELINE ==="

# 1. Install Rustup Toolchains
echo "Setting up compilation targets..."
rustup target add x86_64-pc-windows-msvc
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi
rustup target add aarch64-apple-ios

# ==============================================================================
# Windows Target: x86_64-pc-windows-msvc
# ==============================================================================
echo "--- Building Windows Target: x86_64-pc-windows-msvc ---"
# ONNX Runtime on Windows is linked dynamically or via standard system paths.
cargo build --target x86_64-pc-windows-msvc --release

# Package into Flutter Windows directory
mkdir -p "$FLUTTER_UI_DIR/windows/"
cp "$TARGET_DIR/x86_64-pc-windows-msvc/release/mesh_core.dll" "$FLUTTER_UI_DIR/windows/"
echo "✔ Windows binary packaged successfully."

# ==============================================================================
# Android Target: aarch64-linux-android & armv7-linux-androideabi
# ==============================================================================
# Configure Android NDK paths and linker variables.
# In a real CI environment, NDK_HOME (or ANDROID_NDK_HOME) must be set.
# Example: export ANDROID_NDK_HOME=/path/to/android/sdk/ndk/25.x.x
if [ -n "$ANDROID_NDK_HOME" ]; then
    export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
fi

# Define path to extracted ONNX Runtime Android libraries (AAR shared libraries)
# Cargo build script (ort) queries ORT_LIB_DIR for dynamic/static library files.
export ORT_STRATEGY="dynamic"
if [ -n "$ANDROID_AAR_ORT_DIR" ]; then
    export ORT_LIB_DIR="$ANDROID_AAR_ORT_DIR"
fi

echo "--- Building Android Target: aarch64-linux-android (64-bit ARM) ---"
cargo build --target aarch64-linux-android --release
mkdir -p "$FLUTTER_UI_DIR/android/app/src/main/jniLibs/arm64-v8a/"
cp "$TARGET_DIR/aarch64-linux-android/release/libmesh_core.so" "$FLUTTER_UI_DIR/android/app/src/main/jniLibs/arm64-v8a/"
echo "✔ Android ARM64 binary packaged successfully."

echo "--- Building Android Target: armv7-linux-androideabi (32-bit ARM) ---"
cargo build --target armv7-linux-androideabi --release
mkdir -p "$FLUTTER_UI_DIR/android/app/src/main/jniLibs/armeabi-v7a/"
cp "$TARGET_DIR/armv7-linux-androideabi/release/libmesh_core.so" "$FLUTTER_UI_DIR/android/app/src/main/jniLibs/armeabi-v7a/"
echo "✔ Android ARMv7 binary packaged successfully."

# ==============================================================================
# iOS Target: aarch64-apple-ios
# ==============================================================================
# To build for iOS on macOS, we configure linking paths pointing to Xcode SDKs
# and link with the 'onnxruntime-mobile' pod framework folder.
if [ -n "$IOS_POD_ORT_DIR" ]; then
    # Point ORT to iOS CocoaPods framework location
    export ORT_LIB_DIR="$IOS_POD_ORT_DIR"
fi

echo "--- Building iOS Target: aarch64-apple-ios (64-bit ARM iOS Device) ---"
cargo build --target aarch64-apple-ios --release

# Package into Flutter iOS directory
mkdir -p "$FLUTTER_UI_DIR/ios/"
# iOS links static libraries (.a) or dynamic frameworks (.dylib)
cp "$TARGET_DIR/aarch64-apple-ios/release/libmesh_core.a" "$FLUTTER_UI_DIR/ios/"
echo "✔ iOS static library binary packaged successfully."

echo "=== CROSS-COMPILATION PIPELINE COMPLETED SUCCESSFULLY ==="
