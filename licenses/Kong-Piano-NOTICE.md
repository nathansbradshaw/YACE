# High-resolution piano model and native decoder attribution

The downloaded `piano_kong.onnx` weights are licensed under
[Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/).
The complete license is retained in `Kong-Piano-CC-BY-4.0.txt`.
The original authors are **Qiuqiang Kong, Bochen Li, Xuchen Song, Yuan Wan,
and Yuxuan Wang** (ByteDance), authors of *High-resolution Piano Transcription
with Pedals by Regressing Onset and Offset Times* (IEEE/ACM TASLP, 2021).
[Paper](https://arxiv.org/abs/2010.01815),
[original checkpoint, Zenodo record4034264](https://zenodo.org/records/4034264).

LanOss / MobiMML converted the original
`CRNN_note_F1=0.9677_pedal_F1=0.9186.pth` checkpoint to ONNX opset17,
flattening dictionary outputs; its model card states that the weights were
not modified or fine-tuned. YACE downloads this ONNX file unchanged.

- [Pinned conversion and attribution](https://huggingface.co/LanOss/mobimml-piano-transcription/blob/7dff58faf160d4c0bf13be48e30e614faecdba72/README.md)
- Conversion revision: `7dff58faf160d4c0bf13be48e30e614faecdba72`
- SHA256: `6ec4f07640837df2fcd2cede9c540865acff341def93fbd6432548dee169195a`
- Local filename: `piano_kong.onnx`; downloaded separately through the model catalog.

`src/piano_model.rs` translates the reference regression peak decoder and
50%-overlap/quarter-window deframing into Rust. The original
[ByteDance piano transcription source](https://github.com/bytedance/piano_transcription)
declares Apache2.0; its regression implementation is also published in
[Qiuqiang Kong's inference toolkit, revision0226e74cbc805660e34bbd6a8fed2083890ebb88](https://github.com/qiuqiangkong/piano_transcription_inference/tree/0226e74cbc805660e34bbd6a8fed2083890ebb88).
The Apache2.0 license text is retained in `Kong-Piano-APACHE-2.0.txt`.

YACE changes: native Rust/ORT inference; FFmpeg mono16kHz resampling;
finite/shape/hash checks; original recording time and audio-end bounds;
separate pedal evidence; removal of the reference decoder's fixed six-second
note cap in decoder v2. Frame/offset evidence, actual retriggers and recording
end determine key releases. No Python runtime is bundled or invoked.

Attribution does not imply endorsement by the authors or converter. The
model's published benchmark scores do not establish accuracy on a user's
recording or on YACE's game arrangements.
