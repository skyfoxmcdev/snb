# Branding assets (developer only)

Drop your own files in this folder and reference them from
`repository/config/appearance.json` with app-relative paths:

```json
{
  "logoUrl": "/branding/logo.png",
  "backgroundUrl": "/branding/background.jpg",
  "playerIconUrl": "/branding/player.png"
}
```

You can also use absolute HTTPS URLs instead. The launcher contains no
built-in logo, background photograph or player face: when a value is empty the
UI uses a theme gradient and a typographic monogram.

These values are read from the developer-controlled `appearance.json`; there
is no player-facing setting for them. Files placed here are bundled into the
application at build time, so rebuild after changing them. For artwork that
must change without shipping a new installer, host it over HTTPS and use the
URL in `appearance.json`.
