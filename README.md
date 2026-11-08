# Sky & Beyond Launcher

Official desktop launcher for the **Sky & Beyond** Minecraft server. It installs a dedicated Minecraft **1.21.1** / NeoForge **21.1.248** / Java **21** instance, keeps the server address up to date from public GitHub Pages JSON, and manages mods, content, worlds, backups and diagnostics without touching the player's `.minecraft` folder.

## Player experience

- One central **Play** button; Minecraft opens at the main menu and never auto-connects.
- First-run wizard: username, installation directory, then real step-by-step installation.
- Automatic Java 21 detection or Eclipse Temurin JRE download from Adoptium.
- Official Mojang metadata and checksums; the official NeoForge installer creates the loader profile.
- AutoModpack 4.0.7 installed and protected. Server modpack synchronisation is left entirely to AutoModpack.
- French and English, dark and light themes, local preferences.
- Mods, shaders and resource packs with add/remove/disable; AutoModpack files cannot be disabled or removed.
- Verified zip backups of all worlds, configurable retention, restore with confirmation and a safety snapshot.
- Local diagnostic archive, redacted; nothing is uploaded.
- Mandatory transactional launcher updates with checksum/signature verification, rollback helper and health check.

## Security and privacy

- No telemetry, analytics, tracking or automatic uploads.
- Only the public **Playit** endpoint is used. The origin server IP is rejected in remote configuration, logs, diagnostics and user-facing errors.
- All remote configuration URLs require HTTPS. Downloads use size + SHA-1/SHA-256 checks; update installers can additionally use Ed25519 signatures.
- No Microsoft credentials, GitHub write token or server credential is embedded. Offline profiles are explicitly documented as offline-only.

## Branding (developer)

All in-app artwork comes from `appearance.json`: `logoUrl`, `backgroundUrl` and `playerIconUrl`. Drop files in `public/branding/` and use `/branding/...` paths, or use absolute HTTPS URLs. `repository/config/appearance.json` is compiled in as the build-time fallback, so these local values work without publishing the remote config; a remote config still overrides them. These values are developer-controlled; there is no player-facing image setting. With no values, the launcher uses a theme gradient and typographic monograms, and no artwork is hardcoded in the UI.

## Development

Requirements: Node.js 20+, Rust 1.77.2+, and the Tauri 2 Linux/Windows prerequisites. Java is not required to build.

```bash
npm ci
npm run build                 # TypeScript + Vite production build
cargo test -p snb-core -p snb-updater
npm run sidecar               # builds the update helper for this host
cargo check --workspace
npm run tauri dev             # desktop development
```

On Windows, `npm run sidecar && npm run tauri build` produces the NSIS `.exe` installer under `target/release/bundle/nsis/`. The included Windows workflow builds the same installer in CI. See [docs/RELEASE.md](docs/RELEASE.md).

## Repository layout

```text
crates/snb-core/       configuration, downloads, Minecraft/NeoForge, backups, diagnostics, updates
crates/snb-updater/    external transactional update/rollback helper (Tauri sidecar)
src-tauri/             Tauri commands, window lifecycle, packaging
src/                   React 19 + TypeScript UI (FR/EN, themes)
repository/            starter GitHub Pages JSON and asset folders
docs/                  owner, architecture, remote-config, release and troubleshooting guides
```

## Before publishing

1. `remoteBaseUrl` is set to the project Pages URL (`https://skyfoxmcdev.github.io/snb/`); update it if the repository is renamed.
2. Enable GitHub Pages with **GitHub Actions** as the source; the `Pages` workflow publishes `repository/` (replace every `change-me` endpoint/asset URL).
3. Set the update public key if signed updates are required, or leave it empty to require only SHA-256.
4. Build the Windows installer in CI and publish it with GitHub Releases.
5. Update `repository/launcher/update-manifest.json` with the final version, size, hash and URL.

See **docs/OWNER-GUIDE.md** for non-developer instructions.

## Verification status

At the time of delivery, the following commands were executed successfully:

- `cargo test -p snb-core -p snb-updater` — **91 tests passed** (89 core tests including update-manifest and starter-JSON validation, plus 2 updater-helper tests), 0 failed.
- `npm run build` — TypeScript and Vite production build succeeded.
- `npm run sidecar` — release helper built and installed as a target-triple sidecar.
- `cargo check --workspace` — succeeded.
- `npm run tauri build` — the Windows `.exe` cannot be produced on this Linux host; it is built by `.github/workflows/release-windows.yml`.

Not claimed as tested here: a full real Minecraft launch, a live Playit connection, a real GitHub Pages deployment and a Windows installer run on a clean PC. Those require the owner's actual endpoint, Pages URL, release signing material and a Windows environment.

## License

Proprietary — Sky & Beyond. All rights reserved.
