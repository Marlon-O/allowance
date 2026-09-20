# Release guide

1. Update versions in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`.
2. Update `CHANGELOG.md` and `VALIDATION.md`, then run all checks in `CONTRIBUTING.md`.
3. Build each target on its native operating system. Do not present an untested platform artifact as verified.
4. Sign the macOS app with an Apple Developer ID Application certificate and notarize/staple the DMG. Sign the Windows executable and installer with an Authenticode certificate and timestamp service.
5. Generate SHA-256 checksums from the final signed artifacts. Signing after checksumming invalidates the checksum.
6. Install each final artifact on a clean standard-user machine. Verify launch, tray lifecycle, provider connection, disconnect restoration, notifications, and uninstall behavior.
7. Publish from an immutable version tag with the privacy policy, setup guide, checksums, validation report, and source archive.

The included GitHub Actions workflow creates unsigned test artifacts. Signing credentials are intentionally absent. Add them through the chosen release system's protected secret store; never commit certificates, passwords, tokens, or notarization credentials.

The fallback `scripts/package-macos.sh` creates an ad hoc signed DMG for local testing when Tauri's DMG bundler cannot mount a volume. It is not a substitute for Developer ID signing and notarization.
