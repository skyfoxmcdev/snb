import { BrandMark } from "./Logo";
import type { UpdateCheck } from "../types";

interface UpdateGateProps {
  update: UpdateCheck;
  logoUrl?: string | null;
  updating: boolean;
  progress: { current: number; total: number } | null;
  error: string | null;
  t: (key: string) => string;
  onInstall: () => void;
  onRetry: () => void;
  onLogs: () => void;
}

export function UpdateGate({ update, logoUrl, updating, progress, error, t, onInstall, onRetry, onLogs }: UpdateGateProps) {
  const percent =
    progress && progress.total > 0 ? Math.min(100, Math.round((progress.current / progress.total) * 100)) : null;

  return (
    <div className="update-gate" role="dialog" aria-modal="true">
      <div className="update-inner">
        <BrandMark logoUrl={logoUrl} name="Sky & Beyond" size={44} />
        <p className="menu-eyebrow">{t("mandatoryUpdate")}</p>
        <h1>{t("updateTitle")}</h1>
        <p className="update-body">{update.decision ? t("updateBody") : t("updateBlocked")}</p>

        {update.decision && (
          <>
            <p className="update-version">
              {update.decision.currentVersion} <span aria-hidden="true">→</span> {update.decision.latestVersion}
            </p>
            {update.decision.notes && (
              <p className="update-notes">
                {typeof update.decision.notes === "string"
                  ? update.decision.notes
                  : update.decision.notes.en || update.decision.notes.fr}
              </p>
            )}
          </>
        )}

        {percent !== null && (
          <div className="install-progress install-progress--compact">
            <div className="install-head">
              <span>{t("updateProgress")}</span>
              <span className="install-percent">{percent}%</span>
            </div>
            <div className="install-track">
              <i style={{ width: `${percent}%` }} />
            </div>
          </div>
        )}

        {(error || update.error) && <p className="inline-error">{error || update.error}</p>}

        <div className="update-actions">
          {update.decision ? (
            <button type="button" className="solid-button" disabled={updating} onClick={onInstall}>
              {updating ? t("updateProgress") : t("installUpdate")}
            </button>
          ) : (
            <button type="button" className="solid-button" disabled={updating} onClick={onRetry}>
              {t("retryUpdate")}
            </button>
          )}
          <button type="button" className="text-link" onClick={onLogs}>
            {t("openLogs")}
          </button>
        </div>
      </div>
    </div>
  );
}
