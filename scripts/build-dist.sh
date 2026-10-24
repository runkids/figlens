#!/bin/sh
# Cross-build release archives into dist/. Runs inside Dockerfile.dist's toolchain
# (scripts/dist.sh, locally and in .github/workflows/release.yaml). Upload dist/ to a GitHub release; scripts/install.sh reads it from there.
set -eu

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
targets=${FIGLENS_TARGETS:-"aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu"}
test -f skills/figlens/SKILL.md
test -f skills/figlens/references/planning.md
test -f skills/figlens/references/planning-details.md
test -f skills/figlens/references/yolo.md
mkdir -p dist
set --
for target in $targets; do
    cargo zigbuild --locked --release --target "$target" --target-dir target/dist-build
    release="target/dist-build/$target/release"
    case "$target" in
    *windows*)
        # Windows users expect a zip (Expand-Archive); the image has python3 but no zip.
        test -f "$release/figlens.exe"
        archive="figlens-$version-$target.zip"
        python3 - "dist/$archive" "$release/figlens.exe" <<'PY'
import pathlib, sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_DEFLATED) as z:
    z.write(sys.argv[2], "figlens.exe")
    for file in sorted(pathlib.Path("skills/figlens").rglob("*")):
        if file.is_file():
            z.write(file, file.as_posix())
PY
        ;;
    *)
        test -x "$release/figlens"
        archive="figlens-$version-$target.tar.gz"
        tar -czf "dist/$archive" -C "$release" figlens -C "$PWD" skills/figlens
        ;;
    esac
    set -- "$@" "$archive"
    # Free each target's output but keep the host build scripts and proc macros for the next.
    rm -rf "target/dist-build/$target"
done
cd dist
sha256sum "$@" > SHA256SUMS
sha256sum -c SHA256SUMS
ls -lh "$@" SHA256SUMS
