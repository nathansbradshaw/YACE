# Contributing

Keep changes small enough to review. Explain the problem, the change and how you
checked it. Include reproduction steps for bugs.

## Build and check

Use the pinned Rust toolchain and install FFmpeg/ffprobe, CMake and a C/C++ compiler.

```sh
cargo fmt --all -- --check
CCACHE_DISABLE=1 cargo test --locked --all-targets
CCACHE_DISABLE=1 cargo clippy --locked --all-targets -- -D warnings
```

Some tests need models, private recordings or signed-in providers. They are
ignored by default and explain their requirements. Run relevant ones explicitly;
do not download recordings or submit provider requests as part of ordinary tests.

Tools under `tools/` have separate manifests and lockfiles. Run the checks in the
tool's README when changing that tool. Keep new implementation work in Rust.

## Write comments people can scan

- Lead with the reason or constraint.
- Use one idea per paragraph. Two short sentences are usually enough.
- Use plain verbs: “keep”, “check”, “skip” and “copy”.
- Keep units, bounds and important format names explicit.
- Explain a technical term when the reader needs it to understand the rule.
- Describe the current behavior. Put debugging history in the change description.
- Keep safety explanations and third-party license notices intact.

For example:

```rust
// Use the saved input on retry. The original file may have moved.
```

Avoid comments that repeat the next line of code, long lists of things a function
does not do, or unexplained claims such as “safe”, “correct” or “production-ready”.

## Protect user files

Keep imports and exports separate from the original song. Preserve undo history
and unknown metadata. New analysis must not silently replace a person's edits.

Use synthetic fixtures or recordings you can share. Do not commit songs, lyrics,
model weights, API keys, local project exports or private review receipts.

## Report security issues

Follow [SECURITY.md](SECURITY.md). Please keep exploit details out of public issues.
