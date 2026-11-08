import { useEffect, useState } from "react";

/** Two-letter monogram used only when no developer image is configured. */
export function initials(name?: string | null, fallback = "SB"): string {
  const words = (name || "")
    .trim()
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean);
  if (words.length === 0) return fallback;
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return `${words[0][0]}${words[1][0]}`.toUpperCase();
}

interface BrandMarkProps {
  logoUrl?: string | null;
  name?: string;
  size?: number;
  className?: string;
}

/**
 * Launcher mark. The image is supplied by the developer through
 * appearance.logoUrl (HTTPS or /branding/...); no artwork is bundled in code.
 */
export function BrandMark({
  logoUrl,
  name = "Sky & Beyond",
  size = 54,
  className,
}: BrandMarkProps) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [logoUrl]);

  if (logoUrl && !failed) {
    return (
      <img
        className={className}
        src={logoUrl}
        alt={name}
        width={size}
        height={size}
        style={{ width: size, height: size, objectFit: "cover", borderRadius: "50%" }}
        onError={() => setFailed(true)}
      />
    );
  }

  return (
    <span
      className={`brand-initials${className ? ` ${className}` : ""}`}
      style={{ width: size, height: size, fontSize: Math.max(10, Math.round(size * 0.3)) }}
      role="img"
      aria-label={name}
    >
      {initials(name)}
    </span>
  );
}

interface PlayerAvatarProps {
  username: string;
  imageUrl?: string | null;
  size?: number;
}

/**
 * Profile icon. appearance.playerIconUrl is developer-controlled; players
 * cannot upload or replace it from the launcher UI.
 */
export function PlayerAvatar({ username, imageUrl, size = 88 }: PlayerAvatarProps) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [imageUrl]);

  if (imageUrl && !failed) {
    return (
      <img
        className="player-avatar"
        src={imageUrl}
        alt=""
        width={size}
        height={size}
        style={{ width: "100%", height: "100%", objectFit: "cover" }}
        onError={() => setFailed(true)}
      />
    );
  }

  return (
    <span className="player-initials" style={{ fontSize: Math.max(13, Math.round(size * 0.32)) }} aria-hidden="true">
      {initials(username, "P")}
    </span>
  );
}
