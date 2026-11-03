# Symphonia MPEG audio decoder

Upstream: Symphonia 0.5.5, `symphonia-bundle-mp3`, MPL-2.0.
https://github.com/pdeljanov/Symphonia/tree/v0.5.5/symphonia-bundle-mp3

YACE removes only the final float sample clamp in `src/synthesis.rs`. Legitimate
decoded headroom must survive playback preparation and clean-audio processing,
matching the native float decoder. No gain normalization is added. YACE rejects
nonfinite PCM and applies any integer conversion explicitly.

Covered source, upstream headers and the complete license remain here and in
the static distribution's `sources/symphonia-bundle-mp3` directory. This
component remains MPL-2.0; YACE's MIT license does not replace it.
