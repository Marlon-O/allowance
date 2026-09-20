# Security policy

## Reporting a vulnerability

Do not publish credential exposure, command injection, or local privilege issues in a public issue before a fix is available. Send a private report to the project maintainer through the repository's private security-reporting feature. Include affected versions, reproduction steps, impact, and any proposed mitigation.

No dedicated security-response email is configured in this source package. A public repository owner should enable private vulnerability reporting and add a monitored contact before announcing a stable release.

## Trust boundaries

Allowance launches installed provider CLIs and parses local protocol output. Claude integration writes a quoted helper command to the user-level Claude settings after explicit approval. It backs up the prior value and restores it only if the current value is still owned by Allowance.

Installers should be built from a reviewed tag on native platform runners. Release builds intended for the public should be signed and, on macOS, notarized. Verify published SHA-256 checksums before installation.

Allowance supports current-user installation and one active local account per provider. Do not run it with elevated privileges.
