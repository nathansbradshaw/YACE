//! Thin Rust bindings to the pinned upstream ONNX Runtime WASM distribution.
use anyhow::{Context, Result, ensure};
use js_sys::{Array, Float32Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, prelude::*};
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(raw_module = "./runtime/ort.wasm.min.mjs")]
extern "C" {
    #[wasm_bindgen(js_name = InferenceSession)]
    type InferenceSession;
    #[wasm_bindgen(static_method_of = InferenceSession, js_name = create, catch)]
    fn create(data: &Uint8Array, options: &JsValue) -> std::result::Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = run, catch)]
    fn run_js(this: &InferenceSession, feeds: &JsValue) -> std::result::Result<Promise, JsValue>;
    #[wasm_bindgen(js_name = Tensor)]
    type Tensor;
    #[wasm_bindgen(constructor, catch)]
    fn new(dtype: &str, data: &Float32Array, dims: &Array) -> std::result::Result<Tensor, JsValue>;
    #[wasm_bindgen(thread_local_v2, js_name = env)]
    static ENV: JsValue;
}
fn error(value: JsValue) -> anyhow::Error {
    anyhow::anyhow!("Local ONNX inference: {value:?}")
}
fn set(object: &JsValue, key: &str, value: &JsValue) -> Result<()> {
    Reflect::set(object, &key.into(), value).map_err(error)?;
    Ok(())
}
pub struct Runtime {
    session: InferenceSession,
}
pub struct Output {
    pub shape: Vec<usize>,
    pub values: Vec<f32>,
}
impl Runtime {
    pub async fn load(
        id: &str,
        progress: &mut impl FnMut(String),
        disable_optimization: bool,
    ) -> Result<Self> {
        let weights = crate::model_assets::load(id, progress).await?;
        ENV.with(|env| -> Result<()> {
            let wasm = Reflect::get(env, &"wasm".into()).map_err(error)?;
            set(&wasm, "numThreads", &1.into())?;
            set(&wasm, "proxy", &false.into())?;
            Ok(())
        })?;
        let options = Object::new();
        let providers = Array::new();
        providers.push(&"wasm".into());
        set(&options, "executionProviders", &providers)?;
        if disable_optimization {
            set(&options, "graphOptimizationLevel", &"disabled".into())?;
        }
        progress(format!("Loading {id} transcription model locally"));
        let session = JsFuture::from(
            InferenceSession::create(&Uint8Array::from(weights.as_slice()), &options).map_err(error)?,
        )
        .await
        .map_err(error)?
        .unchecked_into();
        Ok(Self { session })
    }
    pub async fn run(&self, name: &str, values: &[f32], shape: &[usize]) -> Result<JsValue> {
        ensure!(
            shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b)) == Some(values.len()),
            "Invalid input tensor shape"
        );
        ensure!(
            values.iter().all(|v| v.is_finite()),
            "Nonfinite model input"
        );
        let dims = Array::new();
        for dim in shape {
            dims.push(&(*dim as f64).into());
        }
        let tensor = Tensor::new("float32", &Float32Array::from(values), &dims).map_err(error)?;
        let feeds = Object::new();
        set(&feeds, name, &tensor)?;
        JsFuture::from(self.session.run_js(&feeds).map_err(error)?)
            .await
            .map_err(error)
    }
    pub fn output(outputs: &JsValue, name: &str) -> Result<Output> {
        let output = Reflect::get(outputs, &name.into()).map_err(error)?;
        ensure!(!output.is_undefined(), "Model output missing {name}");
        let dims = Reflect::get(&output, &"dims".into()).map_err(error)?;
        let shape = Array::from(&dims)
            .iter()
            .map(|v| {
                let value = v.as_f64().context("Invalid output dimension")?;
                ensure!(
                    value >= 0. && value.fract() == 0. && value <= usize::MAX as f64,
                    "Invalid output dimension"
                );
                Ok(value as usize)
            })
            .collect::<Result<Vec<_>>>()?;
        let data = Reflect::get(&output, &"data".into()).map_err(error)?;
        ensure!(
            data.is_instance_of::<Float32Array>(),
            "Unexpected output type for {name}"
        );
        let values = Float32Array::new(&data).to_vec();
        ensure!(
            shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b)) == Some(values.len())
                && values.iter().all(|v| v.is_finite()),
            "Invalid output tensor for {name}"
        );
        Ok(Output { shape, values })
    }
}
