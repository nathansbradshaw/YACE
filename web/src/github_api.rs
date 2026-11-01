//! Optional direct GitHub authorization. Tokens exist only in this tab's memory.
use crate::github_reviews::{Api, Reply};
use anyhow::{Context, Result, ensure};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
pub struct Client {
    token: String,
}
impl Client {
    pub fn new(token: String) -> Result<Self> {
        ensure!(
            token.len() <= 512 && token.bytes().all(|c| c.is_ascii_graphic()),
            "Invalid GitHub token format"
        );
        Ok(Self { token })
    }
}
struct Abort(web_sys::AbortController);
impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl Api for Client {
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Reply> {
        ensure!(
            matches!(method, "GET" | "POST" | "PATCH")
                && path.starts_with('/')
                && !path.contains("..")
                && path
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"/-_.".contains(&c)),
            "Invalid GitHub API request"
        );
        let abort = Abort(
            web_sys::AbortController::new()
                .map_err(|_| anyhow::anyhow!("Cannot initialize GitHub request"))?,
        );
        let options = web_sys::RequestInit::new();
        options.set_method(method);
        options.set_credentials(web_sys::RequestCredentials::Omit);
        options.set_redirect(web_sys::RequestRedirect::Error);
        options.set_cache(web_sys::RequestCache::NoStore);
        options.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
        options.set_signal(Some(&abort.0.signal()));
        if let Some(body) = body {
            options.set_body(&serde_json::to_string(&body)?.into());
        }
        let request = web_sys::Request::new_with_str_and_init(
            &format!("https://api.github.com{path}"),
            &options,
        )
        .map_err(|_| anyhow::anyhow!("Cannot prepare GitHub request"))?;
        let headers = request.headers();
        for (name, value) in [
            ("Accept", "application/vnd.github+json"),
            ("Content-Type", "application/json"),
            ("X-GitHub-Api-Version", "2026-03-10"),
        ] {
            headers
                .set(name, value)
                .map_err(|_| anyhow::anyhow!("Cannot prepare GitHub headers"))?;
        }
        if !self.token.is_empty() {
            headers
                .set("Authorization", &format!("Bearer {}", self.token))
                .map_err(|_| anyhow::anyhow!("Cannot authorize GitHub request"))?;
        }
        let fetch = async {
            let response: web_sys::Response=JsFuture::from(web_sys::window().context("No browser window")?.fetch_with_request(&request)).await.map_err(|_|anyhow::anyhow!("Cannot reach GitHub. Check connectivity and allowed browser access; local work is retained."))?.dyn_into().map_err(|_|anyhow::anyhow!("Invalid GitHub response"))?;
            let status = response.status();
            let stream = response
                .body()
                .context("GitHub returned no response body")?;
            let reader: web_sys::ReadableStreamDefaultReader = stream.get_reader().unchecked_into();
            let mut bytes = Vec::new();
            loop {
                let next = JsFuture::from(reader.read())
                    .await
                    .map_err(|_| anyhow::anyhow!("GitHub response interrupted"))?;
                if js_sys::Reflect::get(&next, &"done".into())
                    .ok()
                    .and_then(|v| v.as_bool())
                    == Some(true)
                {
                    break;
                }
                let value = js_sys::Reflect::get(&next, &"value".into())
                    .map_err(|_| anyhow::anyhow!("Invalid GitHub response chunk"))?;
                let chunk = js_sys::Uint8Array::new(&value);
                ensure!(
                    bytes.len() + chunk.length() as usize <= 24 * 1024 * 1024,
                    "GitHub response exceeds the review size limit"
                );
                bytes.extend(chunk.to_vec());
            }
            reader.release_lock();
            Ok::<_, anyhow::Error>(Reply {
                status,
                body: serde_json::from_slice(&bytes).context("GitHub returned invalid JSON")?,
            })
        };
        let timeout = gloo_timers::future::TimeoutFuture::new(30_000);
        futures_util::pin_mut!(fetch, timeout);
        match futures_util::future::select(fetch, timeout).await {
            futures_util::future::Either::Left((result, _)) => result,
            futures_util::future::Either::Right(_) => anyhow::bail!(
                "GitHub request timed out. Pull again before publishing; local work is retained."
            ),
        }
    }
}
