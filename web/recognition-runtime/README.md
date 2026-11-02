# Local recognition runtime

The static app embeds `web/runtime/yace-recognition.wasm`. It contains the same
whisper.cpp 1.8.3 / ggml source shipped by the native `whisper-rs-sys` 0.15.0
dependency, plus this Rust wrapper. It is single-threaded SIMD CPU code, with
beam search 5, token timestamps, `max_len=1`, split-on-word and temperature 0.
No JavaScript application runtime, model conversion, Emscripten or Python is
used. The browser host supplies clocks and bounded diagnostic writes; filesystem
access is denied. Cancellation terminates the existing dedicated worker.

Recognition is an estimate. Raw segments and confidence are retained. The
shared Rust word decoder retains uncertain text without inventing boundaries.
Missing lyrics remain explicit pitch-only uncertainty when evidence is absent.

Rebuild with the official WASI SDK 34.0, CMake and YACE's pinned Rust toolchain:

```sh
rustup target add wasm32-wasip1 --toolchain 1.97.1
cargo run --locked --manifest-path web/tools/Cargo.toml -- recognition-build \
  /path/to/wasi-sdk-34.0 \
  /path/to/whisper-rs-sys-0.15.0.crate \
  /path/to/new-build-directory
```

The Rust builder verifies the source archive before extraction. Use the crate
already cached by Cargo, or the unmodified registry download at
`https://static.crates.io/crates/whisper-rs-sys/whisper-rs-sys-0.15.0.crate`.
SHA-256: `6986c0fe081241d391f09b9a071fbcbb59720c3563628c3c829057cf69f2a56f`.
The wrapper's dependencies are pinned in its Cargo.lock. Generated ABI bindings
target WASI; the build never changes Cargo's registry sources.

SDK download: https://github.com/WebAssembly/wasi-sdk/releases/tag/wasi-sdk-34
The exercised arm64 macOS archive SHA-256 is
`9c59398106b417f8f14913380fdf0097a8cc0ff4af9eb3ce0065a859e88d49e9`.
Original runtime license: `licenses/whisper.cpp-MIT.txt`; Whisper weights retain
their separate notice. Model bytes are downloaded/cached separately and checked
against the same native catalogue SHA-256 before inference.
