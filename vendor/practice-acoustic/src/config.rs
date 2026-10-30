#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct Wav2Vec2ModelConfig {
    pub hidden_size: usize,
    pub num_hidden_layers: usize,
    pub num_attention_heads: usize,
    pub intermediate_size: usize,
    pub conv_dim: Vec<usize>,
    pub conv_kernel: Vec<usize>,
    pub conv_stride: Vec<usize>,
    pub num_conv_pos_embeddings: usize,
    pub num_conv_pos_embedding_groups: usize,
    #[serde(default)]
    pub do_stable_layer_norm: bool,
    pub layer_norm_eps: f64,
    pub vocab_size: usize,
    pub feat_extract_norm: String,
    pub conv_bias: bool,
}
