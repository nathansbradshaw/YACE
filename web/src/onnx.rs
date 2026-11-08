//! Thin Rust bindings to the pinned upstream ONNX Runtime WASM distribution.
use anyhow::{Context, Result, ensure};
use js_sys::{Array, Float32Array, Float64Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, prelude::*};
use wasm_bindgen_futures::JsFuture;

// The test runner gives .mjs files a non-script MIME type. The test-only .js
// alias packages the same pinned runtime without changing production imports.
#[cfg_attr(
    not(feature = "browser-tests"),
    wasm_bindgen(raw_module = "./runtime/ort.wasm.min.mjs")
)]
#[cfg_attr(
    feature = "browser-tests",
    wasm_bindgen(module = "/tests/ort-runtime.js")
)]
extern "C" {
    #[wasm_bindgen(js_name = InferenceSession)]
    type InferenceSession;
    #[wasm_bindgen(static_method_of = InferenceSession, js_name = create, catch)]
    fn create(data: &Uint8Array, options: &JsValue) -> std::result::Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = run, catch)]
    fn run_js(this: &InferenceSession, feeds: &JsValue) -> std::result::Result<Promise, JsValue>;
    #[wasm_bindgen(method, js_name = release, catch)]
    fn release_js(this: &InferenceSession) -> std::result::Result<Promise, JsValue>;
    #[wasm_bindgen(js_name = Tensor)]
    type Tensor;
    #[wasm_bindgen(constructor, catch)]
    fn new(dtype: &str, data: &JsValue, dims: &Array) -> std::result::Result<Tensor, JsValue>;
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
pub struct Input<'a> {
    pub name: &'a str,
    pub values: &'a [f32],
    pub shape: &'a [usize],
}
impl Runtime {
    pub fn float_tensor(values: &[f32], shape: &[usize]) -> Result<JsValue> {
        ensure!(
            shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b)) == Some(values.len())
                && values.iter().all(|v| v.is_finite()),
            "Invalid float input tensor"
        );
        let dims = Array::new();
        for &dim in shape {
            dims.push(&(dim as f64).into());
        }
        Ok(Tensor::new("float32", &Float32Array::from(values), &dims)
            .map_err(error)?
            .into())
    }
    pub fn int_tensor(values: &[i64], shape: &[usize]) -> Result<JsValue> {
        ensure!(
            shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b)) == Some(values.len()),
            "Invalid integer input tensor"
        );
        let dims = Array::new();
        for &dim in shape {
            dims.push(&(dim as f64).into());
        }
        Ok(
            Tensor::new("int64", &js_sys::BigInt64Array::from(values), &dims)
                .map_err(error)?
                .into(),
        )
    }
    pub async fn run_tensors(&self, inputs: &[(&str, &JsValue)]) -> Result<JsValue> {
        let feeds = Object::new();
        for &(name, tensor) in inputs {
            set(&feeds, name, tensor)?;
        }
        JsFuture::from(self.session.run_js(&feeds).map_err(error)?)
            .await
            .map_err(error)
    }
    /// Keep large decoder caches in the runtime instead of copying them through
    /// Rust for every autoregressive token. The graph contract remains checked.
    pub fn tensor(outputs: &JsValue, name: &str, shape: &[usize]) -> Result<JsValue> {
        let tensor = Reflect::get(outputs, &name.into()).map_err(error)?;
        ensure!(!tensor.is_undefined(), "Model output missing {name}");
        let actual = Array::from(&Reflect::get(&tensor, &"dims".into()).map_err(error)?);
        ensure!(
            actual.length() as usize == shape.len()
                && actual
                    .iter()
                    .zip(shape)
                    .all(|(a, &b)| a.as_f64() == Some(b as f64)),
            "Unexpected {name} shape"
        );
        let data = Reflect::get(&tensor, &"data".into()).map_err(error)?;
        ensure!(
            data.is_instance_of::<Float32Array>()
                && shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b))
                    == Some(Float32Array::new(&data).length() as usize),
            "Invalid {name} tensor data"
        );
        Ok(tensor)
    }
    pub async fn load(
        id: &str,
        progress: &mut impl FnMut(String),
        disable_optimization: bool,
    ) -> Result<Self> {
        let weights = crate::model_assets::load(id, progress).await?;
        progress(format!("Loading {id} transcription model locally"));
        Self::from_bytes(&weights, disable_optimization).await
    }
    /// Local optional checkpoints use the same pinned runtime without a network request.
    pub async fn from_bytes(weights: &[u8], disable_optimization: bool) -> Result<Self> {
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
        let session = JsFuture::from(
            InferenceSession::create(&Uint8Array::from(weights), &options).map_err(error)?,
        )
        .await
        .map_err(error)?
        .unchecked_into();
        Ok(Self { session })
    }
    pub async fn release(self) -> Result<()> {
        JsFuture::from(self.session.release_js().map_err(error)?)
            .await
            .map_err(error)?;
        Ok(())
    }
    pub async fn run(&self, name: &str, values: &[f32], shape: &[usize]) -> Result<JsValue> {
        self.run_inputs(&[Input {
            name,
            values,
            shape,
        }])
        .await
    }
    pub async fn run_inputs(&self, inputs: &[Input<'_>]) -> Result<JsValue> {
        let feeds = Object::new();
        for &Input {
            name,
            values,
            shape,
        } in inputs
        {
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
            let tensor =
                Tensor::new("float32", &Float32Array::from(values), &dims).map_err(error)?;
            set(&feeds, name, &tensor)?;
        }
        JsFuture::from(self.session.run_js(&feeds).map_err(error)?)
            .await
            .map_err(error)
    }
    pub fn output_f64(outputs: &JsValue, name: &str) -> Result<(Vec<usize>, Vec<f64>)> {
        let output = Reflect::get(outputs, &name.into()).map_err(error)?;
        ensure!(!output.is_undefined(), "Model output missing {name}");
        let dims = Array::from(&Reflect::get(&output, &"dims".into()).map_err(error)?)
            .iter()
            .map(|v| {
                v.as_f64()
                    .map(|v| v as usize)
                    .context("Invalid output dimension")
            })
            .collect::<Result<Vec<_>>>()?;
        let data = Reflect::get(&output, &"data".into()).map_err(error)?;
        ensure!(
            data.is_instance_of::<Float64Array>(),
            "Unexpected output type for {name}"
        );
        let values = Float64Array::new(&data).to_vec();
        ensure!(
            dims.iter().try_fold(1usize, |a, b| a.checked_mul(*b)) == Some(values.len()),
            "Invalid output shape"
        );
        Ok((dims, values))
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
