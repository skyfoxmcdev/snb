import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const rustc = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const host = /^host:\s*(\S+)$/m.exec(rustc)?.[1];
if (!host) throw new Error("Could not determine the Rust host target.");
execFileSync("cargo", ["build", "--release", "-p", "snb-updater"], { stdio: "inherit" });
const suffix = host.includes("windows") ? ".exe" : "";
const source = join("target", "release", `snb-updater${suffix}`);
const destinationDir = join("src-tauri", "binaries");
mkdirSync(destinationDir, { recursive: true });
const destination = join(destinationDir, `snb-updater-${host}${suffix}`);
copyFileSync(source, destination);
console.log(`Sidecar ready: ${destination}`);
