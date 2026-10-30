# Acoustic model attribution

English: Meta/Facebook [wav2vec2-base-960h](https://huggingface.co/facebook/wav2vec2-base-960h),
revision `22aad52d435eb6dbaf354bdad9b0da84ce7d6156`, Apache-2.0.

Spanish: Meta/Facebook [wav2vec2-base-10k-voxpopuli-ft-es](https://huggingface.co/facebook/wav2vec2-base-10k-voxpopuli-ft-es),
revision `1f8c1fd6048a71c6c7644224c5a0aa87fb92cc27`, CC-BY-NC-4.0.
The Spanish model is restricted to noncommercial use. EntreNota converts its
checkpoint to safetensors and corrects config token IDs using the supplied
vocabulary; it does not retrain the model. Both are used to align supplied
lyrics against audio in this free personal app.

Native model source and modifications are described in NOTICE.md and licensed
under the included MPL-2.0 LICENSE. Corresponding modified source is in
[`vendor/practice-acoustic/src`](https://github.com/nathansbradshaw/playlist-manager/tree/codex/learn-to-play/vendor/practice-acoustic/src)
of the EntreNota repository.

YACE includes this native model implementation and its MPL-2.0 source/license.
Only the Apache-2.0 **English** model is integrated into YACE's installer and
packager. The Spanish model is not installed, copied or bundled by YACE.
English model assets are pinned in `src/vocal_alignment.rs`; runtime files live
in `models/vocal-alignment/en`, with the exact config/vocabulary sources in
`assets/vocal-alignment`. Native inference is optional and is never loaded by
the model-free generation path.
