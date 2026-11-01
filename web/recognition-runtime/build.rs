use std::{env, path::PathBuf};
fn main() {
    assert_eq!(env::var("TARGET").unwrap(), "wasm32-wasip1");
    for key in ["YACE_WHISPER_SOURCE", "YACE_WHISPER_BUILD", "WASI_SDK_PATH"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let source =
        PathBuf::from(env::var_os("YACE_WHISPER_SOURCE").expect("Pinned whisper.cpp source"));
    let build =
        PathBuf::from(env::var_os("YACE_WHISPER_BUILD").expect("WASI whisper.cpp libraries"));
    let sdk = PathBuf::from(env::var_os("WASI_SDK_PATH").expect("Official WASI SDK"));
    let sysroot = sdk.join("share/wasi-sysroot");
    bindgen::Builder::default()
        .header(source.join("include/whisper.h").to_str().unwrap())
        .clang_arg("--target=wasm32-wasip1")
        .clang_arg("-DWHISPER_SHARED")
        .clang_arg(format!("--sysroot={}", sysroot.display()))
        .clang_arg(format!("-I{}", source.join("ggml/include").display()))
        .allowlist_function("whisper_.*")
        .allowlist_type("whisper_.*")
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .expect("Generate bindings for WASI ABI")
        .write_to_file(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("whisper.rs"))
        .unwrap();
    for dir in [
        build.join("src"),
        build.join("ggml/src"),
        sysroot.join("lib/wasm32-wasip1/eh"),
        sysroot.join("lib/wasm32-wasip1"),
    ] {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    for lib in [
        "whisper",
        "ggml",
        "ggml-cpu",
        "ggml-base",
        "c++",
        "c++abi",
        "unwind",
        "wasi-emulated-signal",
        "wasi-emulated-process-clocks",
        "pthread",
        "dl",
    ] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    println!("cargo:rustc-link-arg=--export=__wasm_call_ctors");
    println!("cargo:rustc-link-arg=--max-memory=2147483648");
}
