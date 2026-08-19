#!/bin/bash
# Cross-build NASty .debs for armhf inside Debian Trixie (podman/docker).
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
export PATH="/root/.cargo/bin:/usr/local/cargo/bin:${PATH}"

dpkg --add-architecture armhf
apt-get update
apt-get install -y --no-install-recommends \
  build-essential debhelper \
  crossbuild-essential-armhf \
  pkg-config clang ca-certificates git curl \
  nodejs npm \
  libssl-dev:armhf \
  libsqlite3-dev:armhf \
  pkg-config:armhf

# Prefer host-mounted rustup if present; otherwise install.
if ! command -v rustc >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi
rustup target add armv7-unknown-linux-gnueabihf

mkdir -p /src/.cargo
cat > /src/.cargo/config.toml <<'EOF'
[target.armv7-unknown-linux-gnueabihf]
linker = "arm-linux-gnueabihf-gcc"

[env]
PKG_CONFIG_ALLOW_CROSS = "1"
EOF

export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_PATH=/usr/lib/arm-linux-gnueabihf/pkgconfig
export PKG_CONFIG_LIBDIR=/usr/lib/arm-linux-gnueabihf/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=/
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc
# openssl-sys via pkg-config for armhf
export OPENSSL_LIB_DIR=/usr/lib/arm-linux-gnueabihf
export OPENSSL_INCLUDE_DIR=/usr/include

cd /src
# Parent dir must exist for dpkg-buildpackage output (../*.deb)
mkdir -p /out
ln -sfn /src /tmp/nasty-src

# Use debian/rules via dpkg-buildpackage for armhf
dpkg-buildpackage -aarmhf -b -us -uc -j"$(nproc)"

# Collect artifacts
mkdir -p /src/dist
cp -v /nasty_*.deb /nasty-engine_*.deb /nasty-webui_*.deb /src/dist/ 2>/dev/null \
  || cp -v ../*.deb /src/dist/ 2>/dev/null \
  || true
# dpkg-buildpackage writes to parent of source when source is /src and
# parent is not writable — use -nc and check /
ls -la /src/dist/ / /tmp 2>/dev/null | head -40
find / -maxdepth 2 -name 'nasty*.deb' 2>/dev/null
