# Public static hosting

The production hostname is `yace.n8bradshaw.com`. GitHub Pages hosts the compiled
static application with public access and no sign-in, from the root of the
`codex/yace-public-assets` branch in the existing public repository
<https://github.com/nathansbradshaw/YACE>. The asset branch contains deployable
files; `main` contains the Rust sources. Pages uses branch deployment with
`.nojekyll` and `CNAME` at the asset root.

The previous Sites deployment is retained as a rollback, with its stable identity
in `.openai/hosting.json` and origin
<https://yace-n8bradshaw.aurorasheep.chatgpt.site>. Do not create another Site or
delete the retained deployment during routine Pages updates.
The custom hostname was detached from Sites during the migration. A rollback to
that host must register the hostname again and use its returned validation
records; changing DNS alone is insufficient.

## Build and update

Deploy each completed feature after its relevant verification passes. Do not
hold completed browser work for an entire parity backlog. The deployed
`build-info.json` records the exact source revision. After publication, check
the custom-domain application and the feature's actual browser workflow.

1. Fetch/rebase the Rust source, run the checks in [BROWSER.md](BROWSER.md), and
   build with `cargo run --locked --manifest-path web/tools/Cargo.toml -- build`.
2. The application build emits `web/dist`. Do not include recordings, projects,
   models, local credentials or runtime data in a deployment.
3. Reuse a clean worktree of `codex/yace-public-assets` in the existing repository.
   Stage the new bundle with the Rust static tool:
   `yace-static-web stage /path/to/pages-worktree SOURCE_SHA`.
   Use the complete verified Git source revision. This creates an immutable
   `releases/SOURCE_SHA/` and updates the root entry point and build info.
   Preserve earlier release directories and the original root assets; do not
   replace the hosting directory with a flat `rsync --delete` copy.
   If the retained root `runtime/` is byte-identical to the new bundle, add
   `--reuse-root-runtime` to avoid another copy of those inference binaries.
   Staging verifies every runtime file before changing the new generated
   module's relative import; earlier modules and runtime bytes stay untouched.
   A changed runtime is rejected in this mode and needs a complete release.
4. Preserve `.nojekyll` and `CNAME` containing `yace.n8bradshaw.com`. Commit/push
   the staged files to `codex/yace-public-assets` without rewriting its history.
   Wait for the Pages deployment to succeed. Do not merge compiled assets into
   `main`, and do not publish recordings, model weights or local project data.
5. Verify the deployment result and the app on the custom domain. Opening files,
   editing, audio, recovery and exports run locally in the browser; no charting
   API or model service is configured.

### Controlled Pages publication

`Deploy verified static YACE` publishes the existing prebuilt assets branch;
it does not compile Rust or infer feature acceptance. Keep its workflow file
identical on `main` and `codex/yace-public-assets`. After pushing a verified
asset commit, dispatch it with:

```sh
gh workflow run deploy-static-pages.yml --ref codex/yace-public-assets
```

The workflow runs only on that publication branch and retains the existing
`github-pages` environment restriction. Its deployment action has a bounded
20-minute client timeout, within a 25-minute job. This permits an observable
longer file-sync attempt after the built-in publisher repeatedly cancelled at
its default 10-minute timeout. It does not establish the cause of a stalled
sync, override GitHub's service limits, or qualify the release as live. Do not
loop retries or delete immutable releases to make a failed attempt appear done.
Record the actual deployment result and exercise the served feature afterward.

Enable GitHub Actions as the Pages build source only after the reviewed workflow
is present on the publication branch. Preserve the custom domain, HTTPS and
environment policies. Manual dispatch also permits the repository's skip-CI
commit convention without silently skipping publication.

On 2026-11-09 the built-in publisher abandoned one attempt before building;
one documented rebuild then uploaded successfully but cancelled while syncing
files after ten minutes. The reviewed manual workflow published asset commit
`b8e87689` successfully in run `42677080916`. Its job completed in 119 seconds,
so that success does not establish that the longer timeout caused recovery.
The domain and HTTPS setting stayed configured, and the original two allowed
deployment branches were restored after GitHub automatically added `main`
when the build source changed. No immutable release was removed.

Actual hosted acceptance loaded source `8fa1be97`, its release-local module
and the exact 21,808,005-byte WASM with SHA-256
`5c33c71e8af4214adb87964facbdf2476669a93d02591d4a746545351ee1d400`.
Drum authoring, Undo/Redo, late-part inheritance, editable backup and repeated
MIDI export/reopen passed. This is a verified publication increment; future
deployments still require their own result and live workflow checks.

Workers resolve their URL from the generated module's `import.meta.url` through
a Rust wasm-bindgen binding. A tab loaded from one release therefore keeps using
that release's worker, WASM and inference runtime after a later publish. No extra
handwritten JavaScript or inference service is needed. The original root assets
also remain available to tabs opened before release directories were introduced.
Do not prune retained assets as part of an ordinary deployment.

The first published artifact was built from Rust source commit
`8b31a8dac1a8de26335027034affce477be518ef`. Its deployment-repository commit is
`fa87e2008a7f6aa2aae470c31b7b8162b21ef6c5` (Sites version 1).

## DNS

Cloudflare hosts the `n8bradshaw.com` zone. GitHub Pages manages the custom domain
and its HTTPS certificate. Keep this DNS-only CNAME:

| Type | Name | Value |
| --- | --- | --- |
| CNAME | `yace` | `nathansbradshaw.github.io` |

Previous Sites verification records remain in DNS for reference. A new hostname
registration may issue different values:

| Type | Name | Value |
| --- | --- | --- |
| TXT | `_openai-site-verification.yace` | `openai-site-verification=x4Qz9eSRzkB1KW1wh4qcEoMm1d8m5dOtbXChM5Xz2Yw` |
| TXT | `_cf-custom-hostname.yace` | `17977612-e241-4f62-ae5e-0746f61e88ba` |

Configure the custom domain in Pages before changing DNS. Wait for its DNS check
and certificate provisioning; validate ordinary HTTPS before enabling Enforce
HTTPS. If provisioning stalls after DNS is correct, follow GitHub's documented
remove/re-add custom-domain procedure. Never bypass TLS warnings or weaken the
zone's TLS settings. The TXT values above are public ownership challenges.

Changing the host behind the same HTTPS hostname preserves that origin's browser
saves. GitHub's default `github.io` address has a separate browser storage origin.

Browser recovery data belongs to an origin. Projects saved on loopback or the
generated Sites origin do not automatically appear on the custom hostname;
transfer them with Download project backup / Open saved project.
