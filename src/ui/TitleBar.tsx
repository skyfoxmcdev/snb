import { IconClose, IconMaximize, IconMinimize } from "./Icons";

interface TitleBarProps {
  title: string;
  onMinimize: () => void;
  onMaximize: () => void;
  onClose: () => void;
}

export function TitleBar({ title, onMinimize, onMaximize, onClose }: TitleBarProps) {
  return (
    <header className="titlebar" data-tauri-drag-region>
      <span className="titlebar-name" data-tauri-drag-region>
        {title}
      </span>
      <div className="titlebar-actions">
        <button type="button" className="window-button" aria-label="Minimize" onClick={onMinimize}>
          <IconMinimize size={15} />
        </button>
        <button type="button" className="window-button" aria-label="Maximize" onClick={onMaximize}>
          <IconMaximize size={13} />
        </button>
        <button type="button" className="window-button window-button--close" aria-label="Close" onClick={onClose}>
          <IconClose size={15} />
        </button>
      </div>
    </header>
  );
}
