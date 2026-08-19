# Installation (Debian Trixie / QNAP TS-228)

This fork targets **Debian 13 (Trixie)** on **armhf** (Realtek RTD1195),
not the upstream NixOS ISO. Root filesystem must be **btrfs** so snapper
can snapshot before apt upgrades.

## Prerequisites

- Working Debian Trixie rootfs on the TS-228 (or compatible armhf board)
- Root on btrfs
- Network access for `apt`

## Build packages (on a Debian Trixie host)

Native armhf (on-device or armhf chroot):

```bash
sudo apt-get install -y build-essential debhelper cargo rustc nodejs npm \
  libssl-dev pkg-config clang git
dpkg-buildpackage -b -us -uc
```

Cross from amd64:

```bash
sudo dpkg --add-architecture armhf
sudo apt-get update
sudo apt-get install -y crossbuild-essential-armhf debhelper \
  cargo rustc nodejs npm libssl-dev pkg-config clang git
rustup target add armv7-unknown-linux-gnueabihf   # if using rustup
CONFIG_SITE=/etc/dpkg-cross/cross-config.armhf \
  dpkg-buildpackage -b -us -uc -aarmhf
```

## Install

```bash
sudo apt-get install ./nasty-engine_*.deb ./nasty-webui_*.deb ./nasty_*.deb
sudo systemctl enable --now nasty-metrics nasty-engine
```

Merge or replace the Caddy config with `/etc/caddy/Caddyfile.nasty`, then:

```bash
sudo systemctl enable --now caddy
```

Open `https://<device-ip>` — default credentials remain **admin** / **admin**
until changed in the WebUI.

## Snapper

The `nasty` postinst creates `snapper` config `root` when `/` is btrfs.
Verify:

```bash
findmnt -no FSTYPE /
snapper -c root list
```

## Upstream NixOS ISO

The original NixOS installer under `nixos/` is not used on this port.
See upstream [nasty-project/nasty](https://github.com/nasty-project/nasty)
releases if you need NixOS images.
