# Security

YACE is pre-release software. Security fixes target the current `main` branch;
there is no supported older-release series yet.

## Report a vulnerability

Use **Security → Report a vulnerability** on this repository when private
reporting is enabled. If the option is unavailable, open an issue asking for a
private contact method. Do not include exploit details, credentials, recordings
or private project data in a public issue.

Include the affected revision, operating system, reproduction steps and expected
impact in the private report. Use a synthetic file when possible.

## Dependency checks

Install the RustSec scanner and audit the committed lockfiles:

```sh
cargo install cargo-audit --locked
cargo audit --file Cargo.lock
```

Each tool under `tools/` has its own `Cargo.lock`; audit those too. See
[docs/VERIFICATION.md](docs/VERIFICATION.md) for the other checks.
An advisory-free scan alone does not establish that the application is secure.
