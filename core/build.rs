fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup flutter_rust_bridge code generation
    // This will generate the necessary C headers and Rust bindings
    // when building for the Flutter UI

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;

    // Set up gRPC compilation using the downloaded protoc binary (Windows only)
    if cfg!(windows) {
        let protoc_path = std::path::Path::new(&manifest_dir)
            .join("protoc_bin")
            .join("bin")
            .join("protoc.exe");
        std::env::set_var("PROTOC", protoc_path);
    }

    tonic_build::compile_protos("proto/ring.proto")?;

    // ── ONNX Runtime (ort load-dynamic) ──────────────────────────────────────
    let ort_lib_dir = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .join("onnx_extracted")
        .join("onnxruntime-win-x64-1.17.1")
        .join("lib");

    if cfg!(windows) {
        println!(
            "cargo:rustc-link-search=native={}",
            ort_lib_dir.display()
        );
        println!(
            "cargo:rustc-env=ORT_DYLIB_PATH={}",
            ort_lib_dir.join("onnxruntime.dll").display()
        );
    } else {
        println!(
            "cargo:rustc-env=ORT_DYLIB_PATH=onnxruntime"
        );
    }

    Ok(())
}

