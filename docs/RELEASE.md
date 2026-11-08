# Building and releasing

## Local checks

```bash
npm ci
npm run build
cargo test -p snb-core -p snb-updater
npm run sidecar
cargo check --workspace
```

`npm run sidecar` builds `snb-updater` and copies it to `src-tauri/binaries/snb-updater-<rust-host-triple>[.exe]`. Tauri requires this file before bundling.

## Desktop development

```bash
npm run tauri dev
```

Linux needs the standard Tauri 2 libraries (WebKitGTK 4.1, GTK 3, librsvg, patchelf, OpenSSL). Windows needs the WebView2 runtime (installed by the NSIS package if missing) and the MSVC Rust toolchain.

## Windows installer

```powershell
npm ci
npm run build
npm run sidecar
npm run tauri build
```

Output: `target/release/bundle/nsis/*.exe`.

The app uses an NSIS current-user install. Verify on a clean Windows 10/11 VM:

1. Installer runs without admin.
2. Launcher opens and onboarding works.
3. Java, Minecraft, NeoForge, AutoModpack install.
4. Worlds/backups are in the selected instance, not `.minecraft`.
5. Play opens Minecraft at the main menu; launcher minimizes/restores.
6. Uninstall leaves player data intact.

## GitHub Actions

- `.github/workflows/ci.yml`: Linux tests/build and workspace compile.
- `.github/workflows/pages.yml`: publishes `repository/` to GitHub Pages.
- `.github/workflows/release-windows.yml`: Windows tests, sidecar, NSIS build, artifact upload; on a version tag it also creates/updates a GitHub Release.

The Windows workflow is the reproducible build path. It does not publish the Pages update manifest automatically. Enable Pages once (Settings → Pages → Source: GitHub Actions), then update the manifest after testing the generated installer, following [OWNER-GUIDE.md](OWNER-GUIDE.md#5-publish-a-release-and-update-manifest).

## Version bump checklist

- `package.json`
- `src-tauri/tauri.conf.json`
- `Cargo.toml` workspace version
- any explicit crate versions
- `repository/launcher/update-manifest.json` after upload
- release notes/`config/launcher.json` announcements if desired

Do not publish a manifest with a placeholder URL, zero hash or size of 1.

## Future targets

Add Linux/macOS targets only after implementing native bundles and updater commands. The current Tauri `targets` array advertises NSIS only; do not claim support for platforms without tested installers.
