# Remote configuration reference

The launcher fetches four public files in parallel and validates each independently:

```text
config/server.json
config/launcher.json
config/appearance.json
config/news.json
```

A fifth file controls mandatory binary updates: `launcher/update-manifest.json`.

All files use `"schemaVersion": 1`. Invalid files are rejected individually; the last valid cache is used when available, otherwise safe defaults. An unavailable file never crashes the launcher. All remote URLs must be HTTPS; appearance image paths may also be app-relative (for example `/branding/logo.png`). The origin server IP is rejected.

Localized text accepts either a plain string (used for both languages) or an object:

```json
{ "en": "English text", "fr": "Texte français" }
```

## server.json

| Field | Type | Notes |
| --- | --- | --- |
| `schemaVersion` | number | Must be `1`. |
| `endpoint.host` | string | **Public Playit hostname only.** An IPv4/IPv6 literal is rejected. |
| `endpoint.port` | number | 1–65535. |
| `displayName` | string | Shown in the UI; max 80 chars. |
| `description` | localized | Short description. |
| `maintenance` | boolean | Shows a maintenance notice. |
| `maintenanceMessage` | localized | Maintenance text. |
| `info` | array | Optional rows: `{ "label": localized, "value": localized }`. |

The launcher pings the endpoint to display online/offline/unknown and writes the `servers.dat` entry named **Sky & Beyond**. It never auto-connects.

### Contract for the owner's server-side service

The separate service that owns the Playit tunnel must publish only:

```json
{
  "host": "current-public-endpoint.at.playit.gg",
  "port": 25565
}
```

It must never publish, log or return the origin IP. Keep origin credentials off this public path. The launcher can consume an endpoint-only update JSON if you adapt it, but the shipped schema expects the full `server.json`. The service should update the file when the tunnel changes. Clients receive endpoint changes on next startup; no continuous polling is required.

## launcher.json

| Field | Type | Notes |
| --- | --- | --- |
| `schemaVersion` | number | Must be `1`. |
| `latestVersion` | semver/null | Informational; the binary update manifest is authoritative. |
| `minimumVersion` | semver/null | Informational. |
| `notes` | localized | Release notes. |
| `announcements` | array | Up to 10. See below. |
| `buttons` | array | Up to 6 custom external buttons. |
| `links` | object | Optional `discord`, `website`, `support` HTTPS URLs. |
| `maintenance.active` | boolean | Launcher maintenance notice. |
| `maintenance.message` | localized | Maintenance text. |

Announcement fields: `id`, `level` (`info`, `important`, `maintenance`, `event`, `warning`), `title`, `body`, optional `startsAt`, `endsAt`, `link`, and `dismissible`. Duplicate IDs are removed. Links must be HTTPS.

Button fields: `id`, `label` (localized), `url` (HTTPS), `icon` (short name), `style` (`primary` or `secondary`).

## appearance.json

| Field | Notes |
| --- | --- |
| `logoUrl` | Optional image: HTTPS URL or app-relative path (`/branding/logo.png`). |
| `backgroundUrl` | Optional main background: HTTPS URL or app-relative path. |
| `bannerUrl` | Optional banner: HTTPS URL or app-relative path. |
| `playerIconUrl` | Optional account icon: HTTPS URL or app-relative path. Players cannot change it in the UI. |
| `tagline` | Localized short tagline. |
| `colors` | Dark theme palette. |
| `lightColors` | Light theme palette (falls back to sensible light defaults per field). |
| `backgroundOverlay` | Number 0–0.9. |

Palette keys: `background`, `surface`, `text`, `muted`, `border`, `primary`, `secondary`, `accent`. Values must be valid hex colours. Invalid values fall back to defaults.

### Brand assets (developer only)

The launcher ships no logo, background photograph or player face. All of them come from `appearance.json`:

```json
{
  "logoUrl": "/branding/logo.png",
  "backgroundUrl": "/branding/background.jpg",
  "playerIconUrl": "/branding/player.png"
}
```

Put the files in `public/branding/` (they are bundled at build time), or use absolute HTTPS URLs so they can change without a new installer. `repository/config/appearance.json` is also compiled into the launcher as the build-time fallback, so local `/branding/...` values work even when the remote configuration is unreachable; a valid remote `appearance.json` takes precedence. When a value is empty or fails to load, the UI falls back to the theme gradient and a typographic monogram. These fields are not exposed in Settings: only the developer edits `appearance.json`, so players cannot replace the images from the launcher.

Changes take effect on the next launcher startup. Players can override theme mode (system/dark/light) and language in Settings.

## news.json

Fields per item: `id`, `title`, `summary`, `body`, optional `imageUrl` (HTTPS), optional `publishedAt` (ISO-8601), optional `link` (HTTPS), `pinned`. Titles are required; up to 30 items are kept. Items are sanitised and duplicate IDs removed.

## Update manifest

See [UPDATES-AND-ROLLBACK.md](UPDATES-AND-ROLLBACK.md). The shipped starter is `repository/launcher/update-manifest.json`.

## Caching and failure behavior

- Valid configuration is cached in the launcher data directory under `remote-cache/`.
- Each file falls back independently.
- The last valid Playit endpoint is stored separately and reused if the new `server.json` lacks one.
- The launch update manifest is **not** cached: a stale manifest could bypass or fabricate mandatory updates.
- Remote configuration can never execute commands or code.
- The launcher displays a warning and continues with cached/default settings when possible.
