# OCTAVE stylesheet source

These nine CSS files are unmodified copies from OCTAVE commit
[`6026796a62405c8a6c52c2a5cb29fb9410d1138e`](https://github.com/opria123/octave/tree/6026796a62405c8a6c52c2a5cb29fb9410d1138e),
copyright (c) 2026 opria123, MIT licensed. The full license is retained in
[`licenses/OCTAVE-MIT.txt`](../../licenses/OCTAVE-MIT.txt).

- `main.css` originates at `src/renderer/src/assets/main.css`.
- The other files, including `SettingsModal.css` for the clean-audio panel,
  originate at `src/renderer/src/components/<same-name>`.

Rust components port the corresponding DOM/class structure and interactions;
the project-specific extensions belong in a separate stylesheet so this source
snapshot stays reviewable. `main.css`, rather than the unused template
`base.css`, is the upstream renderer entry point's global stylesheet.

No Three.js/YARG meshes, textures, fonts or venue assets are included here.
Those require their respective separate licenses and notices.
