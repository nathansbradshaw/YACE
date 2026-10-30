# MuScriptor attribution and terms

MuScriptor is developed by **Kyutai × Mirelo**. Credit the original authors
**Simon Rouard, Michael Krause, Axel Roebel, Carl-Johann Simon-Gabriel and
Alexandre Défossez**, *MuScriptor: An Open Model for Multi-Instrument Music Transcription*.

- [Original source](https://github.com/muscriptor/muscriptor/tree/8a1ef88672cc23c26619adaff37ec48d82c0b906), revision `8a1ef88672cc23c26619adaff37ec48d82c0b906`.
- Original code: **MIT**, Copyright (c)2026 Kyutai x Mirelo; full notice in `MuScriptor-MIT.txt`.
- [Original small model](https://huggingface.co/MuScriptor/muscriptor-small), metadata revision `8c127f603b807520fa465c838e9bfee8a91ada4e`.
- Model weights: **CC BY-NC4.0**, noncommercial, with the original model's specific conditions of use retained below. Full CC license in `MuScriptor-CC-BY-NC-4.0.txt`.
- [unifolia ONNX conversion](https://huggingface.co/unifolia/muscriptor-small-onnx/tree/15dd43dd431b7b12c0fb2f771bb57dea0cc2d7fa), revision `15dd43dd431b7b12c0fb2f771bb57dea0cc2d7fa`. The conversion model card credits Kyutai × Mirelo × IRCAM and states that weights are unmodified apart from conversion. YACE downloads those ONNX files unchanged.

Pinned assets:

| File | SHA256 |
| --- | --- |
| `muscriptor-small-cond.onnx` | `bfdd8edb30affe9f085cf145400cbd3b320822946468a42d88664d2cd1d91d6b` |
| `muscriptor-small-lm.onnx` | `88231c3343ecb11f8e3cba4dbf22835e7398122425025a5c00c60e4dfd2e1c66` |

YACE changes: Rust/ORT conditioner and cached greedy decoding; FFmpeg mono16kHz
input; embedded spectral front end; original recording coordinates; forced tie
preludes; finite/shape/hash checks; missing releases bounded by the source and
explicitly marked unknown rather than fabricated10ms holds. Experimental Synth
uses the original full mix with advisory lead-group32 conditioning. It retains
all raw notes, token streams and instrument labels, then excludes only drums
and explicit GM bass programs32–39 from keyboard arrangement. Other melodic
labels remain uncertain. Nominal arrangement priority1 is not model confidence.
No Python runtime is bundled or invoked. No endorsement by the original authors
or converter is implied.

## Original model specific conditions of use

The following text is retained from the original model metadata linked above:

MuScriptor is the result of a research collaboration between Mirelo and Kyutai whose purpose is to transcribe audio to MIDI/music sheet. It is provided primarily for research purposes under the CC BY-NC 4.0 licence supplemented by the below specific conditions of use.
Specific conditions of use: MuScriptor and any generated content by MuScriptor are provided as is without any warranty of any kind, including but not limited to any warranty of non-infringement. Use of MuScriptor and its output must comply with all applicable laws and must not result in, involve, or facilitate any illegal or unauthorized activity. Prohibited uses include, without limitation, inputting music files and transcribing them to MIDI/music sheet without having all the necessary rights, including intellectual property rights, under applicable laws. Accordingly, users of MuScriptor undertake and warrant to have all the necessary rights, including intellectual property rights, in connection with their use of MuScriptor and its output. We disclaim all liability for any non-compliant use and users of MuScriptor shall indemnify, defend, and hold harmless Mirelo and Kyutai from and against any and all claims, damages, losses, liabilities, and expenses (including reasonable attorneys' fees) incurred by Mirelo and/or Kyutai arising out of or resulting from their failure to comply with the terms of the CC BY-NC 4.0 licence and/or these specific conditions of use.
