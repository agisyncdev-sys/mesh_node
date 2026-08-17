package com.example.mesh_ui

import io.flutter.embedding.android.FlutterActivity
import android.os.Bundle

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        try {
            System.loadLibrary("rust_lib_mesh_ui")
        } catch (e: Exception) {
            e.printStackTrace()
        } catch (e: UnsatisfiedLinkError) {
            e.printStackTrace()
        }
    }
}
