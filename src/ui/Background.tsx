interface BackgroundLayerProps {
  imageUrl?: string;
  overlay?: number;
  dimmed?: boolean;
}

/**
 * Full-bleed developer background. The artwork comes exclusively from
 * appearance.backgroundUrl (HTTPS or /branding/...); when it is absent the
 * launcher shows the theme gradient from styles.css.
 */
export function BackgroundLayer({ imageUrl, overlay = 0.55, dimmed = false }: BackgroundLayerProps) {
  return (
    <div className={`background${dimmed ? " background--dimmed" : ""}`} aria-hidden="true">
      {imageUrl && (
        <div className="background-media" style={{ backgroundImage: `url("${imageUrl}")` }} />
      )}
      <div className="background-scrim" style={{ opacity: Math.min(1, Math.max(0.3, overlay + 0.22)) }} />
      <div className="background-vignette" />
    </div>
  );
}
