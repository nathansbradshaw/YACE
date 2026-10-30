# Optional YARG archive helper

This standalone helper is LGPL-3.0-only and is not a dependency of the MIT YACE application. The Rust decoder in `src/archive.rs` adapts YARG.Core `YARGSongFileStream.cs`, `SngHandler/SngFile.cs`, `SngMask.cs` and `SngFileStream.cs` at commit `3beb94e526558134145bcd3e409428f759001a40` (YARG checkout `f5bfbe6996b4af35722d4204a93c6cc5c8f52a45`). Original copyright belongs to YARG.Core contributors. Upstream source: https://github.com/YARC-Official/YARG.Core/tree/3beb94e526558134145bcd3e409428f759001a40/YARG.Core/IO

Changes: safe Rust scalar equivalent of stream deobfuscation and SNG XOR masking; bounded reads, checked lengths, unique flat paths, overlapping-entry rejection, read-only originals, SHA-256 receipts and fixed artist-separated evaluation split. Original algorithms and this adaptation remain covered by LGPL v3. Complete modified source is distributed here; build locally using Cargo. No proprietary runtime or hidden component is needed. See `LICENSE` and `COPYING.GPL-3.0`.

Song archives, extracted recordings, lyrics, charts and private receipts are separate content, not licensed by this helper. They remain in ignored local storage and must never be committed or redistributed through this repository. This tool reads archives already installed by the user; it does not download songs.
