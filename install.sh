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
  Linux:aarch64 | Linux:arm64) target="aarch64-unknown-linux-gnu" ;;
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
base_url="${BUNTING_DOWNLOAD_BASE:-https://github.com/$repository/releases/download/$version}"
temporary=$(mktemp -d)
trap '[ -d "$temporary/mount" ] && hdiutil detach -quiet "$temporary/mount" 2>/dev/null; rm -rf "$temporary"' EXIT HUP INT TERM

curl -fsSL "$base_url/SHA256SUMS" -o "$temporary/SHA256SUMS"

# Whether the release lists this file (older releases lack the app bundles).
published() {
  awk -v name="$1" '$2 == name {found = 1} END {exit !found}' "$temporary/SHA256SUMS"
}

# Downloads one release file into $temporary and checks it against SHA256SUMS.
fetch() {
  expected=$(awk -v name="$1" '$2 == name {print $1}' "$temporary/SHA256SUMS")
  if [ -z "$expected" ]; then
    echo "Release checksum for $1 is missing." >&2
    exit 1
  fi
  curl -fsSL "$base_url/$1" -o "$temporary/$1"
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$temporary/$1" | awk '{print $1}')
  else
    actual=$(shasum -a 256 "$temporary/$1" | awk '{print $1}')
  fi
  if [ "$actual" != "$expected" ]; then
    echo "Checksum verification failed for $1." >&2
    exit 1
  fi
}

fetch "$archive"
tar -xzf "$temporary/$archive" -C "$temporary"
mkdir -p "$install_dir"
cp "$temporary/bunting-${version}-${target}/bin/bunting" "$install_dir/bunting"
chmod 755 "$install_dir/bunting"
cp "$temporary/bunting-${version}-${target}/bin/bunting-trader" "$install_dir/bunting-trader"
chmod 755 "$install_dir/bunting-trader"
ln -sf bunting "$install_dir/bunting-tui"
# Replaces the Wasmer launcher earlier releases installed (ADR 0044).
rm -f "$install_dir/bunting-server"
ln -s bunting "$install_dir/bunting-server"
rm -f "$share_dir/bunting-server.wasm"

mkdir -p "$config_dir"
for config in "$temporary/bunting-${version}-${target}/config/"*.json; do
  destination="$config_dir/$(basename "$config")"
  if [ ! -e "$destination" ]; then
    cp "$config" "$destination"
  fi
done

# The desktop app: always on macOS, on Linux when a display is present.
# BUNTING_DESK=1 or BUNTING_DESK=0 decides instead.
desk="${BUNTING_DESK:-auto}"
if [ "$desk" = auto ]; then
  case "$(uname -s)" in
    Darwin) desk=1 ;;
    *) if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then desk=1; else desk=0; fi ;;
  esac
fi
desk_message=""
if [ "$desk" = 1 ]; then
  case "$target" in
    *-apple-darwin)
      image="Bunting-${version}-macOS.dmg"
      if published "$image"; then
        fetch "$image"
        # /Applications when this user may write there (admins can), else
        # ~/Applications. Downloaded by curl, the app has no quarantine flag.
        if [ -n "${BUNTING_APP_DIR:-}" ]; then
          app_dir="$BUNTING_APP_DIR"
        elif [ -w /Applications ]; then
          app_dir=/Applications
        else
          app_dir="$HOME/Applications"
        fi
        mkdir -p "$temporary/mount" "$app_dir"
        hdiutil attach -quiet -nobrowse -readonly -mountpoint "$temporary/mount" "$temporary/$image"
        rm -rf "$app_dir/Bunting.app"
        ditto "$temporary/mount/Bunting.app" "$app_dir/Bunting.app"
        hdiutil detach -quiet "$temporary/mount"
        rmdir "$temporary/mount"
        desk_message="Installed Bunting to $app_dir: open it to trade in the practice market."
      else
        desk_message="Release $version has no macOS app; download it from a newer release."
      fi
      ;;
    *)
      desk_archive="bunting-desk-${version}-${target}.tar.gz"
      if published "$desk_archive"; then
        fetch "$desk_archive"
        tar -xzf "$temporary/$desk_archive" -C "$temporary"
        desk_dir="$temporary/bunting-desk-${version}-${target}"
        cp "$desk_dir/bin/bunting-desk" "$install_dir/bunting-desk"
        chmod 755 "$install_dir/bunting-desk"
        if [ -f "$desk_dir/share/applications/bunting-desk.desktop" ]; then
          data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
          mkdir -p "$data_home/applications" "$data_home/icons/hicolor/scalable/apps"
          cp "$desk_dir/share/icons/hicolor/scalable/apps/bunting-desk.svg" \
            "$data_home/icons/hicolor/scalable/apps/bunting-desk.svg"
          sed "s|@EXEC@|\"$install_dir/bunting-desk\"|" \
            "$desk_dir/share/applications/bunting-desk.desktop" \
            >"$data_home/applications/bunting-desk.desktop"
          if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "$data_home/applications" >/dev/null 2>&1 || true
          fi
          desk_message="Installed the Bunting app to $install_dir/bunting-desk and your app launcher: open it to trade in the practice market."
        else
          desk_message="Installed bunting-desk to $install_dir: run it to trade in the practice market."
        fi
      else
        desk_message="Release $version has no desktop app for $target."
      fi
      ;;
  esac
fi

echo "Installed bunting $version (with bunting-server and bunting-tui aliases) and bunting-trader to $install_dir"
echo "Installed server configuration templates to $config_dir"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "Add $install_dir to PATH to run the commands." ;;
esac
if [ -n "$desk_message" ]; then
  echo "$desk_message"
fi
