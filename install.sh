#!/bin/sh
set -eu

repository="andrewkoumoudjian/bunting"
install_dir="${BUNTING_INSTALL_DIR:-$HOME/.local/bin}"
config_dir="${BUNTING_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/bunting/server}"
share_dir="$(dirname "$install_dir")/share/bunting"
version="${BUNTING_VERSION:-latest}"

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target="aarch64-apple-darwin" ;;
  Darwin:x86_64) target="x86_64-apple-darwin" ;;
  Linux:x86_64) target="x86_64-unknown-linux-gnu" ;;
  *)
    echo "Unsupported platform $(uname -s) $(uname -m). Download a release archive manually." >&2
    exit 1
    ;;
esac

if [ "$version" = "latest" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/$repository/releases/latest" |
    sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' |
    head -n 1)
fi

case "$version" in
  v*) ;;
  *) version="v$version" ;;
esac

archive="bunting-${version}-${target}.tar.gz"
wasi_archive="bunting-${version}-wasi.tar.gz"
base_url="${BUNTING_DOWNLOAD_BASE:-https://github.com/$repository/releases/download/$version}"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

curl -fsSL "$base_url/$archive" -o "$temporary/$archive"
curl -fsSL "$base_url/$wasi_archive" -o "$temporary/$wasi_archive"
curl -fsSL "$base_url/SHA256SUMS" -o "$temporary/SHA256SUMS"
for checked_archive in "$archive" "$wasi_archive"; do
  expected=$(awk -v name="$checked_archive" '$2 == name {print $1}' "$temporary/SHA256SUMS")
  if [ -z "$expected" ]; then
    echo "Release checksum for $checked_archive is missing." >&2
    exit 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$temporary/$checked_archive" | awk '{print $1}')
  else
    actual=$(shasum -a 256 "$temporary/$checked_archive" | awk '{print $1}')
  fi
  if [ "$actual" != "$expected" ]; then
    echo "Checksum verification failed for $checked_archive." >&2
    exit 1
  fi
done

tar -xzf "$temporary/$archive" -C "$temporary"
tar -xzf "$temporary/$wasi_archive" -C "$temporary"
mkdir -p "$install_dir"
cp "$temporary/bunting-${version}-${target}/bin/bunting" "$install_dir/bunting"
chmod 755 "$install_dir/bunting"
ln -sf bunting "$install_dir/bunting-tui"
cp "$temporary/bunting-${version}-wasi/bin/bunting-server" "$install_dir/bunting-server"
chmod 755 "$install_dir/bunting-server"
mkdir -p "$share_dir"
cp "$temporary/bunting-${version}-wasi/bin/bunting-server.wasm" "$share_dir/bunting-server.wasm"

mkdir -p "$config_dir"
for config in "$temporary/bunting-${version}-${target}/config/"*.json; do
  destination="$config_dir/$(basename "$config")"
  if [ ! -e "$destination" ]; then
    cp "$config" "$destination"
  fi
done

echo "Installed bunting $version, the Wasmer server launcher, and the TUI alias to $install_dir"
echo "Installed the portable WASI server module to $share_dir"
echo "Installed server configuration templates to $config_dir"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "Add $install_dir to PATH to run the commands." ;;
esac
