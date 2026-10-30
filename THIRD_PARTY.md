# Upstream provenance and licensing

The application code is MIT. Included and downloaded third-party components
retain their own licenses. No upstream Python program is bundled or invoked.
Research-only source checkouts and private recordings are not included.

| Upstream | Pinned revision | Use and license |
| --- | --- | --- |
| Poly Haven / Rob Tuytel | Original 1K diffuse JPEGs; SHA-256 pins in `assets/surfaces/README.md` | [Blue Metal Plate](https://polyhaven.com/a/blue_metal_plate) and [Green Metal Rust](https://polyhaven.com/a/green_metal_rust), CC0. Unmodified files supply the desktop's worn surfaces; grayscale and exposure are applied in CSS. Source details: `assets/surfaces/README.md`; license: `licenses/CC0-1.0.txt`. |
| zip-rs/zip2 | `zip` 9.1.3, upstream `af2ed2a3e12af1fdd733b1886b3e9067bf510be5` | MIT; complete original notice in `licenses/ZIP-MIT.txt`. Replaces ZIP 7 in the application and converter. NPY/pickle readers handle fallible names; pinned TorchScript conversion supplies its known byte-zero archive offset. |
| Hugging Face Candle | `candle-core` 0.10.2 (`7c7a8c570e6a16ea24cc30a7501c8ffbbdb51680`) and 0.9.2 (`3b39794c14b1de8ba4c2d10d49354d557a415547`) | Supplied Apache-2.0 licenses retained in `vendor/candle-core` and `vendor/candle-core-legacy`. YACE raises both ZIP requirements to 9.1.3 and propagates fallible archive-name errors in NPY/pickle readers. Tensor/model computation sources are unchanged. Each copy's `YACE-NOTICE.md` records changes and omitted Python fixture generators. |
| BS-RoFormer SW, scragnog ONNX conversion | `440487b8300dcd61453cc52ec244a38150b03456` | Optional higher-quality Piano source. Independent Rust frontend and streamed reconstruction; preprocessing/export facts from MIT ZFTurbo/MSS_ONNX_TensorRT. Original weight terms remain unresolved; conversion MIT does not relicense weights. Pinned asset, attribution and full MIT notice: `licenses/BS-RoFormer-SW-NOTICE.md`. Excluded from the default catalog; optional local download/package only. |
| Tackk6/herosaver | `c5ab37d760645cf02edc2746d773dc04f2c3f247` | MIT; `licenses/HeroSaver-MIT.txt`. `src/midi_to_chart.py` GM voices, voice-layer thinning, two-hand priority, density rating; `src/guitar_to_chart.py` gap/chord-tier policy adapted into Rust. |
| ErnestSarna/chartgen | `383d50c03695b7abdec7b451ecd34c0920b4f932` | MIT; `licenses/chartgen-MIT.txt`. Selected policies from `chartgen/transcribe.py`, `reduce.py`, `frets.py`, `lrclib.py`, `chart.py`. Not a port of its whole application or all its measured playability passes. |
| nikhilunni/demucs-rs | `805d6f72ede20282832ac78c634e8767716c0b21` | Apache-2.0; `vendor/demucs-core/LICENSE`. `demucs-core/src` and its manifest copied directly. The application supplies decoding, weight management and orchestration. |
| spotify/basic-pitch | `fa5997af0a8210982619003269994a1be25eddf3` | Apache-2.0, Copyright Spotify AB; `licenses/BasicPitch-APACHE-2.0.txt`. Rust translation of selected inference/note creation code and original ONNX model. |
| Kong et al. / ByteDance high-resolution piano transcription, LanOss/MobiMML ONNX conversion | `7dff58faf160d4c0bf13be48e30e614faecdba72` (ONNX), `0226e74cbc805660e34bbd6a8fed2083890ebb88` (reference inference toolkit) | CC-BY-4.0 weights; credit Qiuqiang Kong, Bochen Li, Xuchen Song, Yuan Wan and Yuxuan Wang. Apache-2.0 reference decoder adapted into Rust. Native inference, original-clock bounds, separate pedal evidence and removal of the fixed six-second note cap are YACE changes. Full attribution, original checkpoint, conversion details and source links: `licenses/Kong-Piano-NOTICE.md`; Apache text: `licenses/Kong-Piano-APACHE-2.0.txt`. |
| Kyutai × Mirelo MuScriptor, unifolia ONNX conversion | `8a1ef88672cc23c26619adaff37ec48d82c0b906` (source), `15dd43dd431b7b12c0fb2f771bb57dea0cc2d7fa` (ONNX) | MIT native-source adaptation; CC-BY-NC-4.0 weights and original model's specific use conditions. Credit Simon Rouard, Michael Krause, Axel Roebel, Carl-Johann Simon-Gabriel and Alexandre Défossez. Full attribution/changes/conditions: `licenses/MuScriptor-NOTICE.md`; full MIT and CC texts included. Experimental Synth uses the full mix, advisory lead conditioning and an explicit non-bass/non-drum selection with all raw evidence preserved. |
| ModernMube/OwnAudioSharp native MT3 core | `4b7cc4f1ea32acb4f0f23723e0fa4508bafa488d` | MIT, Copyright (c)2025 Modernmube; `licenses/OwnAudioSharp-MIT.txt` and `licenses/OwnAudioSharp-MT3-NOTICE.md`. Session/cache driving adapted for experimental `src/mt3.rs`. This source license is established; the separate MT3 checkpoint's lineage/redistribution terms remain unresolved. MT3 weights are outside the application model catalog and packaged assets; production Synth uses MuScriptor instead. |
| lars76/swift-f0 | `d1ba77fe5331310b778ebe126b6cb3656d41d24f` (v0.3.0) | MIT, Copyright (c) 2025–2026 Lars Nieradzik; `licenses/SwiftF0-MIT.txt`. Native Rust ONNX adapter translates selected frame/chunk/silence conventions. Optional model SHA-256 `6385e8c2ebc3872e82c9ff5946228de44cd3be77a750ea53698b7dbfe94b0a22`; raw source-bound evidence is distinct from accepted vocal chart notes. No Python runtime is invoked. |
| thebluescreen/adtof-drums-onnx | `d0e809bb1c1b6fe8a4a00c576852117a0636ab5a` | CC BY-NC-SA 4.0 model; `licenses/ADTOF-NOTICE.md` and license text identify authors, original checkpoint and conversion. Model downloaded separately, unmodified. |
| Magenta Onsets and Frames Drums, thebluescreen/Silly MIDI Tools ONNX conversion | `771fd5529062b9f6f0b58f01cc4b9819db046f4a` (ONNX), `c15687e` (publisher's Magenta source reference), `model.ckpt-569400` (E-GMD checkpoint) | Optional native [ride/crash proposals](docs/DRUM_ARTICULATION.md) for existing source-bound drum hits; model presence alone does not establish chart quality. The ONNX publisher declares Apache-2.0; its complete original `LICENSE` and `NOTICE.md` are retained unchanged as `licenses/OAF-Drums-APACHE-2.0.txt` and `licenses/OAF-Drums-NOTICE.md`. Credit Lee Callender, Curtis Hawthorne and Jesse Engel. SHA-256 `23d4b05ce7e9715bdb4396823d68bc886e8a958fa9f6266c0f7132b78098b521`, 5,966,247 bytes. [Pinned conversion card](https://huggingface.co/thebluescreen/oaf-drums-onnx/blob/771fd5529062b9f6f0b58f01cc4b9819db046f4a/README.md), [pinned publisher notice](https://huggingface.co/thebluescreen/oaf-drums-onnx/blob/771fd5529062b9f6f0b58f01cc4b9819db046f4a/NOTICE.md), [Magenta source/checkpoint links](https://github.com/magenta/magenta/tree/c15687e/magenta/models/onsets_frames_transcription). |
| set-soft/audio_separation | `939723568b2dca203e61cc7294317ba38549964f` | Published Demucs six-stem safetensors conversion used by demucs-rs. Model asset/checksum are pinned separately. Demucs **code** is MIT (`licenses/Demucs-MIT.txt`); the mirror labels weights MIT, but original checkpoint-specific terms require clarification. The author's 2022 weight restriction predates v4 and does not settle these later weights. See the [original Demucs project](https://github.com/facebookresearch/demucs) for the authors' source and model information. |
| opria123/strum | `9f420cb6550284d15188e9b69f27614ee62fa731` | Architecture/reference only. Repository metadata declares MIT but that snapshot has no standalone license file. No STRUM implementation/checkpoint is bundled. |
| opria123/strum model repository | `6f8997f53c6eb04e0cb38f679c593d27ae89ab7f` | Reviewed published checkpoint inventory only. Detailed onset/classifier models are PyTorch `.pt` files, not ONNX; native loading and parity work remain. |
| opria123/octave | `6026796a62405c8a6c52c2a5cb29fb9410d1138e` | MIT, Copyright (c) 2026 opria123; complete notice in `licenses/OCTAVE-MIT.txt`. Rust Dioxus components under `src/studio/` retain adaptations of the upstream component/DOM structure and supported interactions. YACE uses its own stylesheets; the copied OCTAVE stylesheets have been removed. No YARG meshes, textures, fonts, venue assets or upstream Python worker are bundled. |
| pitch-detection (Rust crate) | `0.3.0`, Cargo.lock | MIT/Apache-2.0; YIN/McLeod for bass harmonic refinement and YIN review comparisons. Agreement after refinement is consistency, not independent accuracy. The application recomputes periodicity as 1 minus CMND at the chosen lag, because this version's reported YIN clarity can be negative. |
| rustfft (Rust crate) | `6.4.1`, Cargo.lock | MIT/Apache-2.0; FFT for independent positive-spectral-flux review diagnostics. |
| whisper-rs / whisper.cpp | `whisper-rs 0.16.0`, `whisper-rs-sys 0.15.0`, Cargo.lock | Local English or multilingual ASR and experimental token timestamps through native Rust bindings; no Python runtime. Binding license: Unlicense (`licenses/whisper-rs-UNLICENSE.txt`); native runtime MIT (`licenses/whisper.cpp-MIT.txt`). Metal shaders are embedded on macOS. |
| ggerganov/whisper.cpp model repository | `5359861c739e955e79d9a303bcbc70fb988958b1` | MIT OpenAI Whisper `small.en` weights converted and quantized as `ggml-small.en-q5_1.bin` (190,098,681 bytes); `licenses/Whisper-MIT.txt`. SHA-256 is pinned in `models/manifest.json`; downloaded only when needed or by `models fetch`. [Model card](https://huggingface.co/ggerganov/whisper.cpp/blob/5359861c739e955e79d9a303bcbc70fb988958b1/README.md), [original license](https://github.com/openai/whisper/blob/main/LICENSE). |
| Optional multilingual Whisper `small` | Same model repository revision as above | `ggml-small-q5_1.bin`, 190,085,487 bytes; SHA-256 `ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb`, pinned in `src/lyric_language.rs`. MIT under the same Whisper notice. Spanish/automatic-language recognition only loads this verified model; packaging includes it only with `--multilingual-lyrics`. |
| EntreNota practice-acoustic / Djoe-Denne/wav2vec2-rs | Native model source based on `62aec29d45926f4d8cb626550b80bcda58cbc301` | Optional English supplied-lyrics alignment; `vendor/practice-acoustic/LICENSE` and `NOTICE.md`, MPL-2.0. EntreNota's encoder-normalization fixes are retained; YACE adds bounded-window cancellation/progress. Covered source is included in the vendor directory. The application-side CTC alignment, binding, diagnostics and installer are Rust. |
| facebook/wav2vec2-base-960h | `22aad52d435eb6dbaf354bdad9b0da84ce7d6156` | Apache-2.0 English acoustic model, [official model card](https://huggingface.co/facebook/wav2vec2-base-960h); `licenses/Wav2Vec2-APACHE-2.0.txt` and `licenses/Wav2Vec2-NOTICE.md`. Config, vocabulary and safetensors SHA-256 pins are in `src/vocal_alignment.rs`; the exact normalized config/vocabulary are in `assets/vocal-alignment`. Optional native model files live in `models/vocal-alignment/en`. The Spanish CC-BY-NC model mentioned in upstream documentation is not installed or bundled. |

## Intentional port changes

OAF Drums attribution above records the conversion publisher's license and
checkpoint claims; a converter's license does not relicense an original
checkpoint. Magenta's Apache-2.0 source license and E-GMD's CC-BY-4.0 dataset
license are distinct from checkpoint terms. The original publisher notice is
kept intact, including its checkpoint declaration. YACE's native Rust evaluation
and source-clock/proposal checks are separate application work; the published
ONNX weights are unchanged, and no upstream conversion interpreter is invoked.

The native benchmark also implements Transkun 2.0.1 inference and decoding in
Rust, following [Yujia Yan's Transkun](https://github.com/Yujia-Yan/Transkun)
and the [TuesdayCrowd ONNX export](https://huggingface.co/TuesdayCrowd/transkun-onnx/tree/b44598166715a50d863973a8ecbbdbbb392f08d6).
Copyright (c) 2021 Yujia Yan; the complete MIT notice is in
`licenses/Transkun-MIT.txt`. The six benchmark assets are checksum-pinned in
`src/transkun.rs`; these weights are not part of the default application model catalog.
The optional standalone `tools/reference-benchmark` executable ports the local
archive format reader from YARG.Core revision
`3beb94e526558134145bcd3e409428f759001a40`. Upstream declares LGPL-3.0-or-later;
the standalone adaptation is LGPL-3.0-only. Its complete
notice and licenses are in that tool's directory. It is not linked into the MIT
application. Human-authored charts, recordings and lyrics used for private
evaluation are not redistributed.

- HeroSaver's crash/ride assignments are corrected to green crash / blue ride,
  matching Clone Hero's documented mapping. Hi-hat remains yellow cymbal.
- All time conversion uses piecewise tempo maps and offsets. A transcription's
  absolute audio seconds are never treated as constant-BPM source ticks.
- Drum expert timings are retained; lower tiers thin voice layers without
  silently moving hits onto a straight grid. At most two hand strikes plus a
  kick are retained after grouping classes within 20 ms. Same-class inference
  peaks within 60 ms undergo confidence-ranked suppression. This is not full
  ergonomic validation, and unusually fast real figures still need review.
- Guitar/bass use chartgen's stable pitch/register mapping rather than the
  original interval mapper's repeated edge re-anchoring. Identical input
  pitches map consistently across repeated phrases. Chord and sustain rules
  are a selected subset of chartgen's pipeline.
- Expert bass uses a 55 ms retrigger floor. Stable upper-harmonic candidates
  can be lowered using Rust periodicity and fundamental-energy checks; original
  candidates remain recorded for review and recovery.
- Basic Pitch windowing, inference-onset combination, energy tolerance and
  Melodia recovery are translated into Rust. Recovery maxima are sorted once
  because energy is only removed, avoiding repeated whole-matrix scans.
  FFmpeg resampling differs from upstream librosa/resampy; numerical equivalence
  to the original Python pipeline has not been established.
- INI updates preserve unrelated fields; automatic output never replaces an
  existing authored instrument or writes directly into the source song folder.
- Lyrics matches require exact normalized artist/title plus close duration.
  The port does not remove remix/live qualifiers and then accept a different
  recording. It preserves existing lyrics when a lookup fails.

## Reusing models without Python

Existing exported ONNX/safetensors assets are downloaded and verified directly
by Rust. Original training code need not be executed to use those weights.
STRUM's `.pt` state dictionaries need an actual Rust architecture/weight-loading
implementation or a suitable already-published export. No interpreter fallback
has been added.
# Optional local drum components

The independent Rust LarsNet frontend and constrained TorchScript-tensor-to-ONNX
converter use local weights explicitly licensed CC-BY-NC-4.0 by the official
LarsNet/LARS authors. These weights retain their own noncommercial terms and
are not bundled or automatically fetched by YACE. Five component masks do not
establish physical drum identity or musical correctness. Research, pinned
official distribution, conversion hashes and attribution are recorded in
[docs/DRUM_PARTS.md](docs/DRUM_PARTS.md) and
[tools/larsnet-convert/LARSNET-WEIGHTS-NOTICE.md](tools/larsnet-convert/LARSNET-WEIGHTS-NOTICE.md).
The frontend/converter do not import or execute upstream Python/TorchScript or
copy the unrelated LARS plugin implementation.
