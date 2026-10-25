#!/usr/bin/env node
// `npx @runkids/figlens ...`: downloads this version's release binary on first use, checks it against
// the release's SHA256SUMS, caches it, then runs it. No install script: npx and pnpm may skip those.
const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { asset } = require("../target.js");
const { version } = require("../package.json");

const cacheRoot =
  process.env.XDG_CACHE_HOME ||
  (process.platform === "win32" ? process.env.LOCALAPPDATA : null) ||
  path.join(os.homedir(), ".cache");
const { archive, binary } = asset(process.platform, process.arch, version);
const dir = path.join(cacheRoot, "figlens", "npm", version);
const bin = path.join(dir, binary);

async function download(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return Buffer.from(await res.arrayBuffer());
}

async function install() {
  const base = `https://github.com/runkids/figlens/releases/download/v${version}`;
  const [bytes, sums] = await Promise.all([
    download(`${base}/${archive}`),
    download(`${base}/SHA256SUMS`),
  ]);
  const want = sums
    .toString()
    .split("\n")
    .find((line) => line.trim().endsWith(archive));
  const got = crypto.createHash("sha256").update(bytes).digest("hex");
  if (!want || want.split(/\s+/)[0] !== got) {
    throw new Error(`${archive} does not match the release's SHA256SUMS`);
  }
  // Extract next to the cache, then rename into place, so a half-written binary is never run.
  fs.mkdirSync(path.dirname(dir), { recursive: true });
  const tmp = fs.mkdtempSync(path.join(path.dirname(dir), ".tmp-"));
  try {
    fs.writeFileSync(path.join(tmp, archive), bytes);
    // Git Bash can put GNU tar ahead of Windows' ZIP-capable bsdtar on PATH.
    const extractor = process.platform === "win32"
      ? path.win32.join(process.env.SystemRoot || process.env.SYSTEMROOT || "C:\\Windows", "System32", "tar.exe")
      : "tar";
    const tar = spawnSync(extractor, ["-xf", archive, binary], { cwd: tmp, stdio: "inherit" });
    if (tar.status !== 0) throw new Error("tar could not extract the archive");
    fs.rmSync(path.join(tmp, archive));
    try {
      fs.renameSync(tmp, dir);
    } catch (e) {
      // Another first-use process may already have installed and started the MCP server.
      if (!fs.existsSync(bin)) throw e;
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  } catch (e) {
    fs.rmSync(tmp, { recursive: true, force: true });
    throw e;
  }
}

async function main() {
  if (!fs.existsSync(bin)) await install();
  const run = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
  if (run.error) throw run.error;
  process.exit(run.status ?? 1);
}

main().catch((e) => {
  console.error(`figlens: ${e.message}`);
  process.exit(1);
});
