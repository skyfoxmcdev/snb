# Sky & Beyond Launcher — Owner Guide

This guide is for the Sky & Beyond project owner. It explains what must be configured, how to publish the launcher, and how to operate it safely.

## 1. What you own

There are three separate pieces:

1. **Launcher repository** (this project): source code, Windows installer, automatic updater helper.
2. **GitHub Pages repository**: JSON files in `repository/` that players fetch at startup.
3. **Server-side Playit/hosting service** (outside this project): publishes only the current public Playit endpoint.

The launcher never needs the origin server's public IP. Never publish it, paste it into JSON, or place it on the server machine's public web pages.

## 2. Configure the launcher internals

Edit `src-tauri/resources/launcher-config.json` before building:

| Field | Meaning |
| --- | --- |
| `remoteBaseUrl` | Root HTTPS URL of the published GitHub Pages site. Must end in `/`. |
| `minecraft.version` | Pinned Minecraft version (`1.21.1`). Change only with a tested release. |
| `minecraft.manifestUrl` | Official Mojang version manifest. |
| `neoforge.version` | Pinned NeoForge version (`21.1.248`). |
| `neoforge.mavenBase` | Official NeoForge Maven base. |
| `javaMajor` | Required major Java version (`21`). |
| `adoptiumApiBase` | Official Adoptium API base. |
| `automodpack.version/url/sha256/fileName` | Pinned AutoModpack artifact. |
| `update.channel` | `stable` or `beta`. Stable reads `launcher/update-manifest.json`. |
| `update.publicKey` | Optional raw 32-byte Ed25519 public key in base64. Empty means SHA-256-only updates. |
| `serverEntryName` | Name of the multiplayer entry (`Sky & Beyond`). |
| `backupFolderName` | Backup folder inside the instance (`snb-backups`). |

The launcher embeds this file at build time. Rebuild and release after changing it.

## 3. Publish the GitHub Pages repository

Copy `repository/` to the public Pages repository. Replace all placeholders:

- `config/server.json` — Playit **hostname** and port only, never an IP.
- `config/launcher.json` — announcements, links, custom buttons, maintenance.
- `config/appearance.json` — logo/background/banner URLs and colours.
- `config/news.json` — news items.
- `launcher/update-manifest.json` — release manifest (see below).
- `assets/` — images referenced by HTTPS.

Every file has `schemaVersion: 1`. Validate JSON before pushing. This repository includes `.github/workflows/pages.yml`, which publishes `repository/` to `https://skyfoxmcdev.github.io/snb/`. In GitHub, open **Settings → Pages → Build and deployment** and set **Source** to **GitHub Actions** once. The default `GITHUB_TOKEN` cannot enable Pages by itself.

If Pages is unavailable, the launcher uses the last valid cached configuration and the last known valid endpoint. It warns but does not crash. The **Play** button remains available when the endpoint is cached, as required. If no endpoint has ever been cached, the installer still completes but the server entry cannot be prepared; the launcher warns.

## 4. Build the Windows installer

Windows packaging cannot be performed on a Linux machine. Use one of:

- The included GitHub Actions workflow `.github/workflows/release-windows.yml` (recommended).
- A Windows 10/11 machine with Node.js 20+, Rust MSVC, and Tauri 2 prerequisites.

On Windows:

```powershell
npm ci
npm run build
npm run sidecar
npm run tauri build
```

The NSIS installer is written under `target/release/bundle/nsis/`. The sidecar `snb-updater.exe` is bundled and installed next to the launcher.

The included Windows workflow is the reproducible build path; test the installer on a clean Windows VM before wide distribution.

## 5. Publish a release and update manifest

1. Set the launcher version in `package.json`, `Cargo.toml` workspace/package files and `src-tauri/tauri.conf.json` (all currently `0.1.0`).
2. Build and test the installer on Windows.
3. Upload the `.exe` to a GitHub Release.
4. Compute SHA-256 and size:
   ```bash
   sha256sum "Sky-and-Beyond_x.y.z_x64-setup.exe"
   stat -c %s "Sky-and-Beyond_x.y.z_x64-setup.exe"
   ```
   On Windows PowerShell:
   ```powershell
   Get-FileHash -Algorithm SHA256 .\setup.exe
   (Get-Item .\setup.exe).Length
   ```
5. Update `repository/launcher/update-manifest.json` with the **new** version, minimum version, exact HTTPS URL, size, SHA-256 and filename.
6. Optional: sign the installer and put the base64 Ed25519 signature in `signature`. Verify the public key in `launcher-config.json`.
7. Commit the manifest. Launch the old version once to verify the mandatory update prompt.

If you publish the manifest before the installer URL is live, players will be blocked. If you publish a bad release, follow [UPDATES-AND-ROLLBACK.md](UPDATES-AND-ROLLBACK.md).

## 6. Updates and rollback

The launcher checks the stable update manifest at every startup in release builds. If a newer version or a version below `minimumVersion` is published, a blocking update screen appears. The player cannot bypass it.

The update process:

1. Download to a staging directory.
2. Check size, SHA-256 and optional Ed25519 signature.
3. Start the external `snb-updater` helper.
4. Back up the current launcher directory.
5. Install the NSIS package silently.
6. Start the new launcher with a run identifier.
7. Wait for the UI health marker.
8. If installation or health fails, restore the backup and relaunch the old version.

The previous version remains available under `%APPDATA%\com.skyandbeyond.launcher\update-backups\`. Updater logs are in the launcher data directory under `logs/updater.log`.

If the update service itself is unavailable, the launcher blocks normal use with a clear error instead of silently bypassing the mandatory check. Settings/diagnostics can still be opened from the blocked screen (logs are accessible via the button).

## 7. Player data and world safety

- Dedicated instance default: `%USERPROFILE%\.skyandbeyond` (or platform equivalent).
- Launcher data: `%APPDATA%\com.skyandbeyond.launcher`.
- Backups: `<instance>\snb-backups`, zipped and validated before the old backup policy applies.
- Automatic backups run before launch when the configured interval is due. A game must not be running for backup or restore.
- Restoring requires confirmation, creates a safety snapshot first, extracts into staging, then replaces `saves` transactionally.
- Repair never deletes worlds, settings, local mods or resource packs.

## 8. AutoModpack

The launcher installs the pinned AutoModpack JAR into `mods/` and never modifies `automodpack/` configuration or downloaded packs. AutoModpack may update itself or synchronize the server version; the launcher preserves any valid AutoModpack JAR even if its version differs. Missing or corrupt JARs are repaired. Launcher-managed AutoModpack files are protected from the mod UI.

AutoModpack 4.0.7 has **no pinned-mod-id feature**. Do not claim in-game that client-side mod conflicts are automatically prevented. Document conflicts to players; server-side AutoModpack remains authoritative for synchronised mods.

## 9. Privacy and security checklist

- [ ] No analytics, telemetry, crash upload or tracking.
- [ ] No origin IP in JSON, logs, diagnostics, errors, assets or releases.
- [ ] No GitHub write token, private key, Microsoft token or server credential in the launcher.
- [ ] All remote URLs HTTPS.
- [ ] Update manifest SHA-256 verified; optional Ed25519 key configured.
- [ ] Public repository contains only public JSON/assets/releases.
- [ ] Do not enable Windows auto-start.
- [ ] Keep launcher open while Minecraft runs; it minimises and restores itself.

## 10. Linux and macOS

The core engine, Tauri config, CI and UI are portable, but the initial supported distribution is Windows. To add Linux/macOS:

1. Add a native Tauri bundle target (AppImage/deb, macOS `.app`/`.dmg`) and platform icon.
2. Add a platform-specific update artifact (`.AppImage`, `.dmg`) to the manifest.
3. Update `snb-updater` installation commands for that platform.
4. Build and test on the target OS.
5. Change `bundle.targets` only when a real tested build is ready.

Do not advertise Linux/macOS support until those steps are complete.
