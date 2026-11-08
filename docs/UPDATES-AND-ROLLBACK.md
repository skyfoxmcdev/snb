# Launcher updates and emergency rollback

## Policy

Launcher updates are **mandatory**. In a release build the launcher fetches `launcher/update-manifest.json` at startup. It blocks normal use if:

- the remote version is newer than the installed version, or
- the installed version is below `minimumVersion`, or
- the manifest cannot be fetched or validated.

There is no hidden bypass. The update screen offers retry/log access. Debug builds skip the online check by default to allow local UI development; setting `SNB_FORCE_UPDATE_CHECK=1` tests the live behavior.

## Manifest format

```json
{
  "schemaVersion": 1,
  "version": "0.2.0",
  "minimumVersion": "0.1.0",
  "mandatory": true,
  "releasedAt": "2026-11-07T12:00:00Z",
  "notes": { "en": "Release notes", "fr": "Notes de version" },
  "platforms": {
    "windows-x86_64": {
      "url": "https://github.com/OWNER/REPO/releases/download/v0.2.0/Sky-and-Beyond_0.2.0_x64-setup.exe",
      "sha256": "64 lowercase hex characters",
      "size": 12345678,
      "signature": "optional base64 Ed25519 signature",
      "fileName": "Sky-and-Beyond_0.2.0_x64-setup.exe"
    }
  }
}
```

Rules:

- `schemaVersion` is exactly `1`.
- `version` and `minimumVersion` are valid semver.
- Every platform artifact has a direct HTTPS URL, a 64-character SHA-256, a positive size and a safe filename (`.exe` expected on Windows).
- `mandatory: false` is accepted for data-only releases, but an installed version below `minimumVersion` is always blocked. For launcher binaries, keep `mandatory: true`.
- Use [scripts/prepare-release.mjs](../scripts/prepare-release.mjs) to fill hash and size safely.

## Optional Ed25519 signing

The checksum is mandatory. Signing is strongly recommended:

1. Generate a private key once, offline:
   ```bash
   openssl genpkey -algorithm ED25519 -out update-private.pem
   ```
   Keep this file offline and backed up. Never commit it.
2. Put the raw 32-byte public key, base64-encoded, in `src-tauri/resources/launcher-config.json` at `update.publicKey` (validated as exactly 32 bytes).
3. Sign the exact installer bytes:
   ```bash
   openssl pkeyutl -sign -inkey update-private.pem -rawin -in setup.exe -out signature.bin
   base64 -w0 signature.bin
   ```
   On Windows use `certutil -encode` or Git Bash `base64`.
4. Paste the base64 result into the manifest `signature` field.
5. If a public key is configured, a missing or invalid signature is a hard failure.

You can leave `publicKey` empty during early development; checksum verification still applies. Treat the key change as a security event.

## Transactional flow

1. Manifest fetched and validated; artifact selected for the current platform.
2. Installer downloaded to `%APPDATA%\com.skyandbeyond.launcher\updates\`.
3. Size and SHA-256 verified; optional Ed25519 verified.
4. External `snb-updater` sidecar starts. It copies itself out of the installation directory on Windows so it cannot be overwritten while running.
5. Existing launcher directory is copied to `%APPDATA%\com.skyandbeyond.launcher\update-backups\app-<version>-<timestamp>`.
6. NSIS installer runs silently.
7. New launcher starts with `--snb-update-run=<id>`.
8. The UI calls the health command once bootstrap succeeds; the helper sees `update-health.json`.
9. On success, the helper exits and keeps the previous-version backup for emergency use.
10. On installer failure, missing health marker (120 seconds) or startup failure, the helper restores the backup and relaunches the previous version.

Logs: `<launcher data>\logs\updater.log`.

## Emergency owner rollback

Use this when a published version passed its health check but is functionally bad.

1. **Stop publishing the bad version.** Do not leave the manifest pointing at it.
2. Build a fixed release from the previous good commit with a **new, higher version number**.
3. Publish and verify the fixed installer.
4. Update `launcher/update-manifest.json` to the fixed version. Clients that installed the bad build will see the mandatory update on next launch.
5. If the bad build cannot start, players can install the previous installer manually from GitHub Releases. The automatic helper backup is at:
   - `%APPDATA%\com.skyandbeyond.launcher\update-backups\` (Windows)
   - the equivalent app-data directory on other platforms.
6. If the update service is down, the launcher intentionally blocks; restore Pages availability rather than instructing players to bypass it.
7. Record the incident in the repository release notes, preserve logs, and rotate signing keys only if a private key was compromised.

Never delete the only working installer or private key. Never publish a manifest whose artifact URL/hash is not already live.
