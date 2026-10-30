# Container reader attribution

The YARGSONG wrapper decoding and SNG metadata/listing reader in `src/main.rs`
are adapted into Rust from YARG.Core's `IO/YARGSongFileStream.cs`,
`IO/SngHandler/SngFile.cs`, `SngMask.cs`, and `SngFileStream.cs`.

Source: https://github.com/YARC-Official/YARG.Core

YARG.Core is licensed under GNU LGPL version 3; its license is included in
`LICENSE`. This directory retains the editable Rust implementation. MIDI
parsing uses `midly`; chart comparison and model inference call YACE's native
Rust library. No C# or Python runtime is executed.
