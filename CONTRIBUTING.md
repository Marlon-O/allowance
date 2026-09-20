# Contributing

Use a focused branch and keep provider integrations local and read-only. Do not add credential, prompt, transcript, or conversation storage. New network access, telemetry, or account scopes require an explicit design discussion and corresponding privacy documentation.

Before submitting a change, run:

```sh
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

Test changes that touch installers or native behavior on the target operating system. Record untested platforms in `VALIDATION.md`. Keep `private: true` in `package.json`; this project is distributed as a desktop application, not an npm package.

Provider and settings fixtures must use invented identities and disposable directories. Live provider tests must remain opt-in, read-only, and must never send an AI prompt.
