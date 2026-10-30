const test = require("node:test");
const assert = require("node:assert");
const path = require("node:path");
const { asset, cachedBinary } = require("./target.js");

test("maps each released platform to its archive and binary", () => {
  assert.deepStrictEqual(
    [asset("darwin", "arm64", "1.2.3"), asset("win32", "arm64", "1.2.3")],
    [
      { archive: "figlens-1.2.3-aarch64-apple-darwin.tar.gz", binary: "figlens", companion: "figlens-embed" },
      { archive: "figlens-1.2.3-x86_64-pc-windows-gnu.zip", binary: "figlens.exe", companion: "figlens-embed.exe" },
    ]
  );
});

test("rejects a platform with no build", () => {
  assert.throws(() => asset("freebsd", "x64", "1.2.3"), /no build for freebsd\/x64/);
});

test("names the cached binary the launcher installs for a version", () => {
  assert.strictEqual(cachedBinary("/c", "win32", "arm64", "1.2.3"), path.join("/c", "npm", "1.2.3", "figlens.exe"));
});
