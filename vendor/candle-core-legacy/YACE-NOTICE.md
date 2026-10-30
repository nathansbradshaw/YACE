# Candle archive dependency patch

Source: Hugging Face Candle `candle-core` 0.9.2, crates.io package, upstream
commit `3b39794c14b1de8ba4c2d10d49354d557a415547`.

The supplied Apache-2.0 license is retained in `LICENSE`. This copy retains
the version used by the LarsNet converter and Burn's optional Candle backend.

YACE raises the ZIP requirement from 7.2 to 9.1.3. ZIP 9 returns fallible archive
names, so `src/npy.rs` and `src/pickle.rs` propagate those errors. Tensor and
model-computation sources are unchanged.

The Python fixture generator, registry bookkeeping and upstream lockfile are
omitted. Existing synthetic archive fixtures and Rust tests remain. No model
weights or Python runtime are needed to run those tests. `Cargo.toml.orig`
retains the original workspace manifest; YACE's lockfiles select dependencies.
