import { IconRefresh } from "./Icons";
import { pickText } from "../i18n";
import type { Lang, Localized, ServerStatus } from "../types";

interface ServerPopoverProps {
  open: boolean;
  onClose: () => void;
  name: string;
  description: string;
  status: ServerStatus;
  info: { label: Localized; value: Localized }[];
  maintenance?: string;
  lang: Lang;
  labels: { serverDetails: string; players: string; latency: string; refresh: string; status: string };
  onRefresh: () => void;
}

export function ServerPopover({
  open,
  onClose,
  name,
  description,
  status,
  info,
  maintenance,
  lang,
  labels,
  onRefresh,
}: ServerPopoverProps) {
  if (!open) return null;
  const players =
    status.playersOnline !== undefined && status.playersMax !== undefined
      ? `${status.playersOnline}/${status.playersMax}`
      : "—";

  return (
    <>
      <div className="menu-catcher" onClick={onClose} />
      <aside className="server-popover">
        <p className="menu-eyebrow">{labels.serverDetails}</p>
        <h3>{name}</h3>
        {description && <p className="server-description">{description}</p>}
        {maintenance && <p className="server-maintenance">{maintenance}</p>}

        <dl className="server-stats">
          <div>
            <dt>{labels.players}</dt>
            <dd>{players}</dd>
          </div>
          <div>
            <dt>{labels.status}</dt>
            <dd className="server-state">
              <span className={`status-dot status-dot--${status.state === "online" ? "online" : "offline"}`} />
              {status.state}
            </dd>
          </div>
          {status.latencyMs !== undefined && (
            <div>
              <dt>{labels.latency}</dt>
              <dd>{status.latencyMs} ms</dd>
            </div>
          )}
          {info.map((row, index) => (
            <div key={index}>
              <dt>{pickText(row.label, lang)}</dt>
              <dd>{pickText(row.value, lang)}</dd>
            </div>
          ))}
        </dl>

        <button type="button" className="text-link" onClick={onRefresh}>
          <IconRefresh size={15} />
          <span>{labels.refresh}</span>
        </button>
      </aside>
    </>
  );
}
