// The release archive for a Node platform and arch (see scripts/build-dist.sh).
const OS = { darwin: "apple-darwin", linux: "unknown-linux-musl", win32: "pc-windows-gnu" };
const ARCH = { arm64: "aarch64", x64: "x86_64" };

function asset(platform, arch, version) {
  // Windows on ARM runs the x64 build under emulation; there is no aarch64 Windows build.
  const cpu = platform === "win32" ? "x64" : arch;
  if (!OS[platform] || !ARCH[cpu]) {
    throw new Error(`figlens has no build for ${platform}/${arch}`);
  }
  const windows = platform === "win32";
  return {
    archive: `figlens-${version}-${ARCH[cpu]}-${OS[platform]}.${windows ? "zip" : "tar.gz"}`,
    binary: windows ? "figlens.exe" : "figlens",
  };
}

module.exports = { asset };
