import { IconClose, IconExternal } from "./Icons";
import { pickText } from "../i18n";
import type { Lang } from "../types";

interface NewsPanelProps {
  open: boolean;
  onClose: () => void;
  news: any[];
  announcements: any[];
  buttons: any[];
  lang: Lang;
  labels: { news: string; noNews: string; exportDiag: string; copyDiag: string; copied: string; openLogs: string };
  copied: boolean;
  onOpenUrl: (url: string) => void;
  onExportDiag: () => void;
  onCopySummary: () => void;
  onOpenLogs: () => void;
}

export function NewsPanel({
  open,
  onClose,
  news,
  announcements,
  buttons,
  lang,
  labels,
  copied,
  onOpenUrl,
  onExportDiag,
  onCopySummary,
  onOpenLogs,
}: NewsPanelProps) {
  return (
    <section className={`news-sheet${open ? " news-sheet--open" : ""}`} aria-hidden={!open} inert={!open}>
      <div className="news-sheet-inner">
        <header className="news-head">
          <h2>{labels.news}</h2>
          <button type="button" className="ghost-button" onClick={onClose} aria-label="Close news">
            <IconClose size={18} />
          </button>
        </header>

        <div className="news-scroll">
          {news.length === 0 && <p className="news-empty">{labels.noNews}</p>}
          {news.map((item, index) => (
            <article className="news-article" key={item.id || index}>
              {item.imageUrl && <img className="news-article-image" src={item.imageUrl} alt="" loading="lazy" />}
              <div className="news-article-copy">
                {item.publishedAt && (
                  <p className="news-meta">
                    {new Date(item.publishedAt).toLocaleDateString(lang === "fr" ? "fr-FR" : "en-GB", {
                      day: "numeric",
                      month: "long",
                      year: "numeric",
                    })}
                  </p>
                )}
                <h3>{pickText(item.title, lang)}</h3>
                {(item.summary || item.body) && (
                  <p>{pickText(item.summary || item.description || item.body, lang)}</p>
                )}
                {item.link && (
                  <button type="button" className="text-link" onClick={() => onOpenUrl(item.link)}>
                    <span>{lang === "fr" ? "Lire la suite" : "Read more"}</span>
                    <IconExternal size={14} />
                  </button>
                )}
              </div>
            </article>
          ))}

          {announcements.length > 0 && (
            <div className="news-announcements">
              {announcements.map((announcement, index) => (
                <div className="news-announcement" key={announcement.id || index}>
                  <span
                    className={`status-dot status-dot--${
                      ["warning", "maintenance", "important"].includes(announcement.level) ? "degraded" : "online"
                    }`}
                    aria-hidden="true"
                  />
                  <div>
                    <strong>{pickText(announcement.title, lang)}</strong>
                    {pickText(announcement.body, lang) && <p>{pickText(announcement.body, lang)}</p>}
                    {announcement.link && (
                      <button type="button" className="text-link" onClick={() => onOpenUrl(announcement.link)}>
                        <span>{lang === "fr" ? "En savoir plus" : "Learn more"}</span>
                        <IconExternal size={13} />
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}

          {buttons.length > 0 && (
            <div className="news-links">
              {buttons.map((button) => (
                <button
                  type="button"
                  className="text-link"
                  key={button.id || button.url}
                  onClick={() => onOpenUrl(button.url)}
                >
                  <span>{pickText(button.label, lang)}</span>
                  <IconExternal size={14} />
                </button>
              ))}
            </div>
          )}
        </div>

        <footer className="news-footer">
          <button type="button" className="text-link" onClick={onExportDiag}>
            {labels.exportDiag}
          </button>
          <button type="button" className="text-link" onClick={onCopySummary}>
            {copied ? labels.copied : labels.copyDiag}
          </button>
          <button type="button" className="text-link" onClick={onOpenLogs}>
            {labels.openLogs}
          </button>
        </footer>
      </div>
    </section>
  );
}
