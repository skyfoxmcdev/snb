import { IconClose } from "./Icons";
import { bytes } from "../i18n";
import type { Backup, ContentView, Lang, ModView } from "../types";

export type LibraryKind = "mods" | "content" | "backups";

interface LibraryPanelProps {
  kind: LibraryKind;
  lang: Lang;
  t: (key: string) => string;
  mods: ModView[];
  content: ContentView[];
  contentKind: string;
  worlds: string[];
  selectedWorld: string;
  backups: Backup[];
  onClose: () => void;
  onAddMods: () => void;
  onToggleMod: (mod: ModView) => void;
  onRemoveMod: (mod: ModView) => void;
  onContentKind: (kind: string) => void;
  onSelectWorld: (world: string) => void;
  onPickContent: () => void;
  onRemoveContent: (entry: ContentView) => void;
  onOpenContentFolder: () => void;
  onCreateBackup: () => void;
  onRestore: (backup: Backup) => void;
  onOpenFolder: (kind: string) => void;
}

export function LibraryPanel(props: LibraryPanelProps) {
  const {
    kind,
    lang,
    t,
    mods,
    content,
    contentKind,
    worlds,
    selectedWorld,
    backups,
    onClose,
    onAddMods,
    onToggleMod,
    onRemoveMod,
    onContentKind,
    onSelectWorld,
    onPickContent,
    onRemoveContent,
    onOpenContentFolder,
    onCreateBackup,
    onRestore,
    onOpenFolder,
  } = props;

  const title =
    kind === "mods"
      ? t("mods")
      : kind === "backups"
        ? t("backups")
        : contentKind === "shader"
          ? t("shaders")
          : contentKind === "datapack"
            ? t("datapacks")
            : t("resourcepacks");

  return (
    <div className="library-layer" role="dialog" aria-modal="true" aria-label={title}>
      <div className="library-scrim" onClick={onClose} />
      <section className="library-panel">
        <header className="library-head">
          <div>
            <p className="menu-eyebrow">{t("library")}</p>
            <h2>{title}</h2>
          </div>
          <button type="button" className="ghost-button" onClick={onClose} aria-label={t("close")}>
            <IconClose size={18} />
          </button>
        </header>

        {kind === "mods" && (
          <>
            <div className="library-actions">
              <button type="button" className="solid-button" onClick={onAddMods}>
                {t("addMods")}
              </button>
              <button type="button" className="text-link" onClick={() => onOpenFolder("mods")}>
                {t("openFolder")}
              </button>
            </div>
            <p className="library-hint">{t("modsHint")}</p>
            <div className="library-list">
              {mods.length === 0 && <p className="library-empty">{t("noMods")}</p>}
              {mods.map((mod) => (
                <div className="library-row" key={`${mod.fileName}-${mod.enabled}`}>
                  <div className="row-identity">
                    {mod.icon && <img src={mod.icon} alt="" />}
                    <div>
                      <strong>{mod.name || mod.fileName}</strong>
                      <p>
                        {mod.authors ? `${mod.authors} · ` : ""}
                        {mod.version ? `v${mod.version} · ` : ""}
                        {bytes(mod.size)}
                        {mod.managed ? ` · ${t("managed")}` : ""}
                      </p>
                      {mod.description && <p className="row-description">{mod.description}</p>}
                    </div>
                  </div>
                  <div className="row-actions">
                    {!mod.managed && (
                      <>
                        <button type="button" className="text-link" onClick={() => onToggleMod(mod)}>
                          {mod.enabled ? t("disable") : t("enable")}
                        </button>
                        <button type="button" className="text-link text-link--danger" onClick={() => onRemoveMod(mod)}>
                          {t("remove")}
                        </button>
                      </>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {kind === "content" && (
          <>
            <div className="tab-row">
              {[
                { id: "shader", label: t("shaders") },
                { id: "resourcepack", label: t("resourcepacks") },
                { id: "datapack", label: t("datapacks") },
              ].map((tab) => (
                <button
                  type="button"
                  key={tab.id}
                  className={`tab${contentKind === tab.id ? " tab--active" : ""}`}
                  onClick={() => onContentKind(tab.id)}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            {contentKind === "datapack" &&
              (worlds.length === 0 ? (
                <p className="library-empty">{t("noWorlds")}</p>
              ) : (
                <label className="field field--inline">
                  <span>{t("worlds")}</span>
                  <select value={selectedWorld} onChange={(event) => onSelectWorld(event.target.value)}>
                    {worlds.map((world) => (
                      <option key={world} value={world}>
                        {world}
                      </option>
                    ))}
                  </select>
                </label>
              ))}

            <div className="library-actions">
              <button
                type="button"
                className="solid-button"
                disabled={contentKind === "datapack" && !selectedWorld}
                onClick={onPickContent}
              >
                {contentKind === "datapack" ? t("addDatapack") : t("add")}
              </button>
              <button
                type="button"
                className="text-link"
                disabled={contentKind === "datapack" && !selectedWorld}
                onClick={onOpenContentFolder}
              >
                {t("openFolder")}
              </button>
            </div>

            <div className="library-list">
              {content.length === 0 && <p className="library-empty">{t("noMods")}</p>}
              {content.map((entry) => (
                <div className="library-row" key={entry.name}>
                  <div>
                    <strong>{entry.name}</strong>
                    <p>{entry.directory ? "folder" : bytes(entry.size)}</p>
                  </div>
                  <button type="button" className="text-link text-link--danger" onClick={() => onRemoveContent(entry)}>
                    {t("remove")}
                  </button>
                </div>
              ))}
            </div>
          </>
        )}

        {kind === "backups" && (
          <>
            <div className="library-actions">
              <button type="button" className="solid-button" onClick={onCreateBackup}>
                {t("createBackup")}
              </button>
              <button type="button" className="text-link" onClick={() => onOpenFolder("backups")}>
                {t("openFolder")}
              </button>
            </div>
            <div className="library-list">
              {backups.length === 0 && <p className="library-empty">{t("noBackups")}</p>}
              {backups.map((backup) => (
                <div className="library-row" key={backup.fileName}>
                  <div>
                    <strong>{backup.fileName}</strong>
                    <p>
                      {new Date(backup.createdAtMs).toLocaleString(lang === "fr" ? "fr-FR" : "en-GB")} ·{" "}
                      {bytes(backup.size)}
                    </p>
                  </div>
                  <button type="button" className="text-link" onClick={() => onRestore(backup)}>
                    {t("restore")}
                  </button>
                </div>
              ))}
            </div>
          </>
        )}
      </section>
    </div>
  );
}
