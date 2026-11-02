//! Rebuild the browser runtime from the exact published native dependency.
use super::{Result, run};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
const SOURCE_SHA: &str = "6986c0fe081241d391f09b9a071fbcbb59720c3563628c3c829057cf69f2a56f";
pub fn build(root: &Path, args: &[String]) -> Result<()> {
    if args.len() != 3 {
        return Err(
            "Use recognition-build SDK CRATE WORK; see web/recognition-runtime/README.md".into(),
        );
    }
    let sdk = PathBuf::from(&args[0]).canonicalize()?;
    let archive = PathBuf::from(&args[1]).canonicalize()?;
    let work = PathBuf::from(&args[2]);
    if !fs::read_to_string(sdk.join("VERSION"))?.starts_with("34.0\n") {
        return Err("Use the official WASI SDK 34.0".into());
    }
    let bytes = fs::read(&archive)?;
    if format!("{:x}", Sha256::digest(&bytes)) != SOURCE_SHA {
        return Err("Native source archive checksum mismatch".into());
    }
    // New output roots keep previous libraries and diagnostic evidence intact.
    fs::create_dir(&work)?;
    let work = work.canonicalize()?;
    let source = work.join("source");
    fs::create_dir(&source)?;
    run(Command::new("tar")
        .args(["-xf"])
        .arg(&archive)
        .arg("--strip-components=1")
        .arg("-C")
        .arg(&source))?;
    let build = work.join("build");
    fs::create_dir(&build)?;
    let cpp = source.join("whisper.cpp");
    run(Command::new("cmake")
        .current_dir(&build)
        .arg("-S")
        .arg(&cpp)
        .arg("-B")
        .arg(&build)
        .arg(format!(
            "-DCMAKE_TOOLCHAIN_FILE={}",
            sdk.join("share/cmake/wasi-sdk-p1.cmake").display()
        ))
        .args([
            "-DCMAKE_BUILD_TYPE=Release",
            "-DBUILD_SHARED_LIBS=OFF",
            "-DWHISPER_BUILD_EXAMPLES=OFF",
            "-DWHISPER_BUILD_TESTS=OFF",
            "-DGGML_OPENMP=OFF",
            "-DGGML_METAL=OFF",
            "-DGGML_NATIVE=OFF",
            "-DGGML_BLAS=OFF",
            "-DGGML_CCACHE=OFF",
            "-DWHISPER_CCACHE=OFF",
            "-DCMAKE_C_FLAGS=-msimd128 -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS",
            "-DCMAKE_CXX_FLAGS=-msimd128 -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS",
        ]))?;
    run(Command::new("cmake")
        .current_dir(&build)
        .arg("--build")
        .arg(&build)
        .args(["--parallel", "2"]))?;
    let target = work.join("target");
    run(Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip1",
            "--manifest-path",
            "web/recognition-runtime/Cargo.toml",
        ])
        .env("YACE_WHISPER_SOURCE", &cpp)
        .env("YACE_WHISPER_BUILD", &build)
        .env("WASI_SDK_PATH", &sdk)
        .env("CARGO_TARGET_DIR", &target))?;
    let output = root.join("web/runtime/yace-recognition.wasm");
    fs::copy(
        target.join("wasm32-wasip1/release/yace_recognition_runtime.wasm"),
        &output,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(0o644))?;
    }
    println!(
        "Browser recognition runtime SHA-256: {:x}",
        Sha256::digest(fs::read(output)?)
    );
    Ok(())
}
