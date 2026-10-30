# Run checks

Install the pinned Rust toolchain, a C/C++ compiler, CMake and FFmpeg/ffprobe.
Run from the repository root:

```sh
cargo fmt --all -- --check
CCACHE_DISABLE=1 cargo test --locked --all-targets
CCACHE_DISABLE=1 cargo clippy --locked --all-targets -- -D warnings
```

For the headless build and desktop UI without the separator:

```sh
CCACHE_DISABLE=1 cargo test --locked --no-default-features --all-targets
CCACHE_DISABLE=1 cargo check --locked --no-default-features --features editor --all-targets
```

Some tests need models, recordings or signed-in providers and are ignored by
default. Read the test's requirements before running it explicitly. Ordinary
tests should not download recordings or submit provider requests.

Each tool under `tools/` has its own manifest and lockfile. Use its README for
tool-specific checks.

## Dependencies

```sh
cargo install cargo-audit --locked
cargo audit --file Cargo.lock
```

Audit the `tools/*/Cargo.lock` files too. Read findings before changing or
suppressing them. A clean scan is one check, not a guarantee of security.

## Check the app

Use shareable fixtures to check import, playback, note editing, Undo/Redo,
project reopening and export. Check keyboard focus, Escape, panel resizing and
the smallest supported window. Test exported parts in the target game and with
a controller before making playability claims.
