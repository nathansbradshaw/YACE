# PANNs vocal activity experiment

This Rust research port evaluates acoustic activity only. Tags do not establish
lead singer identity, correct pitches, or one voice per separated stem.

Architecture: Qiuqiang Kong et al., *PANNs: Large-Scale Pretrained Audio Neural
Networks for Audio Pattern Recognition*, official `audioset_tagging_cnn` revision
`d2f4b8c18eab44737fcc0de1248ae21eb43f6aa4`, `pytorch/models.py`, CNN6.
Code is MIT, copyright 2018–2020 Qiuqiang Kong; see LICENSE-PANNs.txt.
The ONNX serializer is shared with YACE's Rust ROSVOT research converter.
The bounded nonexecuting tensor reader is isolated here to preserve the frozen
ROSVOT conversion recipe. Only the pinned CNN6 checkpoint bytes are accepted.

Weights: official Zenodo record 3987831, `Cnn6_mAP=0.343.pth`, 23,698,321 bytes,
publisher MD5 `e25e26b84585b14c7754c91e48efc9be`, SHA-256
`8a17541944f2623a2a0164e072ac2f915dcedca99968083cc44bfbc7eb682275`.
The record API specifies CC-BY-4.0 (verified 2026-11-08); attribution is to
Qiuqiang Kong, Yin Cao, Turab Iqbal, Yuxuan Wang, Wenwu Wang and Mark Plumbley.
https://doi.org/10.5281/zenodo.3987831
https://creativecommons.org/licenses/by/4.0/
The local ONNX conversion is a modified representation of these weights.
Weights are not committed or included in public application assets by this tool.

Frontend: mono 32 kHz, FFT 1024, hop 320, periodic Hann, centred reflection,
power spectrogram, the checkpoint's embedded 513×64 mel matrix, 10 log10 with
1e-10 floor, no top-dB clamp. CNN6 runs on 201-frame windows, global context
reflected at source boundaries, every 50 frames (0.5 seconds). This context
recipe is explicit and not claimed numerically identical to isolated upstream
2-second excerpts. Singing is max AudioSet classes 27–37; voice additionally
includes speech classes 0–3 from the pinned official class index. Raw 527-class
scores are retained. No lyric text or reference pitch is an inference input.

Recorded inference runtime excludes source decoding and mel calculation.
Audio/PCM/model hashes and the exact recipe accompany results. Scores are tags,
not calibrated probabilities that recovered notes are correct. Promotion still
requires separate pitch, false-voicing, exported-note and acceptance checks.
