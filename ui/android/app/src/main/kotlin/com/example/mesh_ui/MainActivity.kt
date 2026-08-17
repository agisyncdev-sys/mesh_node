package com.example.mesh_ui

import io.flutter.embedding.android.FlutterActivity

// flutter_rust_bridge loads the native library automatically via the
// JNI plugin mechanism. Do NOT call System.loadLibrary() manually here
// as it conflicts with the Dart-side FFI initialisation and causes
// a white-screen crash on launch.
class MainActivity : FlutterActivity()
