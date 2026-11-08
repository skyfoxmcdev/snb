import type { ReactNode } from "react";
import { IconArchive, IconChat, IconGear, IconGlobe, IconLayers, IconLifeBuoy, IconPackage } from "./Icons";

interface IconRailProps {
  onSettings: () => void;
  onMods: () => void;
  onContent: () => void;
  onBackups: () => void;
  onOpenUrl: (url: string) => void;
  links: { website?: string; discord?: string; support?: string };
}

export function IconRail({ onSettings, onMods, onContent, onBackups, onOpenUrl, links }: IconRailProps) {
  const community = [
    links.website ? { id: "website", label: "Website", url: links.website, icon: <IconGlobe size={21} /> } : null,
    links.discord ? { id: "discord", label: "Discord", url: links.discord, icon: <IconChat size={21} /> } : null,
    links.support ? { id: "support", label: "Support", url: links.support, icon: <IconLifeBuoy size={21} /> } : null,
  ].filter(Boolean) as { id: string; label: string; url: string; icon: ReactNode }[];

  return (
    <nav className="icon-rail" aria-label="Quick navigation">
      <button type="button" className="rail-button" aria-label="Settings" title="Settings" onClick={onSettings}>
        <IconGear size={21} />
      </button>
      <button type="button" className="rail-button" aria-label="Mods" title="Mods" onClick={onMods}>
        <IconPackage size={21} />
      </button>
      <button type="button" className="rail-button" aria-label="Content" title="Content" onClick={onContent}>
        <IconLayers size={21} />
      </button>
      <button type="button" className="rail-button" aria-label="Backups" title="Backups" onClick={onBackups}>
        <IconArchive size={21} />
      </button>

      {community.map((item) => (
        <button
          type="button"
          className="rail-button"
          key={item.id}
          aria-label={item.label}
          title={item.label}
          onClick={() => onOpenUrl(item.url)}
        >
          {item.icon}
        </button>
      ))}
    </nav>
  );
}
