# Sky & Beyond GitHub Pages repository

This folder is published to GitHub Pages at `https://skyfoxmcdev.github.io/snb/` by `.github/workflows/pages.yml`. GitHub Pages must be set to **GitHub Actions** as its source.

## Before publishing

1. Replace the Playit endpoint in `config/server.json`. **Never put the origin server IP anywhere.**
2. Replace every `change-me`/`example.com` link and asset URL with real HTTPS URLs.
3. Replace the placeholder release URL/hash/size in `launcher/update-manifest.json` after building the installer.
4. Add your real images under `assets/backgrounds`, `assets/logos` and `assets/banners`.

Every JSON file starts with `"schemaVersion": 1`. Do not remove it. The launcher validates every field and keeps the last valid cached copy when Pages is unavailable.

The complete field reference is in [`../docs/REMOTE-CONFIG.md`](../docs/REMOTE-CONFIG.md).

## Safe workflow

1. Edit a file in a local copy.
2. Validate it with `python -m json.tool <file>` or an online JSON validator.
3. Commit and push.
4. Wait for GitHub Pages to deploy.
5. Start the launcher and verify the changed content.

The release channel is stable. Keep `launcher/update-manifest.json` at the version currently published. The manifest is **not** the same as `config/launcher.json`: the update manifest controls the mandatory binary update; `config/launcher.json` controls announcements and links.
