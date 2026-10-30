# Candle archive dependency patch

Source: Hugging Face Candle `candle-core` 0.10.2, crates.io package, upstream
commit `7c7a8c570e6a16ea24cc30a7501c8ffbbdb51680`.

The original Apache-2.0 license is included in `LICENSE`. The package declares
MIT OR Apache-2.0; this copy is distributed under its supplied Apache-2.0 terms.

YACE changes the ZIP requirement from 7.2 to 9.1.3 to address
RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369. ZIP 9 returns fallible
archive names, so `src/npy.rs` and `src/pickle.rs` propagate those errors.
Tensor and model-computation sources are unchanged. `Cargo.toml.orig` retains
the upstream workspace manifest.

The package's Python fixture generator is omitted. Its existing synthetic
archive fixtures and Rust tests are retained; no Python is needed to run them.
Registry bookkeeping and the upstream lockfile are omitted. YACE's committed
root and tool lockfiles define the selected dependencies.

The root patches Candle 0.10.2 to this source and retains a separate ZIP 9 patch
for Candle 0.9.2 in `../candle-core-legacy`. The converter retains 0.9.2. No ZIP
advisories are suppressed, and the actual ZIP package retains its upstream
name and version in the lockfiles.
