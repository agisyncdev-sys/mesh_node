fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup flutter_rust_bridge code generation
    // This will generate the necessary C headers and Rust bindings
    // when building for the Flutter UI

    // Set up gRPC compilation using the downloaded protoc binary
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let protoc_path = std::path::Path::new(&manifest_dir)
        .join("protoc_bin")
        .join("bin")
        .join("protoc.exe");
    std::env::set_var("PROTOC", protoc_path);

    tonic_build::compile_protos("proto/ring.proto")?;

    // ── ONNX Runtime (ort load-dynamic) ──────────────────────────────────────
    // Point `ort` at the pre-extracted Windows x64 ORT 1.17.1 DLL.
    // `ort` with `load-dynamic` reads ORT_DYLIB_PATH at *compile time* (to know
    // where to dlopen), and the same path must be available at runtime too.
    let ort_lib_dir = std::path::Path::new(&manifest_dir)
        .parent()                          // workspace root
        .unwrap()
        .join("onnx_extracted")
        .join("onnxruntime-win-x64-1.17.1")
        .join("lib");

    // Tell the linker where the import lib lives (Windows needs this)
    println!(
        "cargo:rustc-link-search=native={}",
        ort_lib_dir.display()
    );

    // Tell `ort`'s load-dynamic loader the full path to onnxruntime.dll
    println!(
        "cargo:rustc-env=ORT_DYLIB_PATH={}",
        ort_lib_dir.join("onnxruntime.dll").display()
    );

    Ok(())
}

