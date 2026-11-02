# ONNX Runtime Web 1.22.0

Unmodified distribution files from the official `onnxruntime-web@1.22.0` npm
package. This is the same ONNX Runtime release used by the native Rust client.
Only the WASM execution provider is included. YACE configures one thread and
runs it inside its existing dedicated worker; no application service or
cross-origin isolation headers are required.

Package: https://registry.npmjs.org/onnxruntime-web/-/onnxruntime-web-1.22.0.tgz

Package integrity:
`sha512-Ud/+EBo6mhuaQWt/OjaOk0iNWjXqJoeeMFr6xQEERZdIZH2OWpGzuujz7lfuOBjUa6TEE/sc4nb7Da5dNL34fg==`

Upstream source: https://github.com/microsoft/onnxruntime/tree/v1.22.0

`LICENSE` and `ThirdPartyNotices.txt` are from that exact upstream tag. Model
weights are downloaded separately using YACE's pinned catalogue and checked
before use. YACE's orchestration, audio preparation, decoding, evidence and
chart building remain Rust. The JavaScript files here are the platform runtime
distribution, not application logic.
