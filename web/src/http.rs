//! Bounded public catalogue requests. Chart/audio bytes and credentials are absent.
use anyhow::{Context, Result, ensure};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
pub struct Response {
    pub status: u16,
    pub mime: String,
    pub bytes: Vec<u8>,
}
fn allowed(url: &str) -> Result<()> {
    let parsed = web_sys::Url::new(url).map_err(|_| anyhow::anyhow!("Invalid catalogue URL"))?;
    let host = parsed.hostname();
    ensure!(
        parsed.protocol() == "https:"
            && parsed.username().is_empty()
            && parsed.password().is_empty()
            && parsed.port().is_empty()
            && (matches!(
                host.as_str(),
                "musicbrainz.org"
                    | "itunes.apple.com"
                    | "lrclib.net"
                    | "coverartarchive.org"
                    | "archive.org"
            ) || host.ends_with(".archive.org")),
        "Unsupported catalogue or artwork host"
    );
    Ok(())
}
struct Abort(web_sys::AbortController);
impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub async fn get(url: &str, limit: usize) -> Result<Response> {
    allowed(url)?;
    let abort = Abort(
        web_sys::AbortController::new()
            .map_err(|_| anyhow::anyhow!("Cannot prepare catalogue request"))?,
    );
    let options = web_sys::RequestInit::new();
    options.set_method("GET");
    options.set_credentials(web_sys::RequestCredentials::Omit);
    options.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
    options.set_signal(Some(&abort.0.signal()));
    let request = web_sys::Request::new_with_str_and_init(url, &options)
        .map_err(|_| anyhow::anyhow!("Cannot prepare catalogue request"))?;
    let fetch = async {
        let response:web_sys::Response=JsFuture::from(web_sys::window().context("No browser window")?.fetch_with_request(&request)).await.map_err(|_|anyhow::anyhow!("Catalogue request unavailable. Check connectivity or browser access; imported fields are retained."))?.dyn_into().map_err(|_|anyhow::anyhow!("Invalid catalogue response"))?;
        allowed(&response.url())?;
        let status = response.status();
        let mime = response
            .headers()
            .get("Content-Type")
            .ok()
            .flatten()
            .unwrap_or_default();
        let reader: web_sys::ReadableStreamDefaultReader = response
            .body()
            .context("Catalogue returned no data")?
            .get_reader()
            .unchecked_into();
        let mut bytes = Vec::new();
        loop {
            let next = JsFuture::from(reader.read())
                .await
                .map_err(|_| anyhow::anyhow!("Catalogue response interrupted"))?;
            if js_sys::Reflect::get(&next, &"done".into())
                .ok()
                .and_then(|v| v.as_bool())
                == Some(true)
            {
                break;
            }
            let value = js_sys::Reflect::get(&next, &"value".into())
                .map_err(|_| anyhow::anyhow!("Invalid catalogue response"))?;
            let chunk = js_sys::Uint8Array::new(&value);
            ensure!(
                bytes.len() + chunk.length() as usize <= limit,
                "Catalogue response exceeds the size limit"
            );
            bytes.extend(chunk.to_vec());
        }
        reader.release_lock();
        Ok::<_, anyhow::Error>(Response {
            status,
            mime,
            bytes,
        })
    };
    let timeout = gloo_timers::future::TimeoutFuture::new(20_000);
    futures_util::pin_mut!(fetch, timeout);
    match futures_util::future::select(fetch, timeout).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => {
            anyhow::bail!("Catalogue request timed out; imported information is retained")
        }
    }
}
pub fn query(base: &str, fields: &[(&str, String)]) -> String {
    format!(
        "{base}?{}",
        fields
            .iter()
            .map(|(key, value)| format!(
                "{}={}",
                js_sys::encode_uri_component(key),
                js_sys::encode_uri_component(value)
            ))
            .collect::<Vec<_>>()
            .join("&")
    )
}
pub async fn json(url: &str) -> Result<serde_json::Value> {
    let response = get(url, 2_000_000).await?;
    ensure!(
        (200..300).contains(&response.status),
        "Catalogue HTTP {}",
        response.status
    );
    Ok(serde_json::from_slice(&response.bytes)?)
}
