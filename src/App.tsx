import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./styles.css";
import { pickText, strings } from "./i18n";
import { brandingDefaults } from "./branding";
import type {
  Backup,
  Bootstrap,
  ContentView,
  InstallReport,
  Lang,
  ModView,
  MojangState,
  ServerStatus,
  Settings,
} from "./types";
import { BackgroundLayer } from "./ui/Background";
import { TitleBar } from "./ui/TitleBar";
import { BrandMark } from "./ui/Logo";
import { AccountSection } from "./ui/AccountSection";
import { IconRail } from "./ui/IconRail";
import { BottomBar } from "./ui/BottomBar";
import { NewsPanel } from "./ui/NewsPanel";
import { ServerPopover } from "./ui/ServerPopover";
import { LibraryPanel, type LibraryKind } from "./ui/LibraryPanel";
import { InstallProgress } from "./ui/InstallProgress";
import { Onboarding } from "./ui/Onboarding";
import { UpdateGate } from "./ui/UpdateGate";

function humanError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "code" in error) return String((error as { code: unknown }).code);
  return "unknown_error";
}

function installError(error: InstallReport["error"]): string {
  if (!error) return "unknown_error";
  return error.detail ? `${error.code} · ${error.detail}` : error.code;
}

export default function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [installReport, setInstallReport] = useState<InstallReport | null>(null);
  const [installing, setInstalling] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [library, setLibrary] = useState<LibraryKind | null>(null);
  const [contentKind, setContentKind] = useState("shader");
  const [mods, setMods] = useState<ModView[]>([]);
  const [content, setContent] = useState<ContentView[]>([]);
  const [backups, setBackups] = useState<Backup[]>([]);
  const [onboardName, setOnboardName] = useState("");
  const [onboardDir, setOnboardDir] = useState("");
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<{ current: number; total: number } | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [worlds, setWorlds] = useState<string[]>([]);
  const [selectedWorld, setSelectedWorld] = useState("");
  const [copied, setCopied] = useState(false);
  const [newsOpen, setNewsOpen] = useState(false);
  const [accountOpen, setAccountOpen] = useState(false);
  const [serverOpen, setServerOpen] = useState(false);
  const [mojang, setMojang] = useState<MojangState>("unknown");
  const [mojangChecked, setMojangChecked] = useState(false);

  const settings = boot?.settings;
  const lang: Lang =
    settings?.language === "fr"
      ? "fr"
      : settings?.language === "en"
        ? "en"
        : navigator.language.startsWith("fr")
          ? "fr"
          : "en";
  const t = useCallback((key: string) => strings[lang][key] || strings.en[key] || key, [lang]);

  const refresh = useCallback(
    async (quiet = false) => {
      if (!quiet) setLoading(true);
      try {
        const payload = await invoke<Bootstrap>("bootstrap");
        setBoot(payload);
        setError(null);
        if (payload.updateRunId) {
          void invoke("mark_update_ready").catch(() => undefined);
        }
        setOnboardName((current) => current || payload.settings.username);
        setOnboardDir((current) => current || payload.settings.gameDir);
      } catch (cause) {
        setError(humanError(cause));
      } finally {
        if (!quiet) setLoading(false);
      }
    },
    [],
  );

  useEffect(() => {
    void refresh();
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    void listen<InstallReport>("install-progress", (event) => {
      if (!disposed) {
        setInstallReport(event.payload);
        setInstalling(event.payload.running);
      }
    }).then((un) => unlisteners.push(un));
    void listen<InstallReport>("install-finished", (event) => {
      if (disposed) return;
      setInstallReport(event.payload);
      setInstalling(false);
      if (event.payload.ready) void refresh(true);
    }).then((un) => unlisteners.push(un));
    void listen("settings-changed", () => {
      if (!disposed) void refresh(true);
    }).then((un) => unlisteners.push(un));
    void listen("game-started", () => {
      if (!disposed) setLaunching(false);
    }).then((un) => unlisteners.push(un));
    void listen("game-exited", () => {
      if (!disposed) {
        setLaunching(false);
        void refresh(true);
      }
    }).then((un) => unlisteners.push(un));
    void listen<{ current: number; total: number }>("update-progress", (event) => {
      if (!disposed) setUpdateProgress(event.payload);
    }).then((un) => unlisteners.push(un));
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [refresh]);

  useEffect(() => {
    if (!boot) return;
    const root = document.documentElement;
    const theme =
      boot.settings.theme === "system"
        ? window.matchMedia?.("(prefers-color-scheme: light)").matches
          ? "light"
          : "dark"
        : boot.settings.theme;
    root.dataset.theme = theme;
    const colors = theme === "light" ? boot.remote?.appearance?.lightColors : boot.remote?.appearance?.colors;
    if (colors) {
      if (colors.primary) root.style.setProperty("--accent", colors.primary);
      if (colors.accent) root.style.setProperty("--accent-2", colors.accent);
      if (colors.background) root.style.setProperty("--bg", colors.background);
    }
    document.documentElement.lang = lang;
  }, [boot, lang]);

  useEffect(() => {
    if (!boot || mojangChecked) return;
    setMojangChecked(true);
    let cancelled = false;
    void invoke<string>("mojang_status")
      .then((state) => {
        if (cancelled) return;
        if (state === "operational" || state === "degraded" || state === "unavailable") setMojang(state);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [boot, mojangChecked]);

  const news = useMemo(() => {
    const source = boot?.remote?.news?.items || boot?.remote?.news?.articles || [];
    return Array.isArray(source) ? source.slice(0, 30) : [];
  }, [boot]);

  const announcements = useMemo(() => {
    const source = boot?.remote?.launcher?.announcements;
    return Array.isArray(source) ? source.slice(0, 10) : [];
  }, [boot]);

  const customButtons = useMemo(() => {
    const source = boot?.remote?.launcher?.buttons;
    return Array.isArray(source) ? source.slice(0, 6) : [];
  }, [boot]);

  const saveProfile = async () => {
    if (!settings) return;
    const name = onboardName.trim();
    if (!/^[A-Za-z0-9_]{3,16}$/.test(name)) {
      setError(t("onboard_username"));
      return;
    }
    setBusyAction("profile");
    try {
      const next = { ...settings, username: name, gameDir: onboardDir.trim(), onboardingComplete: true };
      const saved = await invoke<Settings>("save_settings", { settings: next });
      setBoot((current) => (current ? { ...current, settings: saved } : current));
      await runInstall(false);
    } catch (cause) {
      setError(humanError(cause));
    } finally {
      setBusyAction(null);
    }
  };

  const runInstall = async (repair: boolean) => {
    setError(null);
    setInstalling(true);
    setInstallReport(null);
    try {
      const report = await invoke<InstallReport>("install_game", { repair });
      setInstallReport(report);
      if (!report.ready && report.error) setError(installError(report.error));
      if (report.ready) await refresh(true);
    } catch (cause) {
      setError(humanError(cause));
    } finally {
      setInstalling(false);
    }
  };

  const play = async () => {
    setLaunching(true);
    setError(null);
    try {
      await invoke("launch_game");
    } catch (cause) {
      setError(humanError(cause));
      setLaunching(false);
    }
  };

  const startUpdate = async () => {
    setUpdating(true);
    setUpdateError(null);
    try {
      await invoke("install_update");
    } catch (cause) {
      setUpdateError(humanError(cause));
      setUpdating(false);
    }
  };

  useEffect(() => {
    if (library !== "mods") return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (disposed || event.payload.type !== "drop") return;
        const jars = event.payload.paths.filter((path) => path.toLowerCase().endsWith(".jar"));
        if (jars.length === 0) return;
        void invoke<number>("add_mod_paths", { paths: jars }).then(async (added) => {
          await loadMods();
          if (added < jars.length) setError("some_mods_skipped");
        });
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [library]);

  const loadMods = async () => setMods(await invoke<ModView[]>("list_mods"));

  const openMods = async () => {
    setLibrary("mods");
    await loadMods();
  };

  const loadDatapacks = async (world: string) => {
    setSelectedWorld(world);
    setContent(world ? await invoke<ContentView[]>("list_datapacks", { world }) : []);
  };

  const loadContent = async (kind: string) => {
    setContentKind(kind);
    if (kind === "datapack") {
      const available = await invoke<string[]>("list_worlds");
      setWorlds(available);
      const world = available.includes(selectedWorld) ? selectedWorld : available[0] || "";
      setSelectedWorld(world);
      setContent(world ? await invoke<ContentView[]>("list_datapacks", { world }) : []);
      return;
    }
    setContent(await invoke<ContentView[]>("list_content", { kind }));
  };

  const loadBackups = async () => {
    setBackups(await invoke<Backup[]>("get_backups"));
    setLibrary("backups");
  };

  const openContent = async () => {
    setLibrary("content");
    await loadContent("shader");
  };

  const refreshServer = async () => {
    const status = await invoke<ServerStatus>("refresh_server");
    setBoot((current) => (current ? { ...current, serverStatus: status } : current));
  };

  const openUrl = (url: string) => void invoke("open_url", { url });

  const copySummary = async () => {
    if (!boot) return;
    try {
      await navigator.clipboard.writeText(
        JSON.stringify(
          {
            launcherVersion: boot.launcherVersion,
            os: navigator.userAgent,
            runtime: boot.runtime,
            readiness: boot.readiness,
            serverState: boot.serverStatus.state,
            mods: mods.map((mod) => mod.fileName),
          },
          null,
          2,
        ),
      );
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      setError("clipboard_unavailable");
    }
  };

  const closeWindow = () => void getCurrentWindow().close();
  const minimizeWindow = () => void getCurrentWindow().minimize();
  const toggleMaximize = () => void getCurrentWindow().toggleMaximize();

  if (loading && !boot) {
    return (
      <div className="launcher">
        <div className="splash">
          <div className="splash-inner">
            <BrandMark size={46} />
            <p className="menu-eyebrow">Sky &amp; Beyond</p>
            <h1>{t("loading")}</h1>
          </div>
        </div>
      </div>
    );
  }

  if (!boot) {
    return (
      <div className="launcher">
        <div className="splash">
          <div className="splash-inner splash-inner--error">
            <BrandMark size={46} />
            <h1>{t("fatal")}</h1>
            <p className="splash-detail">{error}</p>
            <button type="button" className="solid-button" onClick={() => void refresh()}>
              {t("retryUpdate")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  const needsOnboarding = !boot.settings.onboardingComplete || !boot.settings.username;
  const appearance = boot.remote?.appearance;
  const backgroundUrl: string | undefined =
    appearance?.backgroundUrl || appearance?.backgroundImage || brandingDefaults.backgroundUrl;
  const logoUrl: string | undefined = appearance?.logoUrl || brandingDefaults.logoUrl;
  const playerIconUrl: string | undefined = appearance?.playerIconUrl || brandingDefaults.playerIconUrl;
  const currentPercent = (() => {
    const progress = installReport?.steps.find((step) => step.id === installReport.currentStep)?.progress;
    if (!progress || !progress.total) return null;
    return Math.min(100, Math.round((progress.current / progress.total) * 100));
  })();

  const playerCount = boot.serverStatus.playersOnline;
  const playerMax = boot.serverStatus.playersMax;
  const playLabel = boot.gameRunning
    ? t("inGame")
    : launching
      ? t("launching")
      : installing
        ? t("installing")
        : !boot.readiness.ready
          ? t("install")
          : t("play");
  const playDisabled = installing || launching || boot.gameRunning;

  const maintenanceMessage = boot.remote?.launcher?.maintenance?.active
    ? pickText(boot.remote?.launcher?.maintenance?.message, lang) || t("maintenance")
    : boot.remote?.server?.maintenance
      ? pickText(boot.remote?.server?.maintenanceMessage, lang) || t("maintenance")
      : "";
  const hasNotice = Boolean(error || maintenanceMessage || (boot.update?.decision && !boot.update.required));

  return (
    <div className="launcher">
      <BackgroundLayer
        imageUrl={backgroundUrl}
        overlay={typeof appearance?.backgroundOverlay === "number" ? appearance.backgroundOverlay : 0.55}
        dimmed={needsOnboarding || Boolean(boot.update?.required)}
      />

      <div className="app-shell">
        <TitleBar
          title="Sky & Beyond Launcher"
          onMinimize={minimizeWindow}
          onMaximize={toggleMaximize}
          onClose={closeWindow}
        />

        {boot.update?.required && (
          <UpdateGate
            update={boot.update}
            logoUrl={logoUrl}
            updating={updating}
            progress={updateProgress}
            error={updateError}
            t={t}
            onInstall={() => void startUpdate()}
            onRetry={() => void refresh()}
            onLogs={() => void invoke("open_folder", { kind: "logs" })}
          />
        )}

        {needsOnboarding ? (
          <Onboarding
            logoUrl={logoUrl}
            name={onboardName}
            dir={onboardDir}
            busy={Boolean(busyAction)}
            installing={installing}
            report={installReport}
            percent={currentPercent}
            error={error}
            t={t}
            onName={setOnboardName}
            onDir={setOnboardDir}
            onBrowse={async () => {
              const picked = await invoke<string | null>("pick_directory");
              if (picked) setOnboardDir(picked);
            }}
            onBegin={() => void saveProfile()}
          />
        ) : (
          <>
            <div className="brand">
              <div className="brand-mark">
                <BrandMark logoUrl={logoUrl} name="Sky & Beyond" size={88} />
              </div>
            </div>

            <AccountSection
              username={boot.settings.username}
              avatarUrl={playerIconUrl}
              open={accountOpen}
              version={boot.launcherVersion}
              onToggle={() => {
                setAccountOpen((open) => !open);
                setServerOpen(false);
              }}
              onClose={() => setAccountOpen(false)}
              onSettings={() => {
                setAccountOpen(false);
                void invoke("open_settings_window");
              }}
              onFolder={() => {
                setAccountOpen(false);
                void invoke("open_folder", { kind: "game" });
              }}
              onBackups={() => {
                setAccountOpen(false);
                void loadBackups();
              }}
              onMods={() => {
                setAccountOpen(false);
                void openMods();
              }}
              labels={{
                account: t("account"),
                settings: t("settings"),
                folder: t("folder"),
                backups: t("backups"),
                mods: t("mods"),
              }}
            />

            <IconRail
              onSettings={() => void invoke("open_settings_window")}
              onMods={() => void openMods()}
              onContent={() => void openContent()}
              onBackups={() => void loadBackups()}
              onOpenUrl={openUrl}
              links={{
                website: boot.remote?.launcher?.links?.website,
                discord: boot.remote?.launcher?.links?.discord,
                support: boot.remote?.launcher?.links?.support,
              }}
            />

            {installing && installReport && (
              <div className="stage-progress">
                <InstallProgress report={installReport} t={t} percent={currentPercent} />
                <div className="stage-progress-actions">
                  <button type="button" className="text-link" onClick={() => void invoke("cancel_install")}>
                    {t("cancel")}
                  </button>
                </div>
              </div>
            )}

            {hasNotice && (
              <div className="stage-notices">
                {error && (
                  <div className="stage-notice">
                    <span className="status-dot status-dot--offline" aria-hidden="true" />
                    <span>{error}</span>
                    {installReport?.error && !installReport.ready && !installing && (
                      <button type="button" className="text-link" onClick={() => void runInstall(true)}>
                        {t("repair")}
                      </button>
                    )}
                  </div>
                )}
                {maintenanceMessage && (
                  <div className="stage-notice">
                    <span className="status-dot status-dot--degraded" aria-hidden="true" />
                    <span>{maintenanceMessage}</span>
                  </div>
                )}
                {boot.update?.decision && !boot.update.required && (
                  <div className="stage-notice">
                    <span className="status-dot status-dot--degraded" aria-hidden="true" />
                    <span>
                      {t("updateAvailable")} {boot.update.decision.latestVersion}
                    </span>
                    <button
                      type="button"
                      className="text-link"
                      disabled={updating}
                      onClick={() => void startUpdate()}
                    >
                      {t("installUpdate")}
                    </button>
                  </div>
                )}
              </div>
            )}

            <BottomBar
              playersOnline={playerCount}
              playersMax={playerMax}
              mojang={mojang}
              newsOpen={newsOpen}
              onNews={() => {
                setNewsOpen((open) => !open);
                setAccountOpen(false);
                setServerOpen(false);
              }}
              playLabel={playLabel}
              playDisabled={playDisabled}
              onPlay={() => {
                if (boot.readiness.ready) void play();
                else void runInstall(false);
              }}
              serverName={boot.remote?.server?.displayName || "Sky & Beyond"}
              serverOnline={boot.serverStatus.state === "online"}
              onServer={() => {
                setServerOpen((open) => !open);
                setAccountOpen(false);
              }}
              labels={{ players: t("playersLabel"), mojang: t("mojang"), news: t("news") }}
            />

            <ServerPopover
              open={serverOpen}
              onClose={() => setServerOpen(false)}
              name={boot.remote?.server?.displayName || "Sky & Beyond"}
              description={pickText(boot.remote?.server?.description, lang)}
              status={boot.serverStatus}
              info={Array.isArray(boot.remote?.server?.info) ? boot.remote.server.info : []}
              maintenance={boot.remote?.server?.maintenance ? pickText(boot.remote?.server?.maintenanceMessage, lang) : ""}
              lang={lang}
              labels={{
                serverDetails: t("serverDetails"),
                players: t("playersLabel"),
                latency: t("latency"),
                refresh: t("refresh"),
                status: t("status"),
              }}
              onRefresh={() => void refreshServer()}
            />

            <NewsPanel
              open={newsOpen}
              onClose={() => setNewsOpen(false)}
              news={news}
              announcements={announcements}
              buttons={customButtons}
              lang={lang}
              labels={{
                news: t("news"),
                noNews: t("noNews"),
                exportDiag: t("exportDiag"),
                copyDiag: t("copyDiag"),
                copied: t("copied"),
                openLogs: t("openLogs"),
              }}
              copied={copied}
              onOpenUrl={openUrl}
              onExportDiag={() => void invoke("export_diagnostics").catch((cause) => setError(humanError(cause)))}
              onCopySummary={() => void copySummary()}
              onOpenLogs={() => void invoke("open_folder", { kind: "logs" })}
            />

            {library && (
              <LibraryPanel
                kind={library}
                lang={lang}
                t={t}
                mods={mods}
                content={content}
                contentKind={contentKind}
                worlds={worlds}
                selectedWorld={selectedWorld}
                backups={backups}
                onClose={() => setLibrary(null)}
                onAddMods={async () => {
                  await invoke("pick_and_add_mods");
                  await loadMods();
                }}
                onToggleMod={async (mod) => {
                  await invoke("toggle_mod", { fileName: mod.fileName, enabled: !mod.enabled });
                  await loadMods();
                }}
                onRemoveMod={async (mod) => {
                  if (!confirm(`${t("remove")} ${mod.fileName} ?`)) return;
                  await invoke("remove_mod", { fileName: mod.fileName });
                  await loadMods();
                }}
                onContentKind={(kind) => void loadContent(kind)}
                onSelectWorld={(world) => void loadDatapacks(world)}
                onPickContent={async () => {
                  if (contentKind === "datapack") {
                    if (!selectedWorld) {
                      setError(t("worldMissing"));
                      return;
                    }
                    await invoke("pick_and_add_datapacks", { world: selectedWorld });
                    await loadContent("datapack");
                    return;
                  }
                  await invoke("pick_and_add_content", { kind: contentKind });
                  await loadContent(contentKind);
                }}
                onRemoveContent={async (entry) => {
                  if (!confirm(`${t("remove")} ${entry.name} ?`)) return;
                  if (contentKind === "datapack") {
                    await invoke("remove_datapack", { world: selectedWorld, name: entry.name });
                    await loadContent("datapack");
                    return;
                  }
                  await invoke("remove_content", { kind: contentKind, name: entry.name });
                  await loadContent(contentKind);
                }}
                onOpenContentFolder={() => {
                  if (contentKind === "datapack") {
                    void invoke("open_datapacks", { world: selectedWorld });
                    return;
                  }
                  void invoke("open_folder", { kind: contentKind === "shader" ? "shaders" : "resourcepacks" });
                }}
                onCreateBackup={async () => {
                  await invoke("make_backup");
                  await loadBackups();
                }}
                onRestore={async (backup) => {
                  if (!confirm(t("restoreConfirm"))) return;
                  await invoke("restore_worlds", { fileName: backup.fileName, confirmed: true });
                  await loadBackups();
                }}
                onOpenFolder={(kind) => void invoke("open_folder", { kind })}
              />
            )}

          </>
        )}
      </div>
    </div>
  );
}
