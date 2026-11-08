import { createHash } from "node:crypto";
import { readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, resolve } from "node:path";

const [, , installerArg, version, url, channel = "stable"] = process.argv;
if (!installerArg || !version || !url) {
  console.error("Usage: node scripts/prepare-release.mjs <installer.exe> <version> <https-url> [stable|beta]");
  process.exit(2);
}
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("Version must be valid semver.");
if (!/^https:\/\//.test(url)) throw new Error("Artifact URL must use HTTPS.");
const installer = resolve(installerArg);
const bytes = readFileSync(installer);
const sha256 = createHash("sha256").update(bytes).digest("hex");
const manifestPath = resolve(
  channel === "beta"
    ? "repository/launcher/update-manifest-beta.json"
    : "repository/launcher/update-manifest.json",
);
let manifest;
try {
  manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
} catch {
  manifest = {
    schemaVersion: 1,
    version,
    minimumVersion: version,
    mandatory: true,
    releasedAt: new Date().toISOString(),
    notes: { en: "Release notes", fr: "Notes de version" },
    platforms: {},
  };
}
manifest.version = version;
manifest.minimumVersion = manifest.minimumVersion || version;
manifest.mandatory = true;
manifest.releasedAt = new Date().toISOString();
manifest.platforms = manifest.platforms || {};
manifest.platforms["windows-x86_64"] = {
  url,
  sha256,
  size: statSync(installer).size,
  signature: null,
  fileName: basename(installer),
};
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Updated ${manifestPath}`);
console.log(`version=${version}`);
console.log(`sha256=${sha256}`);
console.log(`size=${statSync(installer).size}`);
console.log("Add an Ed25519 signature manually if a public key is configured.");
