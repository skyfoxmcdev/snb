# Architecture

## Stack

- **Tauri 2** desktop shell, Rust backend, WebView frontend.
- **React 19 + TypeScript + Vite** UI, no telemetry SDK and no remote script execution.
- **`snb-core`**: pure testable Rust domain logic (no Tauri dependency).
- **`snb-updater`**: separate transactional update helper, bundled as a Tauri sidecar.
- Blocking `reqwest`/rustls downloads are executed in Tauri worker threads, never on the UI thread.

## Data boundaries

| Data | Default location |
| --- | --- |
| Launcher settings/logs/cache/update staging | OS app-data directory for `com.skyandbeyond.launcher` |
| Dedicated Minecraft instance | `~/.skyandbeyond` (player-selectable) |
| Worlds and backups | `<instance>/saves`, `<instance>/snb-backups` |
| Minecraft/NeoForge libraries/assets | `<instance>/versions`, `libraries`, `assets` |
| Launcher-managed marker/cache | `<instance>/.snb` |

The vanilla `.minecraft` directory is explicitly rejected as an instance directory during settings validation.

A single installation orchestrator runs ten idempotent steps: prepare, Java, Minecraft metadata/client, libraries, assets, NeoForge, AutoModpack, instance, server entry, verification. Each step emits real byte/file progress; no synthetic progress is used. All downloads are resumable `.part` files and checksum-verified before atomic rename. A failed step remains failed; the report is never marked ready unless the independent readiness checker passes.

## Modules

- `util`: atomic writes, hashes, safe joins, offline UUID, argument splitting, redaction/IP detection.
- `settings`: persisted preferences, memory/resolution validation, backup policy.
- `config` / `remote`: versioned remote schemas, HTTPS/IP validation, per-file caching and endpoint fallback.
- `http`: blocking downloader with resume, size/hash verification, cancellation and loopback test server.
- `server`: uncompressed NBT `servers.dat` create/update preserving other entries, plus server-list ping.
- `java`: detection, version parsing, Adoptium download, safe extraction, process execution with timeout/output limits.
- `minecraft`: Mojang manifest/version model, inheritance merge, rules, libraries, assets, completion marker.
- `neoforge`: checksum-verified official installer and generated profile validation.
- `automodpack`: valid-JAR detection, protection, repair without touching AutoModpack config/packs.
- `launch`: placeholder resolution, classpath, offline profile, argument construction, process spawning.
- `backups`: validated zip snapshots, retention, staging and transactional restore.
- `diagnostics`: local redacted zip export.
- `updater`: manifest, semver decisions, SHA-256 + optional Ed25519, staging and health markers.
- Tauri `main.rs`: state, commands, events, window lifecycle, sidecar handoff.

## Offline identity

The launcher creates an offline UUID (MD5-based Java name UUID), no Microsoft token and no fake premium API. The server must legitimately support the resulting offline/forwarding mode. This restriction is intentional and documented; the launcher is not responsible for the server's authentication configuration.

## Security model

- HTTPS-only remote URLs; loopback HTTP allowed only for local tests/debug.
- Endpoint cannot be a literal IP, localhost or an internal suffix; remote JSON is scanned for IP literals before acceptance.
- All remote text is length-limited and sanitised; URLs are validated; no remote command execution.
- Path traversal protections on downloads, jars, archives, backups, assets and diagnostics.
- Logs are redacted at write time; diagnostics redact IPs, secrets, tokens and the home directory.
- No admin elevation, no Windows auto-start, no background daemon.
- Auto-update helper is external to the running app directory and restores a full directory backup on any failure.

## Process lifecycle

Play verifies installation and Java, refreshes `servers.dat`, optionally creates a due automatic world backup, spawns Java with redirected logs, minimises the launcher, and starts a waiter thread. When Minecraft exits, the launcher restores and focuses itself and emits `game-exited`. A shared atomic flag prevents duplicate game/install/update activity.

## Constraints

- Minecraft/NeoForge version changes require code-level testing; the runtime configuration is intentionally internal.
- Windows is the only initially packaged platform. Linux/macOS still need bundle targets, platform installers and updater command adaptation.
- AutoModpack 4.0.7 does not expose pinned mod IDs; local/server conflict prevention is not claimed.
