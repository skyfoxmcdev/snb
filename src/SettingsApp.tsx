import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { BrandMark } from "./ui/Logo";
import { brandingDefaults } from "./branding";
import { IconClose } from "./ui/Icons";
import "./styles.css";

interface Settings {
  username: string; gameDir: string; javaPath: string; language: string; theme: string;
  minMemoryMb: number; maxMemoryMb: number; width: number; height: number; fullscreen: boolean;
  jvmArgs: string; gameArgs: string; autoInstallJava: boolean; autoBackup: boolean;
  backupIntervalHours: number; backupRetention: number; lastBackupMs: number; onboardingComplete: boolean;
}
interface Bootstrap {
  settings: Settings;
  launcherVersion: string;
  runtime: { javaMajor: number; minecraft: { version: string }; neoforge: { version: string } };
  remote?: { appearance?: { logoUrl?: string | null } };
}
interface InstallReport { ready: boolean; error?: { code: string; detail?: string }; currentStep?: string; steps: { id: string; status: string }[]; running: boolean }

const copy = {
  fr: {
    title: "Paramètres", navProfile: "Profil", navLauncher: "Launcher", navJava: "Java",
    navPerformance: "Performance", navDisplay: "Affichage", navBackups: "Sauvegardes",
    navFolders: "Dossiers", navAbout: "À propos",
    profileLede: "Votre identité de jeu et l'apparence du launcher.",
    account: "Profil", username: "Pseudo Minecraft", java: "Java", javaPath: "Chemin de Java 21",
    autoJava: "Télécharger Java 21 automatiquement s'il manque", memory: "Mémoire",
    min: "RAM minimale (Mo)", max: "RAM maximale (Mo)", display: "Affichage Minecraft",
    resolution: "Résolution", fullscreen: "Plein écran", args: "Arguments de lancement",
    jvm: "Arguments JVM supplémentaires", game: "Arguments Minecraft supplémentaires", install: "Installation",
    gameDir: "Dossier du jeu", repair: "Réparer l'installation", launcher: "Launcher", language: "Langue", theme: "Thème",
    backups: "Sauvegardes", autoBackup: "Sauvegarde automatique", interval: "Intervalle (heures)",
    retention: "Nombre de sauvegardes conservées", folders: "Dossiers", gameFolder: "Ouvrir le dossier du jeu",
    modsFolder: "Gérer le dossier des mods", logs: "Ouvrir les logs", shadersFolder: "Dossier des shaders",
    packsFolder: "Dossier des packs de ressources", worldsFolder: "Dossier des mondes",
    save: "Enregistrer", saved: "Préférences enregistrées.", close: "Fermer", choose: "Parcourir",
    usernameConfirm: "Changer de pseudo redémarre le profil Minecraft hors ligne. Confirmer ?",
    javaWarn: "Les arguments personnalisés peuvent empêcher le jeu de démarrer. Ne modifiez que si vous savez ce que vous faites.",
    repairProgress: "Réparation en cours", repairDone: "Réparation terminée. Redémarrez le launcher si nécessaire.",
    dark: "Sombre", light: "Clair", system: "Système", french: "Français", english: "English",
    ledeLauncher: "Langue, thème et dossier d'installation.",
    ledeJava: "Le runtime Java 21 utilisé pour lancer Minecraft.",
    ledePerformance: "Mémoire allouée et arguments de la JVM.",
    ledeDisplay: "Fenêtre du jeu et arguments Minecraft.",
    ledeBackups: "Sauvegardes automatiques des mondes.",
    ledeFolders: "Accès direct aux dossiers de l'instance.",
    ledeAbout: "Versions installées et réparation.",
    offlineProfile: "Profil hors ligne. Aucun compte Microsoft n'est utilisé.",
  },
  en: {
    title: "Settings", navProfile: "Profile", navLauncher: "Launcher", navJava: "Java",
    navPerformance: "Performance", navDisplay: "Display", navBackups: "Backups",
    navFolders: "Folders", navAbout: "About",
    profileLede: "Your in-game identity and how the launcher looks.",
    account: "Profile", username: "Minecraft username", java: "Java", javaPath: "Java 21 path",
    autoJava: "Automatically download Java 21 if missing", memory: "Memory", min: "Minimum RAM (MB)",
    max: "Maximum RAM (MB)", display: "Minecraft display", resolution: "Resolution", fullscreen: "Fullscreen",
    args: "Launch arguments", jvm: "Extra JVM arguments", game: "Extra Minecraft arguments", install: "Installation",
    gameDir: "Game directory", repair: "Repair installation", launcher: "Launcher", language: "Language", theme: "Theme",
    backups: "Backups", autoBackup: "Automatic backups", interval: "Interval (hours)",
    retention: "Number of backups to keep", folders: "Folders", gameFolder: "Open game folder",
    modsFolder: "Open mods folder", logs: "Open logs", shadersFolder: "Shaders folder",
    packsFolder: "Resource packs folder", worldsFolder: "Worlds folder",
    save: "Save", saved: "Preferences saved.", close: "Close", choose: "Browse",
    usernameConfirm: "Changing your username restarts the offline Minecraft profile. Continue?",
    javaWarn: "Custom arguments can prevent the game from starting. Only change these if you know what you are doing.",
    repairProgress: "Repair in progress", repairDone: "Repair finished. Restart the launcher if needed.",
    dark: "Dark", light: "Light", system: "System", french: "Français", english: "English",
    ledeLauncher: "Language, theme and installation folder.",
    ledeJava: "The Java 21 runtime used to start Minecraft.",
    ledePerformance: "Allocated memory and JVM arguments.",
    ledeDisplay: "Game window and Minecraft arguments.",
    ledeBackups: "Automatic world backups.",
    ledeFolders: "Direct access to the instance folders.",
    ledeAbout: "Installed versions and repair.",
    offlineProfile: "Offline profile. No Microsoft account is used.",
  },
};

type Section = "profile" | "launcher" | "java" | "performance" | "display" | "backups" | "folders" | "about";

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (value: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="switch"
      onClick={() => onChange(!checked)}
    />
  );
}

export default function SettingsApp() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [repair, setRepair] = useState<InstallReport | null>(null);
  const [section, setSection] = useState<Section>("profile");

  const lang = draft?.language === "fr" ? "fr" : draft?.language === "en" ? "en" : navigator.language.startsWith("fr") ? "fr" : "en";
  const t = (key: keyof typeof copy.en) => copy[lang][key] || copy.en[key];

  useEffect(() => {
    invoke<Bootstrap>("bootstrap")
      .then((payload) => {
        setBoot(payload);
        setDraft(payload.settings);
      })
      .catch((cause) => setError(String(cause)));
    const un = listen<InstallReport>("install-progress", (event) => setRepair(event.payload));
    return () => {
      void un.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (!draft) return;
    const theme =
      draft.theme === "system"
        ? window.matchMedia?.("(prefers-color-scheme: light)").matches
          ? "light"
          : "dark"
        : draft.theme;
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = lang;
  }, [draft?.theme, lang]);

  const patch = (values: Partial<Settings>) => setDraft((current) => (current ? { ...current, ...values } : current));

  const save = async () => {
    if (!draft) return;
    setError("");
    try {
      if (boot && draft.username !== boot.settings.username && !confirm(t("usernameConfirm"))) return;
      const saved = await invoke<Settings>("save_settings", { settings: draft });
      setDraft(saved);
      setBoot((current) => (current ? { ...current, settings: saved } : current));
      setStatus(t("saved"));
      window.setTimeout(() => setStatus(""), 2500);
    } catch (cause: any) {
      setError(cause?.detail || cause?.code || String(cause));
    }
  };

  const runRepair = async () => {
    setError("");
    setRepair(null);
    try {
      const report = await invoke<InstallReport>("install_game", { repair: true });
      setRepair(report);
      if (report.error) setError(report.error.code + (report.error.detail ? ` · ${report.error.detail}` : ""));
    } catch (cause: any) {
      setError(cause?.detail || cause?.code || String(cause));
    }
  };

  const sections: { id: Section; label: string }[] = [
    { id: "profile", label: t("navProfile") },
    { id: "launcher", label: t("navLauncher") },
    { id: "java", label: t("navJava") },
    { id: "performance", label: t("navPerformance") },
    { id: "display", label: t("navDisplay") },
    { id: "backups", label: t("navBackups") },
    { id: "folders", label: t("navFolders") },
    { id: "about", label: t("navAbout") },
  ];

  if (!draft) {
    return (
      <main className="settings-root">
        <div className="settings-topbar" data-tauri-drag-region>
          <div className="settings-identity" data-tauri-drag-region>
            <BrandMark logoUrl={boot?.remote?.appearance?.logoUrl || brandingDefaults.logoUrl} size={26} />
            <h1 data-tauri-drag-region>{t("title")}</h1>
          </div>
          <button type="button" className="ghost-button" onClick={() => void getCurrentWindow().close()} aria-label="Close">
            <IconClose size={16} />
          </button>
        </div>
        <p className="settings-note" style={{ padding: 24 }}>{error || "Loading…"}</p>
      </main>
    );
  }

  return (
    <main className="settings-root">
      <div className="settings-topbar" data-tauri-drag-region>
        <div className="settings-identity" data-tauri-drag-region>
          <BrandMark logoUrl={boot?.remote?.appearance?.logoUrl || brandingDefaults.logoUrl} size={26} />
          <h1 data-tauri-drag-region>{t("title")}</h1>
        </div>
        <button type="button" className="ghost-button" onClick={() => void getCurrentWindow().close()} aria-label={t("close")}>
          <IconClose size={16} />
        </button>
      </div>

      <div className="settings-body">
        <nav className="settings-nav" aria-label={t("title")}>
          {sections.map((item) => (
            <button
              type="button"
              key={item.id}
              aria-current={section === item.id}
              onClick={() => setSection(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>

        <div className="settings-content">
          {section === "profile" && (
            <section className="settings-section">
              <h2>{t("navProfile")}</h2>
              <p className="settings-lede">{t("profileLede")}</p>

              <div className="settings-row">
                <div className="settings-row-label">
                  <strong>{t("username")}</strong>
                  <span>{t("offlineProfile")}</span>
                </div>
                <div className="settings-control">
                  <input
                    value={draft.username}
                    maxLength={16}
                    spellCheck={false}
                    onChange={(event) => patch({ username: event.target.value })}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("language")}</strong></div>
                <div className="settings-control">
                  <select value={draft.language} onChange={(event) => patch({ language: event.target.value })}>
                    <option value="">{t("system")}</option>
                    <option value="fr">{t("french")}</option>
                    <option value="en">{t("english")}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("theme")}</strong></div>
                <div className="settings-control">
                  <select value={draft.theme} onChange={(event) => patch({ theme: event.target.value })}>
                    <option value="system">{t("system")}</option>
                    <option value="dark">{t("dark")}</option>
                    <option value="light">{t("light")}</option>
                  </select>
                </div>
              </div>
            </section>
          )}

          {section === "launcher" && (
            <section className="settings-section">
              <h2>{t("navLauncher")}</h2>
              <p className="settings-lede">{t("ledeLauncher")}</p>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("gameDir")}</strong></div>
                <div className="settings-control">
                  <input value={draft.gameDir} spellCheck={false} onChange={(event) => patch({ gameDir: event.target.value })} />
                  <button
                    type="button"
                    className="text-link"
                    onClick={async () => {
                      const path = await invoke<string | null>("pick_directory");
                      if (path) patch({ gameDir: path });
                    }}
                  >
                    {t("choose")}
                  </button>
                </div>
              </div>
            </section>
          )}

          {section === "java" && (
            <section className="settings-section">
              <h2>{t("navJava")}</h2>
              <p className="settings-lede">{t("ledeJava")}</p>

              <div className="settings-row">
                <div className="settings-row-label">
                  <strong>{t("javaPath")}</strong>
                  <span>Java {boot?.runtime.javaMajor ?? 21}</span>
                </div>
                <div className="settings-control">
                  <input
                    value={draft.javaPath}
                    spellCheck={false}
                    placeholder={`auto (Java ${boot?.runtime.javaMajor ?? 21})`}
                    onChange={(event) => patch({ javaPath: event.target.value })}
                  />
                  <button
                    type="button"
                    className="text-link"
                    onClick={async () => {
                      const path = await invoke<string | null>("pick_java_path");
                      if (path) patch({ javaPath: path });
                    }}
                  >
                    {t("choose")}
                  </button>
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("autoJava")}</strong></div>
                <div className="settings-control">
                  <Toggle
                    checked={draft.autoInstallJava}
                    label={t("autoJava")}
                    onChange={(value) => patch({ autoInstallJava: value })}
                  />
                </div>
              </div>
            </section>
          )}

          {section === "performance" && (
            <section className="settings-section">
              <h2>{t("navPerformance")}</h2>
              <p className="settings-lede">{t("ledePerformance")}</p>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("min")}</strong></div>
                <div className="settings-control">
                  <input
                    type="number"
                    min={512}
                    max={32768}
                    value={draft.minMemoryMb}
                    onChange={(event) => patch({ minMemoryMb: Number(event.target.value) })}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("max")}</strong></div>
                <div className="settings-control">
                  <input
                    type="number"
                    min={1024}
                    max={32768}
                    value={draft.maxMemoryMb}
                    onChange={(event) => patch({ maxMemoryMb: Number(event.target.value) })}
                  />
                </div>
              </div>

              <div className="settings-row" style={{ display: "block" }}>
                <div className="settings-row-label" style={{ marginBottom: 10 }}>
                  <strong>{t("jvm")}</strong>
                  <span>{t("javaWarn")}</span>
                </div>
                <textarea
                  className="settings-textarea"
                  value={draft.jvmArgs}
                  spellCheck={false}
                  onChange={(event) => patch({ jvmArgs: event.target.value })}
                />
              </div>
            </section>
          )}

          {section === "display" && (
            <section className="settings-section">
              <h2>{t("navDisplay")}</h2>
              <p className="settings-lede">{t("ledeDisplay")}</p>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("resolution")}</strong></div>
                <div className="settings-control">
                  <div className="resolution">
                    <input type="number" value={draft.width} onChange={(event) => patch({ width: Number(event.target.value) })} />
                    <span>×</span>
                    <input type="number" value={draft.height} onChange={(event) => patch({ height: Number(event.target.value) })} />
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("fullscreen")}</strong></div>
                <div className="settings-control">
                  <Toggle checked={draft.fullscreen} label={t("fullscreen")} onChange={(value) => patch({ fullscreen: value })} />
                </div>
              </div>

              <div className="settings-row" style={{ display: "block" }}>
                <div className="settings-row-label" style={{ marginBottom: 10 }}>
                  <strong>{t("game")}</strong>
                </div>
                <textarea
                  className="settings-textarea"
                  value={draft.gameArgs}
                  spellCheck={false}
                  onChange={(event) => patch({ gameArgs: event.target.value })}
                />
              </div>
            </section>
          )}

          {section === "backups" && (
            <section className="settings-section">
              <h2>{t("navBackups")}</h2>
              <p className="settings-lede">{t("ledeBackups")}</p>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("autoBackup")}</strong></div>
                <div className="settings-control">
                  <Toggle checked={draft.autoBackup} label={t("autoBackup")} onChange={(value) => patch({ autoBackup: value })} />
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("interval")}</strong></div>
                <div className="settings-control">
                  <input
                    type="number"
                    min={1}
                    max={720}
                    value={draft.backupIntervalHours}
                    onChange={(event) => patch({ backupIntervalHours: Number(event.target.value) })}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="settings-row-label"><strong>{t("retention")}</strong></div>
                <div className="settings-control">
                  <input
                    type="number"
                    min={1}
                    max={50}
                    value={draft.backupRetention}
                    onChange={(event) => patch({ backupRetention: Number(event.target.value) })}
                  />
                </div>
              </div>
            </section>
          )}

          {section === "folders" && (
            <section className="settings-section">
              <h2>{t("navFolders")}</h2>
              <p className="settings-lede">{t("ledeFolders")}</p>
              <div className="settings-folder-grid">
                <button type="button" onClick={() => void invoke("open_folder", { kind: "game" })}>{t("gameFolder")}</button>
                <button type="button" onClick={() => void invoke("open_folder", { kind: "mods" })}>{t("modsFolder")}</button>
                <button type="button" onClick={() => void invoke("open_folder", { kind: "shaders" })}>{t("shadersFolder")}</button>
                <button type="button" onClick={() => void invoke("open_folder", { kind: "resourcepacks" })}>{t("packsFolder")}</button>
                <button type="button" onClick={() => void invoke("open_folder", { kind: "saves" })}>{t("worldsFolder")}</button>
                <button type="button" onClick={() => void invoke("open_folder", { kind: "logs" })}>{t("logs")}</button>
              </div>
            </section>
          )}

          {section === "about" && (
            <section className="settings-section">
              <h2>{t("navAbout")}</h2>
              <p className="settings-lede">{t("ledeAbout")}</p>

              <div className="settings-row">
                <div className="settings-row-label">
                  <strong>{t("repair")}</strong>
                  {repair && (
                    <span>
                      {repair.ready
                        ? t("repairDone")
                        : `${t("repairProgress")}: ${repair.currentStep || ""} ${
                            repair.steps.find((step) => step.id === repair.currentStep)?.status || ""
                          }`}
                    </span>
                  )}
                </div>
                <div className="settings-control">
                  <button type="button" className="text-link" onClick={() => void runRepair()}>
                    {t("repair")}
                  </button>
                </div>
              </div>

              <p className="settings-version">
                Sky &amp; Beyond Launcher v{boot?.launcherVersion} · Minecraft {boot?.runtime.minecraft.version} · NeoForge{" "}
                {boot?.runtime.neoforge.version} · Java {boot?.runtime.javaMajor}
              </p>
            </section>
          )}
        </div>
      </div>

      <footer className="settings-footer">
        <span className="settings-status">{status || error}</span>
        <div className="settings-buttons">
          <button type="button" className="text-link" onClick={() => void getCurrentWindow().close()}>
            {t("close")}
          </button>
          <button type="button" className="solid-button" onClick={() => void save()}>
            {t("save")}
          </button>
        </div>
      </footer>
    </main>
  );
}
