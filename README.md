<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="webui/src/lib/assets/btrnas-white.svg" />
    <source media="(prefers-color-scheme: light)" srcset="webui/src/lib/assets/btrnas.svg" />
    <img src="webui/src/lib/assets/btrnas-white.svg" width="300" alt="btrNAS" />
  </picture>
</p>

<p align="center">
  <strong>A NAS appliance built on Debian, btrfs, and ksmbd.</strong>
</p>

<p align="center">
  <img src="webui/src/lib/assets/icons/nas-bays.svg" width="72" alt="NAS enclosure with disk bays" />
  <img src="webui/src/lib/assets/icons/disk-stack.svg" width="72" alt="Stacked hard disks" />
  <img src="webui/src/lib/assets/icons/nas-network.svg" width="72" alt="NAS with network and a disk" />
  <img src="webui/src/lib/assets/icons/platter-tree.svg" width="72" alt="Disk platters in a storage tree" />
</p>

---

NASty is a NAS operating system built on **Debian Trixie** (this fork targets armhf / QNAP TS-228). It turns commodity hardware into a storage appliance serving NFS, SMB, FTP, SFTP, S3, iSCSI, and NVMe-oF — managed from a single web UI, updated via apt, and rolled back with snapper btrfs snapshots.

## Star History

<a href="https://www.star-history.com/">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=nasty-project/nasty&type=date&theme=dark&legend=top-left&sealed_token=j8EcY6L-xSWlQVDgIXXbwrqWyYn6QFx2CJHl3VCTqF0JfudPfxG0GZzd6ScaGSp04ca98RzbcTU5AmPvKdN34tWMo-Ok7N7QxUabUCs4kgIJCZ4OWlJK22PwzmFZdnKhtmcnjaL2RBfcg0x7K-4uWtcqQHMFqlwE0cH5qZaYdL1EDNMr5cxwa2PyDVOS" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=nasty-project/nasty&type=date&legend=top-left&sealed_token=j8EcY6L-xSWlQVDgIXXbwrqWyYn6QFx2CJHl3VCTqF0JfudPfxG0GZzd6ScaGSp04ca98RzbcTU5AmPvKdN34tWMo-Ok7N7QxUabUCs4kgIJCZ4OWlJK22PwzmFZdnKhtmcnjaL2RBfcg0x7K-4uWtcqQHMFqlwE0cH5qZaYdL1EDNMr5cxwa2PyDVOS" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=nasty-project/nasty&type=date&legend=top-left&sealed_token=j8EcY6L-xSWlQVDgIXXbwrqWyYn6QFx2CJHl3VCTqF0JfudPfxG0GZzd6ScaGSp04ca98RzbcTU5AmPvKdN34tWMo-Ok7N7QxUabUCs4kgIJCZ4OWlJK22PwzmFZdnKhtmcnjaL2RBfcg0x7K-4uWtcqQHMFqlwE0cH5qZaYdL1EDNMr5cxwa2PyDVOS" />
 </picture>
</a>

## Features

### Storage
- **btrfs** — compression, checksumming, multi-device profiles, subvolumes, and snapshots under `/fs/`
- **File sharing** — NFS, SMB (ksmbd), FTP, SFTP, and S3 (rclone serve) with per-share configuration
- **Block storage** — iSCSI and NVMe-oF with dedicated targets per volume, per-target portal management, and optional RDMA transports (iSER, NVMe-oF/RDMA, NFS-RDMA) for RoCE and InfiniBand NICs
- **Subvolumes** — filesystem and block subvolumes with optional compression
- **Snapshots** — space-efficient point-in-time copies (`subvol@snap`)
- **File browser** — browse, upload, edit, rename, copy, move, and bulk-manage files from the web UI
- **Backups** — encrypted, deduplicated, incremental backups to local, S3, SFTP, REST, or Backblaze B2 with per-profile schedules and retention — plus whole-snapshot restore, including disaster recovery onto a fresh box from an existing repository

### Monitoring & Alerts
- **Dashboard** — CPU, memory, storage, temperature, frequency — with scrollable history charts (30-day retention)
- **Alerts** — configurable rules for filesystem usage, disk health, temperatures, scrub errors, and more
- **Notifications** — alert delivery via SMTP email, Telegram, webhooks, and ntfy push notifications
- **S.M.A.R.T.** — disk health monitoring with per-disk details
- **[nasty-top](https://github.com/nasty-project/nasty-top)** — standalone TUI for live per-device IO, latency, and tuning

### Apps & VMs
- **Apps** — Docker containers and Compose stacks with image pull progress, container inspect, live per-app resource usage (CPU %, memory, network and disk I/O), and custom `.env` files for compose stacks, and an `allow_unsafe` escape hatch for stacks that need privileged options. See [Jellyfin on NASty](docs/jellyfin.md) for a complete media-server example
- **Virtual machines** — QEMU/KVM with VNC console, disk snapshots, USB passthrough (editable on existing VMs), bridge selection, and an inline disk-import wizard for qcow2, raw, img, vdi, and vmdk images (optionally .xz/.gz/.bz2 compressed)
- **Hardware passthrough** — IOMMU group view, USB device inventory, vfio-pci toggles that survive reboots, and SR-IOV virtual-function management (per-VF VLAN, MAC, trust, spoof-check)
- **Network bridges** — Linux bridges for attaching VMs (and apps) to L2 networks alongside the host

### System
- **Web UI** — manage filesystems, subvolumes, snapshots, shares, disks, services, and more
- **Web terminal** — built-in shell with command cheatsheets and diagnostic tools
- **Custom config** — optional `/etc/nasty/custom.conf` for notes/local overrides; NASty never overwrites it
- **Glossary** — built-in help page with storage terms, protocol guidance, and FAQ
- **Networking** — NetworkManager-based with confirm-or-rollback: edits stage, apply, and auto-revert if you don't confirm in time, so a typo can't lock you out over SSH
- **Let's Encrypt** — automatic TLS certificates via ACME (TLS-ALPN and DNS challenges)
- **Tailscale** — built-in VPN with one-click setup
- **Access control** — local user accounts with role-based permissions, API tokens, OIDC single sign-on, **WebAuthn / passkey** sign-in with admin-side credential reset, and an append-only audit log of every mutation, login attempt, and privileged-console open
- **Firewall** — engine-managed nftables, deny-by-default, with per-service source/interface restrictions and user-defined custom port rules for anything running outside NASty's service model
- **UPS monitoring** — NUT integration for graceful shutdown on power loss (opt-in)
- **Updates** — apt-based, with snapper btrfs snapshots and one-click rollback

> This TS-228 fork drops Samba AD / Time Machine (no vfs_fruit), native FS encryption/TPM, erasure coding, and bcachefs tiering. Local SMB users and btrfs pools are the supported path.

## Kubernetes

NASty can serve as a storage backend for Kubernetes — provisioning persistent volumes, snapshots, and clones on demand across all four protocols (NFS, SMB, iSCSI, NVMe-oF).

- **[nasty-csi](https://github.com/nasty-project/nasty-csi)** — CSI driver for dynamic provisioning, snapshots, cloning, and online expansion
- **[nasty-chart](https://github.com/nasty-project/nasty-chart)** — Helm chart for one-command install
- **[nasty-plugin](https://github.com/nasty-project/nasty-plugin)** — `kubectl-nasty` for inspecting volumes, snapshots, clones, and health from the CLI

## Community

Integrations built by the community on top of NASty's JSON-RPC API:

- **[nastyplugin](https://github.com/WarlockSyno/nastyplugin)** by [@WarlockSyno](https://github.com/WarlockSyno) — Proxmox storage plugin for using NASty as a backing store for VM and container disks

Building something with NASty? Open an issue or PR and we'll add it here.

## Screenshots

<p align="center">
  <img src="images/dashboard.jpg" width="800" alt="Dashboard — system overview with CPU, memory, storage, and network stats" />
</p>
<p align="center"><em>Dashboard</em></p>

<p align="center">
  <img src="images/filesystems.jpg" width="800" alt="Filesystems — bcachefs filesystem with compression, replicas, and scrub status" />
</p>
<p align="center"><em>Filesystems</em></p>

<p align="center">
  <img src="images/subvolumes.jpg" width="800" alt="Subvolumes — list with snapshots, block devices, and clone relationships" />
</p>
<p align="center"><em>Subvolumes</em></p>

<p align="center">
  <img src="images/sharing.jpg" width="800" alt="Sharing — per-subvolume NFS, SMB, iSCSI, and NVMe-oF shares" />
</p>
<p align="center"><em>Sharing</em></p>

<p align="center">
  <img src="images/apps.jpg" width="800" alt="Apps — Docker containers and compose stacks with live CPU and memory stats" />
</p>
<p align="center"><em>Apps</em></p>

<p align="center">
  <img src="images/terminal.jpg" width="800" alt="Terminal — built-in web shell with bcachefs tools" />
</p>
<p align="center"><em>Terminal</em></p>

<p align="center">
  <img src="images/settings.jpg" width="800" alt="Settings — hostname, timezone, log level, and telemetry" />
</p>
<p align="center"><em>Settings</em></p>

## Getting Started

1. Build `.deb` packages (see [INSTALL.md](INSTALL.md)) and install onto a Debian Trixie armhf rootfs
2. Enable `nasty-engine` / `nasty-metrics` (and Caddy)
3. Open the WebUI at `https://<nasty-ip>`
4. Default credentials: **admin** / **admin**

This fork does not ship a NixOS ISO. Root must be btrfs for snapper rollback.

## Update Flavors

NASty has three update flavors:

| Flavor | What you get | Description |
|--------|-------------|-------------|
| **Mild** | Tagged stable releases (`v*`) | Stable releases |
| **Spicy** | Pre-release builds (`s*`) | Pre-release builds with newer features |
| **Nasty** | Latest commit on main | Bleeding edge, no guarantees |

Switch flavors from **Settings → Update → Flavor** in the WebUI.

## Architecture

| Component | Technology | Why |
|-----------|------------|-----|
| Engine | Rust | Async runtime, handles all storage and system operations |
| Web UI | SvelteKit + TypeScript | Reactive UI with real-time WebSocket updates |
| OS | Debian Trixie | apt updates, snapper btrfs rollback (armhf / TS-228) |
| Filesystem | btrfs | Checksumming, compression, subvolumes, snapshots |
| API | JSON-RPC 2.0 over WebSocket | Persistent connection, bidirectional, low overhead |

## Project Structure

```
engine/         Rust workspace — storage, sharing, system management
webui/          SvelteKit web interface
debian/         Debian packaging (armhf .deb)
```

The full ecosystem (CSI driver, Helm chart, kubectl plugin, and more) lives at [github.com/nasty-project](https://github.com/nasty-project).

## FAQ

See [FAQ.md](FAQ.md) for common questions about this Debian port and project status.

## Telemetry

NASty sends a random installation ID and daily aggregate usage data for mounted storage, configured VMs, apps, and sharing exports, software version/build, and CPU architecture. The report does not include names, paths, file contents, hostnames, or hardware identifiers. Disable anytime from **Settings → Telemetry**. Details: [nasty-telemetry](https://github.com/nasty-project/nasty-telemetry).

## License

GPLv3
