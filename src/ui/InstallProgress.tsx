import type { InstallReport } from "../types";

interface InstallProgressProps {
  report: InstallReport;
  t: (key: string) => string;
  percent: number | null;
}

export function InstallProgress({ report, t, percent }: InstallProgressProps) {
  const current = report.steps.find((step) => step.id === report.currentStep);
  return (
    <section className="install-progress">
      <div className="install-head">
        <span>{report.ready ? t("ready") : report.error ? t("failed") : t("progress")}</span>
        {percent !== null && <span className="install-percent">{percent}%</span>}
      </div>
      <div className="install-track">
        <i
          className={percent === null ? "indeterminate" : ""}
          style={{ width: percent === null ? "18%" : `${Math.max(2, percent)}%` }}
        />
      </div>
      {current && (
        <p className="install-step">
          {t(`step_${current.id}`)} · {t(current.status)}
        </p>
      )}
      {report.error && (
        <p className="inline-error">
          {report.error.code}
          {report.error.detail ? ` · ${report.error.detail}` : ""}
        </p>
      )}
    </section>
  );
}
