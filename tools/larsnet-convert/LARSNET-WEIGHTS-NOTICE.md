# Optional LarsNet/LARS model weights

The five original TorchScript models and their locally generated ONNX derivatives remain subject to the authors' **Creative Commons Attribution-NonCommercial4.0 International** terms. They are not covered by YACE's MIT source license and are not bundled or downloaded by default.

LarsNet research: Alessandro Ilic Mezza, Riccardo Giampiccolo, Alberto Bernardini and Augusto Sarti, *Toward Deep Drum Source Separation*, Pattern Recognition Letters183(2024),86–91, DOI[10.1016/j.patrec.2024.04.026](https://doi.org/10.1016/j.patrec.2024.04.026).

Original model publication: [EdoardoMor/LARS](https://github.com/EdoardoMor/LARS), revision `872991160e7c26b1d9be76ddc625d01373727200`, directory `drums_demix/DrumsDemixUtils/DrumsDemixModels`, filenames `my_scripted_module_{kick,snare,toms,hihat,cymbals}.pt`. The [LARS README](https://github.com/EdoardoMor/LARS/blob/872991160e7c26b1d9be76ddc625d01373727200/README.md#license) explicitly identifies its LarsNet weights as CC-BY-NC4.0. The [LarsNet authors' README](https://github.com/polimi-ispl/larsnet/blob/17d631fd18e77ee2f1d23ee7b3fc0fb46ae629e2/README.md#pretrained-larsnet-models-) independently states the pretrained-weight terms.

License text: [CC-BY-NC4.0 legal code](https://creativecommons.org/licenses/by-nc/4.0/legalcode.en).

Modification: YACE independently reconstructs the published magnitude-mask network as ONNX opset17 using the original named tensors. It removes waveform STFT/ISTFT and block folding from the model graph and supplies those operations in Rust. Weights are not retrained. Original and converted hashes are recorded in the converter README and each companion conversion receipt. This notice does not imply author endorsement or upstream numerical/quality certification.

No permission to use unrelated upstream Python or C++ plugin source is inferred from these weight terms. YACE's independent Rust frontend/converter code is MIT; normal dependency licenses remain separate. Keep this notice, original attribution, provenance receipts and weight-license link with any redistributed converted weight files.
