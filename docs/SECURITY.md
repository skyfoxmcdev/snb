# Security

## Non-negotiable rules

- Never publish the Minecraft origin server's public IP or origin credentials. Only the public Playit endpoint is allowed.
- Never commit the update private key, GitHub tokens, Microsoft credentials or server secrets to this repository or the Pages repository.
- Do not disable HTTPS certificate validation or add HTTP remote URLs in release builds.
- Do not add telemetry, automatic diagnostic uploads, ad identifiers or behavioural analytics.
- Do not execute commands or code supplied by remote JSON.

## Implemented protections

- Remote JSON schemas are versioned and validated; URLs must be HTTPS; endpoint literals/internal hostnames are rejected.
- Remote JSON text is scanned for IP literals before acceptance.
- Downloads use `.part` resume, size checks, SHA-1/SHA-256 checks and atomic replacement.
- The update installer requires SHA-256 and, when a public key is configured, a valid Ed25519 signature over the exact bytes.
- Update helper backs up the previous app directory and restores it when installation or startup health checks fail.
- Zip/tar extraction rejects traversal; launcher content paths are validated; the vanilla `.minecraft` directory is rejected.
- Logs redact credential patterns and IP addresses. Diagnostics additionally redact the user's home path and secrets, and are exported locally only.
- No administrator privileges required; no auto-start; no background daemon.

## Residual risks

- Offline-mode identity is not authentication; the server must legitimately support it.
- Unmanaged mods can be malicious. Users add local JARs at their own risk; the launcher does not provide a mod catalogue.
- A compromised owner machine/Pages account could publish a malicious update until signing keys or repository access are rotated. Protect those accounts and the offline private key.
- Unsigned Windows builds can trigger SmartScreen. Use Authenticode signing for public distribution.

## Reporting

Report vulnerabilities privately to the Sky & Beyond owner. Do not publish exploit details before a fix and a rotated release are available.
