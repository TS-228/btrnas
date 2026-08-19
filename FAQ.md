# FAQ

## Why does NASty exist?

Because bcachefs deserves a proper NAS appliance, and nobody was building one.

bcachefs is arguably the most interesting Linux filesystem in years. But using it for NAS meant CLI-only. Upstream NASty wraps it in an appliance with a web UI and NixOS. **This fork** ports that appliance to Debian Trixie (armhf / QNAP TS-228), with apt updates and snapper rollback, and is migrating storage/sharing toward btrfs + ksmbd.

## Why bcachefs instead of ZFS?

ZFS is battle-tested and great. I'm not here to trash it. But:

- **bcachefs is GPL.** Fits Linux perfectly. And with NASty you don't even need to know what DKMS is.
- **Simpler model.** A "filesystem" is just a filesystem. Subvolumes are just directories. Snapshots are just snapshots. No datasets, zvols, pools-within-pools, or property inheritance trees.
- **Modern features out of the box.** Tiering (move cold data to slow disks automatically), erasure coding, and online filesystem repair — things that ZFS either doesn't have or requires third-party tools.
- **Active development.** Kent Overstreet is shipping features at a pace ZFS hasn't seen in years.

The tradeoff: bcachefs is younger and less proven. I'm comfortable with that for a project that's explicitly exploring what's next.

## Why Debian (this fork)?

Upstream NASty uses NixOS for atomic generations. This QNAP TS-228 port uses **Debian Trixie** because the RTD1195 is armv7 and the board already runs a Debian rootfs on btrfs.

- **apt upgrades** apply package updates through the WebUI or CLI.
- **snapper** takes btrfs snapshots of `/` before/after apt so you can roll back.
- Packages ship as `.deb` files (`nasty`, `nasty-engine`, `nasty-webui`).

## Can I add custom configuration?

Yes. Drop notes or local overrides in `/etc/nasty/custom.conf`. NASty never overwrites that file. System packages and units are managed with apt/systemd as usual.

## Is this production-ready?

No. NASty is experimental and under active development. bcachefs itself is still maturing.

That said, NASty is probably the most thoroughly tested one-person NAS project you'll find:

- **170 Kubernetes E2E tests** — real cluster provisioning real volumes over all four network protocols, including snapshots, clones, and scale tests
- **362 CSI driver unit tests** — covering node staging, volume lifecycle, health monitoring, and recovery
- **76 CSI sanity tests** — spec compliance verification
- **Integration test suite** — exercising the engine API across all protocols, snapshots, clones, and data integrity
- CI/CD pipeline builds, lints, tests, and publishes container images automatically

Use it for homelabs, development, and learning. Not for storing your only copy of irreplaceable data. Yet.

## What protocols does NASty support?

- **NFS** — Network File System. Standard Linux/Unix file sharing.
- **SMB** — Server Message Block. Windows/macOS file sharing over the network.
- **FTP** — File Transfer Protocol via `rclone serve ftp`. Shares appear as top-level folders. One username/password for the protocol (optional anonymous login). Default port 21 plus a passive range.
- **SFTP** — SSH File Transfer via `rclone serve sftp` on port 2022 (so it does not collide with the box SSH daemon). Encrypted; one username/password for the protocol.
- **S3** — S3-compatible object API via `rclone serve s3`. Share names are path-style bucket names. One access key / secret key for the protocol. Default port 9000.
- **iSCSI** — Internet SCSI. Block storage over TCP. Used by Kubernetes for persistent volumes.
- **NVMe-oF** — NVMe over Fabrics. High-performance block storage over TCP. The modern alternative to iSCSI.

File and object protocols (NFS, SMB, FTP, SFTP, S3) plus the two block protocols are managed through the same WebUI and API. The Kubernetes CSI driver supports NFS, SMB, iSCSI, and NVMe-oF.

## How are snapshots and clones different from ZFS?

Simpler.

In bcachefs, a snapshot IS a subvolume. It's a first-class citizen, not a dependent child of its parent. Delete the parent — the snapshot survives. No "promote", no "detach", no dependency chains.

A clone is just a writable snapshot. One command: `bcachefs subvolume snapshot` (without `-r`). Instant, COW, fully independent. No clone modes, no send/receive for independence.

## What about VMs and Apps?

They work, but both are early-stage.

VMs use QEMU/KVM with a noVNC console in the browser. You can create a VM, boot an ISO, and use it. It won't replace Proxmox, but it handles simple workloads.

Apps run on Docker. You can deploy single containers or full Compose stacks from the web UI. The management interface is basic.

Both features are under active development. Contributions in these areas would have outsized impact.

## Can I run Jellyfin?

Yes. Jellyfin runs as a Docker Compose app and does not require any NixOS experience. See [Jellyfin on NASty](docs/jellyfin.md) for a safe CPU/direct-play setup, optional HTTPS and hardware acceleration, updates, backups, and troubleshooting.

## How can I help?

Try it. Break it. Tell me what sucks. Open issues. Send patches. Or just use it and let the telemetry tell me you exist — that alone is motivating.

NASty is a small project and always looking for contributors. Whether you're into Rust, SvelteKit, NixOS, Kubernetes, bcachefs, or just want a NAS that doesn't feel like it was designed in 2005 — there's something here for you.

The best way to start:
- **Use it** — install on spare hardware, play with it, find the rough edges
- **File issues** — even "this confused me" is valuable feedback
- **Join the conversation** — bcachefs IRC on OFTC (`#bcachefs`) or [Matrix](https://matrix.to/#/#_oftc_%23bcache:matrix.org)
- **Contribute code** — pick an issue, send a PR, or just improve something that bothers you

No contribution is too small. Documentation fixes, typo corrections, better error messages — it all counts.

## Where does the name come from?

NAS + ty. It's the only English word with "NAS" in it that I could think of. Maybe there are others. I don't care. Just a NAS that's a bit nasty.
