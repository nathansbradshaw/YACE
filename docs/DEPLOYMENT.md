# Public static hosting

The production hostname is `yace.n8bradshaw.com`. Sites hosts the compiled static
application with public access and no sign-in. The GitHub source repository is
public at <https://github.com/nathansbradshaw/YACE>.

The stable Sites identity is tracked in `.openai/hosting.json`. Reuse that project
ID; do not create another Site for an update. Its generated origin is
<https://yace-n8bradshaw.aurorasheep.chatgpt.site>.

## Build and update

1. Fetch/rebase the Rust source, run the checks in [BROWSER.md](BROWSER.md), and
   build with `cargo run --locked --manifest-path web/tools/Cargo.toml -- build`.
2. The application build emits `web/dist`. Do not include recordings, projects,
   models, local credentials or runtime data in a deployment.
3. Open the existing Site's deployment repository in a separate, empty checkout
   using its native Sites source credential and source workflow. This repository
   contains the compiled assets, not the native application's source history.
   Copy the new `web/dist` contents into that checkout's `dist` directory and
   preserve its `.openai/hosting.json` and project ID.
4. Use the Sites source workflow to commit/push that exact state and package it.
   Save the returned commit/archive as a Site version, then deploy that version
   with the public deployment operation. Keep credentials in memory and pass
   them only through the helper's hidden stdin.
5. Verify the deployment result and the app on the custom domain. Opening files,
   editing, audio, recovery and exports run locally in the browser; no charting
   API or model service is configured.

The first published artifact was built from Rust source commit
`8b31a8dac1a8de26335027034affce477be518ef`. Its deployment-repository commit is
`fa87e2008a7f6aa2aae470c31b7b8162b21ef6c5` (Sites version 1).

## DNS

Cloudflare hosts the `n8bradshaw.com` zone. The custom-domain configuration is
managed through Sites. Keep its CNAME and verification TXT records in DNS:

| Type | Name | Value |
| --- | --- | --- |
| CNAME | `yace` | `custom-domains.chatgpt.site` |
| TXT | `_openai-site-verification.yace` | `openai-site-verification=x4Qz9eSRzkB1KW1wh4qcEoMm1d8m5dOtbXChM5Xz2Yw` |
| TXT | `_cf-custom-hostname.yace` | `17977612-e241-4f62-ae5e-0746f61e88ba` |

Use DNS-only routing for the CNAME while Sites provisions and validates the
custom hostname and HTTPS certificate. These TXT values are public domain
ownership challenges, not application secrets. If the custom domain is ever
recreated, use the fresh records returned by Sites rather than reusing this list.

Browser recovery data belongs to an origin. Projects saved on loopback or the
generated Sites origin do not automatically appear on the custom hostname;
transfer them with Save project ZIP / Open saved project.
