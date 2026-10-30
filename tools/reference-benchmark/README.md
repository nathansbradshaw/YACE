# Private installed-pack preparation

This optional standalone Rust executable inventories and extracts an already installed YARG song pack. It produces source identity receipts and a synchronized full mix for evaluating a generator against a human-authored chart of the **same recording**. It never modifies the archives or downloads songs.

This helper is **LGPL-3.0-only**, separate from the MIT application. See [NOTICE.md](NOTICE.md), [LICENSE](LICENSE), and [COPYING.GPL-3.0](COPYING.GPL-3.0). No extracted music, lyrics, charts, or private manifests may be committed.

```sh
cargo test --offline --manifest-path tools/reference-benchmark/Cargo.toml
cargo run --offline --manifest-path tools/reference-benchmark/Cargo.toml -- inventory "/path/to/installed/pack" "/ignored/new-inventory.json"
cargo run --offline --manifest-path tools/reference-benchmark/Cargo.toml -- select-heldout "/ignored/new-inventory.json" "/ignored/new-blind-batch.json"
cargo run --offline --manifest-path tools/reference-benchmark/Cargo.toml -- prepare-heldout "/ignored/new-blind-batch.json" "/ignored/new-blind-batch-directory"
cargo run --offline --manifest-path tools/reference-benchmark/Cargo.toml -- extract "/path/to/song.yargsong" "/ignored/new-song-directory"
cargo run --offline --manifest-path tools/reference-benchmark/Cargo.toml -- mix "/ignored/new-song-directory" "/ignored/new-song-directory/source.wav"
```

`select-heldout` reads metadata, hashes, file names and track labels only. It requires direct bass/vocal stems and both instrument tracks, sorts by archive SHA-256, then selects the first six distinct artists and genres from the frozen held-out partition. Run it before any held-out model measurements. Keep unsupported outcomes rather than substituting songs with better scores. Preserve the partition and manifest hashes when comparing policies.

`prepare-heldout` verifies every frozen archive hash, then extracts and mixes all six songs in order under `heldout-01` through `heldout-06`. It performs no inference and does not read note targets. Keep the batch under evaluator control until the policy is locked.

For a subsequent full-band evaluation, use `select-fresh INVENTORY EXPOSURE_HISTORY NEW_MANIFEST`. The history JSON requires a nonempty `excluded_artists` array: include every previously exposed artist, including prior held-out batches and separate instrument experiments. This command additionally requires direct guitar/drum stems, all four instrument track labels and `pro_drums=1`. It selects six distinct artists and genres by archive hash, without reading target note values. It refuses insufficient eligibility instead of silently relaxing the rule. The resulting protocol evaluates Expert and Pro only; lower difficulty correlations are deferred. Use the separate MIT [chart benchmark runner](../chart-benchmark/README.md) after extraction.

`mix` requires `ffmpeg` on PATH. Recognized unique game stems are decoded to 44,100 Hz stereo, summed on their original sample clock, and padded with silence at shorter tails. A single global gain limits the peak to 0.95; individual stems are not normalized or retimed. Crowd and preview audio are excluded. Full and split versions of the same instrument, alternate encodings, and byte-identical files in different roles require explicit disambiguation. `song.opus` can be a short backing track rather than the full recording, so it must not silently substitute for this full mix.

`reference-source.json` pins the archive and extracted assets. `source.mix.json` pins every included audio asset, gain, sample count, and resulting mix. The INI delay is recorded but is **not** applied to audio; the chart comparator must apply it when converting authored chart ticks to source time. Metadata title/artist matches alone do not establish recording identity.

Archive parsing rejects unsafe/duplicate paths, overlapping/out-of-range entries, oversized metadata, and unsupported versions. Output destinations must be new. After any interrupted operation, discard the incomplete *new* destination and rerun into another new directory; an existing partial output is never trusted or replaced. Archives are read-only. No encryption key, chart notes, lyric timing, or pitch reference enters audio generation or mixing.

Use only recordings and charts you have permission to process.
Keep extracted content and reports outside the source repository.
