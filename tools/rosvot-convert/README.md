# ROSVOT Rust research converter

Experimental, not an approved chart-generation backend. No model downloads,
Python, PyTorch runtime, transcript or reference notes enter inference.

`cargo build --release --manifest-path tools/rosvot-convert/Cargo.toml --locked`

Commands:

- `inspect OFFICIAL_CHECKPOINT --key state_dict`: list tensor shapes.
- `export OFFICIAL_CHECKPOINT NEW_DIRECTORY [--rwbd]`: convert ROSVOT or RWBD.
- `infer STEM GENERATION_REPORT ROSVOT_DIRECTORY RWBD_DIRECTORY NEW_JSON`:
  predict note events using the report's RMVPE contour for pitch conditioning.
- `inspect-onnx PATH`: inspect native runtime input/output contracts.

Only two checksum-pinned legacy checkpoints are accepted. Tensor metadata is
parsed as data, never executed. The exact hashed checkpoint bytes back all tensor
reads. A directory must be new. Recordings, checkpoints and evaluation reports
belong outside public Git history.

Official bundle SHA-256:
`b6055e81315b93415c9bd7fc48e10a28a3da1bea960cab7385483bd7443ba852`.
ROSVOT checkpoint:
`7501fb5f913d971c2f51bcb3063b930027b03206581820a4d2bfdc394c9c3fcb`.
RWBD checkpoint:
`0bc2d42a6d4b7a05436deb937e2deda1c12de49e5687cfda0bdf6a430120dcd2`.

Preprocessing: 24 kHz mono, FFT/window 512, hop 128, periodic Hann,
192-sample reflection padding, magnitude sqrt(re²+im²+1e-9), first 40 bands of
an 80-band Slaney mel bank spanning 30–12000 Hz, log10(max(mel,1e-5)).
Pitch conditioning uses source-clock RMVPE with 50–900 Hz coarse bins and
explicit unvoiced flags. No lyric text is needed. RWBD predicts acoustic boundary
proposals. Boundary thresholds are 0.5/0.85, minimum gap 17 frames, word-reference
radius 8 frames. Inference uses 3000-frame cores with 375-frame context and
16-frame padding. Chunking and native numerics have not been demonstrated
identical to upstream full-song execution. Do not describe them as such.

The initial four-song development comparison showed that unrestricted note
events also chart accompaniment/non-target sound. A fixed ROSVOT/SwiftF0
correction profile retained every original voiced frame. Its eight-recording
validation reduced macro wrong-pitch rates approximately 15% for contours and
10% for assembled notes, below the required 20%. It remains experimental.
Those eight recordings had been evaluated by earlier YACE work and are not
fresh held-out evidence. Generated charts never become reference annotations.

See [attribution and weight provenance](THIRD_PARTY.md).
