//! Explicit language selection for local vocal recognition. Legacy jobs remain English.
use crate::models::ModelAsset;
use serde::{Deserialize, Serialize};

/// Research profiles remain independent of the production analysis mode.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum RecognitionProfile {
    #[default]
    Small,
    LargeV3,
    LargeV3Turbo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Auto,
    #[default]
    English,
    Spanish,
}

impl Language {
    pub fn research_asset(self, profile: RecognitionProfile) -> ModelAsset {
        let (id, filename, sha256) = match profile {
            RecognitionProfile::Small => return self.asset(),
            RecognitionProfile::LargeV3 => (
                "lyrics-large-v3",
                "ggml-large-v3-q5_0.bin",
                "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
            ),
            RecognitionProfile::LargeV3Turbo => (
                "lyrics-large-v3-turbo",
                "ggml-large-v3-turbo-q5_0.bin",
                "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
            ),
        };
        ModelAsset {
            id: id.into(),
            filename: filename.into(),
            sha256: sha256.into(),
            url: format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/{filename}"
            ),
            license:
                "MIT; OpenAI Whisper weights converted/quantized by whisper.cpp; research only"
                    .into(),
        }
    }
    pub fn code(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::English => Some("en"),
            Self::Spanish => Some("es"),
        }
    }

    pub fn asset(self) -> ModelAsset {
        if self == Self::English {
            crate::models::assets()
                .into_iter()
                .find(|a| a.id == "lyrics")
                .expect("English model")
        } else {
            multilingual_asset()
        }
    }

    /// The English CTC alphabet must never align non-English lyrics by stripping accents.
    pub fn use_english_alignment(self, text: &str) -> bool {
        if self == Self::Spanish {
            return false;
        }
        #[cfg(feature = "vocal-alignment")]
        {
            whatlang::detect(text)
                .is_some_and(|info| info.is_reliable() && info.lang() == whatlang::Lang::Eng)
        }
        #[cfg(not(feature = "vocal-alignment"))]
        {
            let _ = text;
            false
        }
    }
}

pub fn multilingual_asset() -> ModelAsset {
    ModelAsset {
        id: "lyrics-multilingual".into(),
        filename: "ggml-small-q5_1.bin".into(),
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small-q5_1.bin".into(),
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb".into(),
        license: "MIT; OpenAI Whisper weights converted/quantized by whisper.cpp".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spanish_and_auto_cannot_load_the_english_only_model() {
        assert_eq!(Language::English.asset().id, "lyrics");
        for language in [Language::Spanish, Language::Auto] {
            assert_eq!(language.asset().id, "lyrics-multilingual");
            assert!(!language.asset().filename.contains(".en"));
            assert!(!language.use_english_alignment("Dale gracias a la vida y sonríele cada día"));
        }
        assert_eq!(Language::Spanish.code(), Some("es"));
        assert_eq!(Language::Auto.code(), None);
    }

    #[test]
    fn missing_or_corrupt_multilingual_model_never_falls_back_to_english() {
        let temp = tempfile::tempdir().unwrap();
        let control = crate::clean_recognition::Control::default();
        assert!(
            crate::clean_recognition::prepare_model(
                temp.path(),
                false,
                Language::Spanish,
                &control
            )
            .is_err()
        );
        std::fs::write(
            temp.path().join(Language::Spanish.asset().filename),
            b"invalid",
        )
        .unwrap();
        assert!(
            crate::clean_recognition::prepare_model(
                temp.path(),
                false,
                Language::Spanish,
                &control
            )
            .is_err()
        );
    }
}
