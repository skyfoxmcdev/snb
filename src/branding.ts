import appearance from "../repository/config/appearance.json";

function safeAsset(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const candidate = value.trim();
  if (!candidate) return undefined;
  if (candidate.startsWith("/")) {
    const safe =
      candidate.length > 1 &&
      !candidate.startsWith("//") &&
      !candidate.includes("..") &&
      !candidate.includes("\\") &&
      /^[A-Za-z0-9._/-]+$/.test(candidate);
    return safe ? candidate : undefined;
  }
  try {
    const url = new URL(candidate);
    return url.protocol === "https:" ? url.toString() : undefined;
  } catch {
    return undefined;
  }
}

const source = appearance as {
  logoUrl?: unknown;
  backgroundUrl?: unknown;
  playerIconUrl?: unknown;
};

/**
 * Build-time defaults from repository/config/appearance.json. They let the
 * developer ship /branding/... files without waiting for the remote config,
 * while a valid remote appearance still takes precedence in App.
 */
export const brandingDefaults = {
  logoUrl: safeAsset(source.logoUrl),
  backgroundUrl: safeAsset(source.backgroundUrl),
  playerIconUrl: safeAsset(source.playerIconUrl),
};
