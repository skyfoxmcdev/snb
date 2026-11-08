import { IconChevronUp } from "./Icons";
import type { MojangState } from "../types";

interface BottomBarProps {
  playersOnline?: number;
  playersMax?: number;
  mojang: MojangState;
  newsOpen: boolean;
  onNews: () => void;
  playLabel: string;
  playDisabled: boolean;
  onPlay: () => void;
  serverName: string;
  serverOnline: boolean;
  onServer: () => void;
  labels: { players: string; mojang: string; news: string };
}

const MOJANG_LABEL: Record<MojangState, string> = {
  operational: "Operational",
  degraded: "Degraded",
  unavailable: "Unavailable",
  unknown: "Unknown",
};

export function BottomBar({
  playersOnline,
  playersMax,
  mojang,
  newsOpen,
  onNews,
  playLabel,
  playDisabled,
  onPlay,
  serverName,
  serverOnline,
  onServer,
  labels,
}: BottomBarProps) {
  const players =
    playersOnline !== undefined && playersMax !== undefined
      ? `${playersOnline}/${playersMax}`
      : "—";

  return (
    <footer className="bottombar">
      <div className="bar-side bar-side--left">
        <div className="bar-stat">
          <span className="bar-label">{labels.players}</span>
          <span className="bar-value">{players}</span>
        </div>
        <span className="bar-divider" aria-hidden="true" />
        <div className="bar-stat">
          <span className="bar-label">{labels.mojang}</span>
          <span className={`status-dot status-dot--${mojang}`} title={MOJANG_LABEL[mojang]}>
            <span className="sr-only">{MOJANG_LABEL[mojang]}</span>
          </span>
        </div>
      </div>

      <button
        type="button"
        className={`news-trigger${newsOpen ? " news-trigger--open" : ""}`}
        onClick={onNews}
        aria-expanded={newsOpen}
      >
        <IconChevronUp size={20} />
        <span>{labels.news}</span>
      </button>

      <div className="bar-side bar-side--right">
        <button
          type="button"
          className="play-action"
          onClick={onPlay}
          disabled={playDisabled}
        >
          {playLabel}
        </button>
        <span className="bar-divider" aria-hidden="true" />
        <button type="button" className="server-trigger" onClick={onServer}>
          <span className={`status-dot status-dot--${serverOnline ? "online" : "offline"}`} aria-hidden="true" />
          <span>{serverName}</span>
        </button>
      </div>
    </footer>
  );
}
