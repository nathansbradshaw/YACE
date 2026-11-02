//! Public pinned checkpoints cached by the browser; no recording leaves it.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

fn error(value: impl Into<wasm_bindgen::JsValue>) -> anyhow::Error {
    let value = value.into();
    anyhow::anyhow!("Model download/cache failed: {value:?}")
}
fn valid(bytes: &[u8], asset: &crate::models::ModelAsset) -> bool {
    format!("{:x}", Sha256::digest(bytes)) == asset.sha256
}
async fn cache() -> Result<web_sys::Cache> {
    let storage = js_sys::Reflect::get(&js_sys::global(), &"caches".into())
        .map_err(error)?
        .dyn_into::<web_sys::CacheStorage>()
        .map_err(error)?;
    JsFuture::from(storage.open("YACE-verified-models-v1"))
        .await
        .map_err(error)?
        .dyn_into()
        .map_err(error)
}

/// A selected local file is accepted only as an exact catalogue checkpoint.
pub async fn import(mut bytes: Vec<u8>) -> Result<String> {
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let asset = crate::models::assets()
        .into_iter()
        .find(|asset| asset.sha256 == digest)
        .context("This file is not one of YACE's pinned model checkpoints")?;
    let cache = cache()
        .await
        .context("Browser model storage is unavailable")?;
    let response = web_sys::Response::new_with_opt_u8_array(Some(&mut bytes)).map_err(error)?;
    JsFuture::from(cache.put_with_str(&asset.url, &response))
        .await
        .map_err(error)?;
    Ok(asset.id)
}

pub async fn load(id: &str, progress: &mut impl FnMut(String)) -> Result<Vec<u8>> {
    let asset = crate::models::assets()
        .into_iter()
        .find(|asset| asset.id == id)
        .context("Unknown model checkpoint")?;
    let cache = cache().await.ok();
    if let Some(cache) = &cache {
        let response = JsFuture::from(cache.match_with_str(&asset.url))
            .await
            .map_err(error)?;
        if let Ok(response) = response.dyn_into::<web_sys::Response>() {
            progress(format!("Checking cached {} model", asset.id));
            let data = JsFuture::from(response.array_buffer().map_err(error)?)
                .await
                .map_err(error)?;
            let bytes = js_sys::Uint8Array::new(&data).to_vec();
            if valid(&bytes, &asset) {
                return Ok(bytes);
            }
            JsFuture::from(cache.delete_with_str(&asset.url))
                .await
                .map_err(error)?;
        }
    }
    progress(format!("Downloading {} model", asset.id));
    let init = web_sys::RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
    let request = web_sys::Request::new_with_str_and_init(&asset.url, &init).map_err(error)?;
    let response = JsFuture::from(
        js_sys::global()
            .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
            .fetch_with_request(&request),
    )
    .await
    .map_err(error)?
    .dyn_into::<web_sys::Response>()
    .map_err(error)?;
    ensure!(
        response.ok(),
        "Model download returned HTTP {}",
        response.status()
    );
    let maximum = 1024 * 1024 * 1024;
    let expected = response
        .headers()
        .get("Content-Length")
        .map_err(error)?
        .and_then(|length| length.parse::<usize>().ok());
    ensure!(
        expected.is_none_or(|length| length <= maximum),
        "Model exceeds 1 GiB"
    );
    let reader = response
        .body()
        .context("Model download has no body")?
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(error)?;
    let mut bytes = Vec::with_capacity(expected.unwrap_or(256 * 1024));
    let mut reported = 0;
    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(error)?;
        if js_sys::Reflect::get(&chunk, &"done".into())
            .map_err(error)?
            .as_bool()
            == Some(true)
        {
            break;
        }
        let data =
            js_sys::Uint8Array::new(&js_sys::Reflect::get(&chunk, &"value".into()).map_err(error)?);
        ensure!(
            bytes.len() + data.length() as usize <= maximum,
            "Model exceeds 1 GiB"
        );
        bytes.extend(data.to_vec());
        if bytes.len() / (1024 * 1024) > reported {
            reported = bytes.len() / (1024 * 1024);
            progress(format!("Downloading {} · {reported} MiB", asset.id));
        }
    }
    ensure!(
        valid(&bytes, &asset),
        "Downloaded {} model checksum does not match the desktop checkpoint",
        asset.id
    );
    if let Some(cache) = cache {
        let response = web_sys::Response::new_with_opt_u8_array(Some(&mut bytes)).map_err(error)?;
        // Cache failure must not invalidate a verified in-memory checkpoint.
        if JsFuture::from(cache.put_with_str(&asset.url, &response))
            .await
            .is_err()
        {
            progress("Model verified; browser could not retain its download cache".into());
        }
    }
    Ok(bytes)
}
