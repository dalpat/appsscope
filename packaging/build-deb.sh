#!/usr/bin/env bash
#
# Build a .deb for AppsScope.
#
# Deliberately hand-rolled rather than debhelper: this is a single Rust binary
# plus four data files, and `dpkg-deb` does that in a way anyone can read in
# one sitting. It still uses `dpkg-shlibdeps` for the dependency line, because
# hand-written Depends are how packages end up either uninstallable or subtly
# broken on older releases.
#
# Usage: packaging/build-deb.sh [output-dir]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/dist}"

APP_ID="io.github.dalpat.AppsScope"
PKG="appsscope"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(dpkg --print-architecture)"

BIN="$ROOT/target/release/appsscope"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
# mktemp creates 0700; that mode would be baked into the package root and
# shipped as an unreadable "/".
chmod 755 "$STAGE"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found — run: cargo build --release -p appsscope-gtk" >&2
    exit 1
fi

echo "==> staging $PKG $VERSION ($ARCH)"

install -Dm755 "$BIN" "$STAGE/usr/bin/appsscope"
install -Dm644 "$ROOT/data/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$ROOT/data/$APP_ID.metainfo.xml" "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
install -Dm644 "$ROOT/data/icons/$APP_ID.svg" \
    "$STAGE/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"

# Debian expects a copyright file and a compressed changelog.
install -Dm644 "$ROOT/LICENSE" "$STAGE/usr/share/doc/$PKG/copyright" 2>/dev/null || \
    printf 'AppsScope\nCopyright (C) 2026\nLicence: GPL-3.0-or-later\n' \
        > >(install -Dm644 /dev/stdin "$STAGE/usr/share/doc/$PKG/copyright")

printf '%s (%s) unstable; urgency=low\n\n  * Initial release.\n\n -- AppsScope <noreply@localhost>  %s\n' \
    "$PKG" "$VERSION" "$(date -R)" | gzip -9n \
    > >(install -Dm644 /dev/stdin "$STAGE/usr/share/doc/$PKG/changelog.Debian.gz")

# Dependency line, derived from the binary's symbol versions rather than
# guessed. Falls back to a conservative hand-written list if dpkg-dev is
# absent, so the script still works on a machine without build tooling.
echo "==> resolving dependencies"
DEPENDS=""
if command -v dpkg-shlibdeps >/dev/null; then
    PROBE="$(mktemp -d)"
    mkdir -p "$PROBE/debian"
    printf 'Source: %s\nPackage: %s\nArchitecture: %s\nDescription: probe\n' \
        "$PKG" "$PKG" "$ARCH" > "$PROBE/debian/control"
    cp "$BIN" "$PROBE/appsscope"
    DEPENDS="$(cd "$PROBE" && dpkg-shlibdeps -O --ignore-missing-info ./appsscope 2>/dev/null \
        | sed 's/^shlibs:Depends=//')"
    rm -rf "$PROBE"
fi

if [[ -z "$DEPENDS" ]]; then
    echo "    dpkg-shlibdeps unavailable; using the conservative fallback list"
    DEPENDS="libadwaita-1-0 (>= 1.6), libgtk-4-1 (>= 4.12.0), libflatpak0 (>= 1.2), libc6 (>= 2.39), libgcc-s1 (>= 4.2)"
fi

# glib was renamed for the 64-bit time_t transition; accept either name so the
# package installs on releases from both sides of it.
#
# The version constraint has to be repeated on both alternatives: in Debian
# syntax `a | b (>= x)` constrains only `b`, which would have left the t64
# package unversioned.
DEPENDS="$(printf '%s' "$DEPENDS" | sed -E 's/libglib2\.0-0t64 \(([^)]*)\)/libglib2.0-0t64 (\1) | libglib2.0-0 (\1)/')"
echo "    $DEPENDS"

INSTALLED_KB="$(du -sk "$STAGE" | cut -f1)"

install -d "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Section: admin
Priority: optional
Architecture: $ARCH
Maintainer: AppsScope <noreply@localhost>
Installed-Size: $INSTALLED_KB
Depends: $DEPENDS
Recommends: flatpak, packagekit
Description: See what an app can access before you install it
 AppsScope is an app store for Linux that grades every application's sandbox
 permissions in plain language, shows what an install will really download
 including runtimes, and lets you revoke permissions without leaving the
 store. It manages Flatpaks and distro packages side by side.
 .
 Flatpak support needs the flatpak package; distro package management needs
 packagekit. Either may be absent — the backend is simply hidden.
EOF

# Refresh the desktop and icon caches so the app appears without a re-login.
cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database -q /usr/share/applications || true
    fi
    if command -v gtk-update-icon-cache >/dev/null; then
        gtk-update-icon-cache -qtf /usr/share/icons/hicolor || true
    fi
fi
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

cat > "$STAGE/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "remove" ]; then
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database -q /usr/share/applications || true
    fi
    if command -v gtk-update-icon-cache >/dev/null; then
        gtk-update-icon-cache -qtf /usr/share/icons/hicolor || true
    fi
fi
EOF
chmod 755 "$STAGE/DEBIAN/postrm"

mkdir -p "$OUT"
DEB="$OUT/${PKG}_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null

echo "==> built $DEB"
ls -lh "$DEB" | awk '{print "    " $5, $9}'
