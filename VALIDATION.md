# Validation — Allowance

Tested on September 20, 2026, on an Apple Silicon Mac.

## Passed

- Production React/TypeScript build.
- Frontend logic tests: missing values versus zero, percentage conversion, stale/future timestamps, and reset boundaries.
- Rust unit tests: dynamic provider windows, null fields, alert timing/deduplication, obsolete-data cleanup, malformed settings preservation, Claude setup/restoration, external settings edits, path quoting, repeated observations, concurrent sessions, account switching, migration from a stale helper to a content-versioned current helper, and credential-free parsing of Claude Desktop's latest local usage sample and cached reset timestamps.
- One native-executable integration test: an existing Claude status-line command receives its original input and working directory. Test data stays in disposable directories.
- Explicit live Codex test: the installed CLI app server returns both quota windows using its current login. No conversation or billable model request is made.
- Rust Clippy with warnings denied.
- Browser visual/interaction checks: enlarged 410×560 tray layout, refresh notice dismissal, dark and light themes, remaining/used toggle, and preferences.
- A transient freshness-display issue found during native testing was fixed by updating the frontend clock with each fetched state.
- Native release `.app` built successfully. The final DMG uses a drag-to-Applications layout and a locally ad hoc signed app; its image checksum is verified by `hdiutil verify`.
- Windows x64 release and NSIS installer cross-built successfully with Tauri's documented `cargo-xwin` flow. The payload is a 64-bit AMD64 Windows GUI executable and the package is a valid NSIS self-extracting executable.
- npm dependency audit reported no known vulnerabilities after upgrading the test runner.
- Legal, privacy, security, setup, release, and validation documentation is included. Claude setup requires explicit approval before changing the user-level status line.

## Not yet verified

- Live Claude quota readings: Claude Desktop's local usage history and cached response were inspected without reading credentials. The cache contained a null reset for a fully available 5-hour window and an exact weekly reset timestamp; both states are covered by automated tests.
- Windows runtime behavior. The executable and installer were compile-checked, linted, and structurally inspected on this Mac, but cannot be launched here for an end-to-end native Windows test.
- Intel Mac build/runtime.
- The new tray-relative popover placement still needs native verification on macOS, Windows, multi-monitor layouts, and display scaling.
- Real OS notifications, launch-at-login, and sleep/wake were not exercised end to end. Their implementation is present; alert semantics are covered by unit tests.
- Full live offline/expired-auth behavior. The adapters implement timeout, backoff, signed-out, and stale states, but deliberately disconnecting the user's network or changing account credentials was not part of testing.

## Packaging notes

The included `scripts/package-macos.sh` builds the app, applies a local ad hoc signature, creates a standard compressed drag-to-Applications DMG with `hdiutil`, and verifies the finished image.

The Windows package was cross-built on macOS as an unsigned x64 NSIS installer. Cross-platform NSIS packaging is supported with caveats by Tauri; a matching Windows host is still preferred for final release validation and Authenticode signing.

The Mac app has a local ad hoc signature, not an Apple Developer ID signature or notarization. It must be clearly labeled as unsigned. A generally trusted public installer still requires Developer ID signing and notarization. Automatic updates, browser-only logins, API billing, and WSL-only installations remain outside the current scope.

`npm run dev` is an explicitly labeled sample-data preview. Real desktop usage never falls back to preview data. Account labels stay on this computer and are not included in the source archive or installer.
