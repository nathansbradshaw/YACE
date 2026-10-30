# Check the pitch probe

```sh
cargo fmt --manifest-path tools/singing-pitch-probe/Cargo.toml -- --check
cargo test --locked --manifest-path tools/singing-pitch-probe/Cargo.toml
cargo clippy --locked --manifest-path tools/singing-pitch-probe/Cargo.toml -- -D warnings
```

Model-dependent tests require the separately acquired, verified model files.
Use shareable input files and keep output outside the source repository.
