#!/bin/sh
# Install the latest figlens release from GitHub. Run:
#   curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh | sh
set -eu

bin_dir=${FIGLENS_BIN_DIR:-$HOME/.local/bin}
case "$(uname -s)" in
    Darwin) os=apple-darwin ;;
    Linux) os=unknown-linux-musl ;;
    *) echo "figlens: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    arm64 | aarch64) arch=aarch64 ;;
    x86_64 | amd64) arch=x86_64 ;;
    *) echo "figlens: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
repo=https://github.com/runkids/figlens
tag=$(curl -fsSL https://api.github.com/repos/runkids/figlens/releases/latest | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')
[ -n "$tag" ] || { echo "figlens: no release found at $repo/releases" >&2; exit 1; }
archive="figlens-${tag#v}-$arch-$os.tar.gz"
curl -fsSL -o "$tmp/$archive" "$repo/releases/download/$tag/$archive"
curl -fsSL -o "$tmp/SHA256SUMS" "$repo/releases/download/$tag/SHA256SUMS"
(cd "$tmp" && grep -- "  $archive$" SHA256SUMS | shasum -a 256 -c - >/dev/null)
tar -xzf "$tmp"/figlens-*.tar.gz -C "$tmp"
mkdir -p "$bin_dir"
# Rename over the old binaries: writing into a running one fails (Linux) or breaks its signature (macOS).
# figlens-embed is the optional local model runtime figlens looks for beside itself; releases
# before it shipped have none. Both are staged before either is replaced.
names=figlens
[ -f "$tmp/figlens-embed" ] && names="figlens-embed figlens"
for name in $names; do install -m 755 "$tmp/$name" "$bin_dir/.$name.new"; done
for name in $names; do mv -f "$bin_dir/.$name.new" "$bin_dir/$name"; done
echo "installed $("$bin_dir/figlens" --version) to $bin_dir"
case ":$PATH:" in *":$bin_dir:"*) ;; *) echo "add $bin_dir to PATH" >&2 ;; esac
