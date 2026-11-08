import { PlayerAvatar } from "./Logo";
import { IconArchive, IconFolder, IconGear, IconPackage } from "./Icons";

interface AccountSectionProps {
  username: string;
  avatarUrl?: string | null;
  open: boolean;
  version: string;
  onToggle: () => void;
  onClose: () => void;
  onSettings: () => void;
  onFolder: () => void;
  onBackups: () => void;
  onMods: () => void;
  labels: {
    account: string;
    settings: string;
    folder: string;
    backups: string;
    mods: string;
  };
}

export function AccountSection({
  username,
  avatarUrl,
  open,
  version,
  onToggle,
  onClose,
  onSettings,
  onFolder,
  onBackups,
  onMods,
  labels,
}: AccountSectionProps) {
  return (
    <div className="account">
      <button
        type="button"
        className="account-trigger"
        onClick={onToggle}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span className="account-name">{username || "Player"}</span>
        <span className="account-avatar">
          <PlayerAvatar username={username} imageUrl={avatarUrl} size={88} />
        </span>
      </button>

      {open && (
        <>
          <div className="menu-catcher" onClick={onClose} />
          <div className="account-menu" role="menu">
            <p className="menu-eyebrow">{labels.account}</p>
            <p className="menu-name">{username || "Player"}</p>
            <div className="menu-rule" />
            <button type="button" role="menuitem" onClick={onSettings}>
              <IconGear size={16} />
              <span>{labels.settings}</span>
            </button>
            <button type="button" role="menuitem" onClick={onMods}>
              <IconPackage size={16} />
              <span>{labels.mods}</span>
            </button>
            <button type="button" role="menuitem" onClick={onBackups}>
              <IconArchive size={16} />
              <span>{labels.backups}</span>
            </button>
            <button type="button" role="menuitem" onClick={onFolder}>
              <IconFolder size={16} />
              <span>{labels.folder}</span>
            </button>
            <div className="menu-rule" />
            <p className="menu-version">Launcher {version}</p>
          </div>
        </>
      )}
    </div>
  );
}
