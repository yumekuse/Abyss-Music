#!/bin/sh
# abyss — install from the tarball (any distro).
#   ./install.sh            installs for you only  (~/.local)
#   sudo ./install.sh       installs for everyone  (/usr/local)
#   ./install.sh --uninstall   removes it again (your settings in ~/.config/abyss stay)
set -e
here=$(cd "$(dirname "$0")" && pwd)
if [ "$(id -u)" = "0" ]; then prefix=/usr/local; else prefix="$HOME/.local"; fi

if [ "$1" = "--uninstall" ]; then
  rm -f "$prefix/bin/abyss" "$prefix/share/applications/abyss.desktop" "$prefix/share/icons/hicolor/256x256/apps/abyss.png"
  echo "abyss removed from $prefix"
  exit 0
fi

install -Dm755 "$here/abyss" "$prefix/bin/abyss"
install -Dm644 "$here/abyss.desktop" "$prefix/share/applications/abyss.desktop"
install -Dm644 "$here/abyss.png" "$prefix/share/icons/hicolor/256x256/apps/abyss.png"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$prefix/share/icons/hicolor" 2>/dev/null || true

echo "abyss installed to $prefix/bin/abyss"
case ":$PATH:" in *":$prefix/bin:"*) ;; *) echo "note: add $prefix/bin to your PATH to start it with 'abyss'";; esac

# runtime libraries abyss needs (WebKitGTK + GStreamer codecs for the YouTube player)
missing=""
for lib in libwebkit2gtk-4.1.so.0 libgtk-3.so.0; do
  ldconfig -p 2>/dev/null | grep -q "$lib" || missing="$missing $lib"
done
if [ -n "$missing" ]; then
  echo
  echo "missing:$missing — install them with one of:"
  echo "  Arch / CachyOS / Manjaro : sudo pacman -S --needed webkit2gtk-4.1 gst-plugins-good gst-plugins-bad gst-libav"
  echo "  Debian / Ubuntu / Kali   : sudo apt install libwebkit2gtk-4.1-0 gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav"
  echo "  Fedora                   : sudo dnf install webkit2gtk4.1 gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-libav"
  echo "  openSUSE                 : sudo zypper install libwebkit2gtk-4_1-0 gstreamer-plugins-good gstreamer-plugins-bad gstreamer-plugins-libav"
fi
