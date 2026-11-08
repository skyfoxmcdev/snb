# Troubleshooting

## The launcher says installation required

Open Settings → **Repair installation**. Repair verifies launcher-managed files and redownloads corrupt/missing components. It preserves worlds, settings, local mods, shaders and resource packs. If Java is missing and auto-download is disabled, enable it or select a Java 21 executable manually.

## Java problems

- The launcher requires **Java 21**. Other versions are listed but cannot be selected as valid.
- Auto-download uses Eclipse Temurin from Adoptium into `<instance>/java/runtime-21`.
- If a manually selected Java is invalid, clear the field and press Repair or choose another executable.
- Antivirus/controlled-folder-access can prevent extraction. Allow the launcher and the instance directory.
- Java logs are under `<instance>/logs/`.

## Minecraft installed but Play fails

1. Open Settings → Repair.
2. Ensure the game directory is not the vanilla `.minecraft`.
3. Check free disk space (assets alone are roughly 826 MB; leave at least 2 GB free).
4. Export diagnostics and inspect the newest `logs/game-*.log`.
5. NeoForge installer failures are reported with its output tail; rerun Repair after fixing permissions/network.

## Server status offline / endpoint warning

- The launcher needs the latest public Playit endpoint from GitHub Pages.
- If Pages is temporarily unavailable, the last valid cached endpoint is used.
- If the server is genuinely offline, Play remains available and Minecraft still opens.
- Never add the origin IP. If the Playit endpoint changes, update `config/server.json` in Pages.
- To refresh immediately, restart the launcher (endpoint checks happen at startup) or use the server refresh button; the multiplayer entry is updated on next Play.

## Server entry missing or stale

The launcher rewrites the entry named **Sky & Beyond** in `servers.dat`. Existing unrelated entries are preserved. If the file is corrupt it is backed up first and recreated. Start the launcher, wait for endpoint refresh, then click Play (this updates the entry). Do not edit `servers.dat` while Minecraft is running.

## Mods

- Disable moves the JAR to `mods-disabled`; it is never deleted by disabling.
- AutoModpack JARs are protected and cannot be disabled/removed from the launcher.
- Add only mods compatible with Minecraft 1.21.1 / NeoForge; the launcher deliberately does not block based on speculative compatibility checks.
- If AutoModpack and a local mod disagree, remove/disable the local copy or ask the server owner. AutoModpack 4.0.7 does not offer pinned-mod-id support.
- `.jar` files only. Invalid/corrupt mods may make Minecraft fail to start; inspect the game log.

## Shaders/resource packs

Use the Shaders / Resource packs panels or open the folders from Settings. Add `.zip` archives; remove through the panel. Iris/Oculus/shaders must already be supported by the server/client mod set. The launcher does not provide an online catalogue.

## Backups and worlds

- Close Minecraft before backup/restore; the launcher refuses while the game is running.
- Backups live under `<instance>/snb-backups`.
- Restore requires confirmation and creates a safety snapshot of current worlds first.
- An interrupted backup leaves a `.part` file that is ignored and cleaned on the next backup; existing backups are untouched.
- The launcher never deletes a backup before a new backup is fully verified.

## Launcher update is blocked

This is intentional. The launcher cannot verify/install the mandatory update. Check internet access, GitHub Pages availability and that `launcher/update-manifest.json` has a valid SHA-256, size, HTTPS URL and platform entry. Click Retry. Do not instruct players to bypass it.

If the new version fails to start, the updater helper restores the previous version automatically. Owner recovery is documented in [UPDATES-AND-ROLLBACK.md](UPDATES-AND-ROLLBACK.md).

## Where are the logs?

- Launcher: app-data `logs/launcher-*.log` (rotating, redacted).
- Updater: app-data `logs/updater.log`.
- Minecraft: `<instance>/logs/latest.log`, `debug.log` and game logs.
- Diagnostics: use **Export diagnostics**; the archive is created locally and must be sent manually. It contains no telemetry and is redacted.

## Reset only the launcher UI

Delete `settings.json` in the app-data directory while the launcher is closed. This resets preferences but not the game or worlds. Do not delete the whole instance to fix a UI issue.

## Windows SmartScreen

An unsigned installer can show SmartScreen. Code-sign releases with an Authenticode certificate for public distribution. Never tell players to disable Windows security globally.
