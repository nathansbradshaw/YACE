# Drum components

The drum analysis view offers timing and frequency-band activity from a drum
stem without additional models. Bands describe spectral activity, not verified
kick, snare or cymbal identities.

## Optional LarsNet models

LarsNet estimates kick, snare, toms, hi-hat and cymbals from an existing drum stem.
It does not separate individual toms or distinguish ride from crash.

The five local ONNX models are optional. YACE does not bundle or automatically
download them. The authors license the weights under **CC-BY-NC-4.0**;
[the weight notice](../tools/larsnet-convert/LARSNET-WEIGHTS-NOTICE.md) retains
attribution and the original terms. The Rust converter has
[separate instructions](../tools/larsnet-convert/README.md).

```sh
cargo run --release --bin separate-drum-parts -- \
  --audio /path/to/drums.wav \
  --models /path/to/local/larsnet-models \
  --output /path/to/new-drum-components
```

The output folder must be new. It contains five stereo WAV files and a receipt
identifying the input and model files. Listen for bleed, missing hits and
artifacts. Component estimates do not change chart notes or approve review flags.
