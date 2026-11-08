import { IconArrowRight } from "./Icons";
import { BrandMark } from "./Logo";
import { InstallProgress } from "./InstallProgress";
import type { InstallReport } from "../types";

interface OnboardingProps {
  logoUrl?: string;
  name: string;
  dir: string;
  busy: boolean;
  installing: boolean;
  report: InstallReport | null;
  percent: number | null;
  error: string | null;
  t: (key: string) => string;
  onName: (value: string) => void;
  onDir: (value: string) => void;
  onBrowse: () => void;
  onBegin: () => void;
}

export function Onboarding({
  logoUrl,
  name,
  dir,
  busy,
  installing,
  report,
  percent,
  error,
  t,
  onName,
  onDir,
  onBrowse,
  onBegin,
}: OnboardingProps) {
  return (
    <main className="onboarding">
      <div className="onboarding-inner">
        <div className="onboarding-mark">
          <BrandMark logoUrl={logoUrl} name="Sky & Beyond" size={56} />
        </div>
        <p className="menu-eyebrow">Sky &amp; Beyond</p>
        <h1>{t("welcome")}</h1>
        <p className="onboarding-lede">{t("intro")}</p>

        <div className="onboarding-fields">
          <label className="field">
            <span>{t("username")}</span>
            <input
              value={name}
              maxLength={16}
              spellCheck={false}
              autoComplete="off"
              placeholder="Steve"
              onChange={(event) => onName(event.target.value)}
            />
          </label>
          <label className="field">
            <span>{t("gameDir")}</span>
            <div className="field-inline">
              <input value={dir} spellCheck={false} onChange={(event) => onDir(event.target.value)} />
              <button type="button" className="text-link" onClick={onBrowse}>
                {t("choose")}
              </button>
            </div>
          </label>
        </div>

        <button type="button" className="begin-button" disabled={busy || installing} onClick={onBegin}>
          <span>{installing ? t("installingShort") : busy ? t("loading") : t("begin")}</span>
          {!installing && <IconArrowRight size={18} />}
        </button>

        {error && <p className="inline-error">{error}</p>}
        {report && <InstallProgress report={report} t={t} percent={percent} />}
      </div>
    </main>
  );
}
