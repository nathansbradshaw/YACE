# Symphonia Vorbis decoder

Upstream: Symphonia 0.5.5, `symphonia-codec-vorbis`, MPL-2.0.
https://github.com/pdeljanov/Symphonia/tree/v0.5.5/symphonia-codec-vorbis

YACE removes the final sample clamp in `src/dsp.rs`. Vorbis float decoding can
produce legitimate samples above unity. Keeping them matches the native float
decoder and avoids changing source audio before stem subtraction and clean-audio
rendering. No gain normalization is added. Nonfinite decoded audio is rejected
by YACE's audio adapter. Integer playback/export conversion remains explicit.

The modified covered source and complete MPL license are included here and in
the static distribution's `sources/symphonia-codec-vorbis` directory. This
component retains MPL-2.0; YACE's MIT license does not replace it.
