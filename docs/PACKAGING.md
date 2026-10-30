# macOS packaging

## Build a bundle without weights

```sh
cargo build --locked --release
cargo run --locked --release --bin package-macos
```

The result is `output/YACE.app`, with identifier `dev.yace.studio`. It includes
required license notices and the covered acoustic-model source, but no model
weights. Missing models download to `~/Library/Application Support/YACE/models`
when the user enables downloads. They do not change the signed app.

Use `-- --replace` to update the same output path. The old app is saved as a
recovery ZIP before replacement. Existing complete private bundles can still
read their bundled weights; incomplete bundles use the user cache.
Missing optional lyric/alignment assets also use that cache. Corrupt bundled
assets fail verification without changing the app.

FFmpeg/ffprobe must be installed on the target Mac. They are not included.

## Private bundles with models

```sh
cargo run --locked --release --bin package-macos -- --include-models
```

This verifies and copies the default local model catalogue. Optional
`--high-quality-piano` and `--multilingual-lyrics` require `--include-models`.
They do not download missing weights during packaging.

Keep such bundles private until every included model's redistribution terms
have been reviewed. Checksums verify identity, not permission to share a model.
See [THIRD_PARTY.md](../THIRD_PARTY.md).

## Signing and notarization

The default signature is ad-hoc, suitable for local verification. To request a
Developer ID signature, pass `--signing-identity` with an installed certificate's
identity. The packager enables hardened runtime and timestamps that signature,
then verifies it before publishing the app.

```sh
security find-identity -v -p codesigning
cargo run --locked --release --bin package-macos -- \
  --signing-identity 'Developer ID Application: YOUR IDENTITY'
```

Developer ID signing requires the corresponding private key in the keychain.
Keep signing credentials and certificates outside the repository.

Once signing is available, submit a ZIP of the signed app with Apple's
`notarytool`, wait for acceptance, staple the ticket to the app, and verify it
with `stapler validate` and `spctl --assess`. Use the existing `notarytool`
keychain profile rather than putting credentials in commands or files.

Signing, notarization, a clean-machine launch and a musical playtest are separate
release checks. A successful local bundle build does not complete them.
