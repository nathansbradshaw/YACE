# Research port attribution

The graph architecture, boundary regulation and acoustic preprocessing are a
Rust translation of [ROSVOT](https://github.com/RickyL-2000/ROSVOT), revision
`3c8332bf43adae35f6e4d64971862f2f6139b310`, copyright 2024 Ruiqi Li,
[MIT license](licenses/ROSVOT-MIT.txt).

The relative attention implementation follows its ESPnet-derived legacy
attention, copyright 2019 Shigeki Karita,
[Apache License 2.0](licenses/Apache-2.0.txt). YACE changes translate operations
to Rust and ONNX, separate note aggregation from the encoder, and bound context.
No upstream executable scripts are run.

The official README publishes the checkpoint bundle and an MIT repository
license. The bundle contains no separate weight license. Its README also
prohibits generating someone's speech/singing without consent. This tool
extracts note proposals; it does not synthesize voices. Checkpoints and generated
ONNX weights remain local research assets, outside public Git and app downloads.
This record is provenance, not a claim of redistribution clearance.
