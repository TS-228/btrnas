<script lang="ts">
	import { Card, CardContent } from '$lib/components/ui/card';
	import { CircleHelp } from '@lucide/svelte';

	type Entry = { term: string; summary: string; detail?: string };

	const sections: { title: string; entries: Entry[] }[] = [
		{
			title: 'Getting Started',
			entries: [
				{
					term: 'Filesystem',
					summary: 'A storage pool built from one or more disks.',
					detail: 'In NASty, a filesystem is a bcachefs pool that spans one or more devices. All your data, subvolumes, and shares live inside a filesystem. You need at least one filesystem before you can store anything. Create one under Filesystems.',
				},
				{
					term: 'Subvolume',
					summary: 'An isolated directory or block device within a filesystem.',
					detail: 'Subvolumes are lightweight divisions of a filesystem. Each subvolume can have its own quota, compression, and tiering settings. There are two types: "filesystem" subvolumes (used for NFS, SMB, FTP, SFTP, and S3 shares) and "block" subvolumes (used for iSCSI/NVMe-oF block storage). Think of them like folders with superpowers — they can be snapshotted, quota-limited, and independently managed.',
				},
				{
					term: 'Share',
					summary: 'A subvolume exported over the network so other machines can access it.',
					detail: 'A share makes a subvolume available to other computers on your network using a protocol like NFS, SMB, FTP, SFTP, or S3. Without a share, data in a subvolume is only accessible locally on the NAS.',
				},
				{
					term: 'Snapshot',
					summary: 'A point-in-time copy of a subvolume.',
					detail: 'Snapshots are instant, space-efficient copies. They don\'t duplicate data — they share blocks with the original and only consume space as data diverges. Useful for backups and rollback.',
				},
			],
		},
		{
			title: 'Sharing Protocols',
			entries: [
				{
					term: 'NFS',
					summary: 'Network File System — the standard for Linux and macOS file sharing.',
					detail: 'Use NFS when your clients are Linux servers, Kubernetes nodes, or macOS workstations. It\'s fast, low-overhead, and widely supported. Best for: home labs, media servers, container storage, development environments.',
				},
				{
					term: 'SMB',
					summary: 'Server Message Block — the standard for Windows file sharing.',
					detail: 'Use SMB when your clients are Windows PCs or you need broad compatibility. Also works with macOS and Linux. Supports user authentication. Best for: Windows networks, mixed OS environments, desktop file access.',
				},
				{
					term: 'FTP',
					summary: 'File Transfer Protocol — a simple file server for cameras, scanners, and legacy clients.',
					detail: 'Served by rclone. Shares appear as top-level folders under one listener (default port 21, plus a passive port range). Authentication is a single username/password for the whole protocol, not per share. FTP is unencrypted — prefer SFTP on untrusted networks. Enable it under Sharing or Services, then create shares like SMB.',
				},
				{
					term: 'SFTP',
					summary: 'SSH File Transfer — encrypted file access over SSH, without opening the box SSH daemon.',
					detail: 'Served by rclone on port 2022 by default so it does not collide with SSH on port 22. Shares appear as top-level folders. Like FTP, credentials are per-protocol (one user). Best for: encrypted uploads from scripts, NAS clients, and tools that speak SFTP but not SMB.',
				},
				{
					term: 'S3',
					summary: 'S3-compatible object API — apps talk to shares as buckets.',
					detail: 'Served by rclone on port 9000 by default. Each share name is a path-style bucket. Access key and secret key are protocol-level (one pair for every bucket). Best for: backup tools, object-storage clients, and apps that expect an S3 endpoint rather than a file share.',
				},
				{
					term: 'iSCSI',
					summary: 'Internet SCSI — presents a block device over the network.',
					detail: 'Use iSCSI when you need raw block storage — the client sees a disk, not files. Common for databases, VMs, and applications that need direct disk access. The client formats and mounts the block device itself. Best for: databases, virtual machines, applications needing consistent low-latency block I/O.',
				},
				{
					term: 'NVMe-oF',
					summary: 'NVMe over Fabrics — high-performance block storage over the network.',
					detail: 'Like iSCSI but faster — uses the NVMe protocol natively over the network. Requires NVMe-oF support on both ends. Best for: high-performance workloads, low-latency requirements, modern infrastructure.',
				},
				{
					term: 'RDMA',
					summary: 'Remote Direct Memory Access — NICs move data between machines\' memory directly, bypassing most of the CPU and TCP overhead.',
					detail: 'With RDMA-capable NICs on both ends, storage traffic skips the kernel\'s TCP path almost entirely: lower latency, higher throughput, less CPU burned per gigabyte. NASty supports RDMA transports for its block and file protocols — iSER (iSCSI), NVMe-oF over RDMA, and NFS-RDMA. It\'s a per-box opt-in on the Sharing page and needs an RDMA-capable NIC (RoCE or InfiniBand); everything keeps working over plain TCP without it.',
				},
				{
					term: 'RoCE',
					summary: 'RDMA over Converged Ethernet — RDMA running on ordinary Ethernet gear.',
					detail: 'RoCEv2 encapsulates all RDMA traffic in UDP port 4791 — the "ports" you see for NVMe/RDMA (4420) or NFS-RDMA (20049) are RDMA connection-manager service IDs, not real IP ports. Practical consequence: firewall source restrictions on RoCE traffic can only filter at the 4791 level, not per service. Works over standard (ideally lossless-configured) Ethernet switches.',
				},
				{
					term: 'InfiniBand',
					summary: 'A dedicated high-speed fabric with native RDMA — an alternative to Ethernet, not something that runs on it.',
					detail: 'InfiniBand is its own network technology with its own switches and cabling. Native InfiniBand traffic never traverses the IP firewall at all — no port rules needed or possible. IP-over-InfiniBand (IPoIB) provides a normal IP interface on the same fabric for everything else.',
				},
				{
					term: 'iSER',
					summary: 'iSCSI Extensions for RDMA — the iSCSI protocol with its data path moved onto RDMA.',
					detail: 'Same iSCSI targets, LUNs, and ACLs you already configured — but data flows over the RDMA transport instead of TCP. Enable RDMA on the Sharing page and iSER portals become available alongside plain TCP ones. The initiator side needs iSER support too (Linux open-iscsi has it).',
				},
				{
					term: 'iSCSI Portal',
					summary: 'The address and port an iSCSI target listens on.',
					detail: 'Each iSCSI target accepts connections through one or more portals (IP:port pairs). NASty lets you manage portals per target — bind a target to a specific interface or a custom port — and the firewall follows the configured portals automatically, so a portal on a non-default port is reachable without manual rules.',
				},
				{
					term: 'Guest Share',
					summary: 'A public link to a file or folder for someone who has no NASty account.',
					detail: 'Create one from the Files page (the Share action) to hand a file or whole folder to an outside recipient. The link itself is the credential — only its hash is stored, so it\'s shown once at creation and can\'t be retrieved afterwards. Optional controls: an expiry, a password, and a download limit. Folders download as a streamed ZIP. Recipients land on a no-login page; downloads are always served as attachments (never rendered inline) so shared content can\'t run on the app origin, and any unavailable link (expired, revoked, over its limit) returns the same generic message. Manage and revoke links under Sharing → Guest Shares.',
				},
			],
		},
		{
			title: 'Storage Concepts',
			entries: [
				{
					term: 'Quota',
					summary: 'A size limit on a subvolume.',
					detail: 'Quotas prevent a subvolume from consuming more than its allocated space. For block subvolumes (iSCSI/NVMe-oF), the quota defines the size of the virtual disk. For filesystem subvolumes (NFS, SMB, FTP, SFTP, S3), it\'s optional — without one, the subvolume can use all available space.',
				},
				{
					term: 'Replication',
					summary: 'Storing multiple copies of data across devices for redundancy.',
					detail: 'With 2x replication, every block is written to two different disks. If one disk fails, your data is still intact on the other. Higher replication means more safety but uses more space. Also called "mirroring" in traditional RAID terminology.',
				},
				{
					term: 'Compression',
					summary: 'Reducing data size on disk to save space.',
					detail: 'bcachefs supports transparent compression — data is compressed when written and decompressed when read. Options: lz4 (fast, moderate compression), zstd (good balance), gzip (maximum compression, slower). Compression is per-subvolume and can be changed at any time.',
				},
				{
					term: 'Tiering',
					summary: 'Automatically moving data between fast and slow storage.',
					detail: 'If your filesystem has both SSDs and HDDs, tiering writes new data to the fast tier (SSD) and moves cold data to the slow tier (HDD) in the background. This gives you SSD performance with HDD capacity. Configured via foreground/background/promote targets.',
				},
				{
					term: 'Scrub',
					summary: 'A background check that verifies all data checksums.',
					detail: 'Scrubbing reads every block and verifies its checksum to detect silent data corruption (bit rot). If replication is enabled, corrupted copies are automatically repaired from good ones. Run periodically — e.g., monthly.',
				},
				{
					term: 'Reconcile',
					summary: 'Background rebalancing of data across devices.',
					detail: 'Reconcile moves data between devices to maintain the desired layout — for example, after adding or removing a disk, or after changing tiering targets. It runs automatically when enabled.',
				},
				{
					term: 'Evacuate',
					summary: 'Moving all data off a device so it can be removed from the filesystem.',
					detail: 'When you need to replace or decommission a disk, evacuate migrates every block off that device onto other devices in the pool. The device is non-writable during evacuation but stays in the filesystem; once finished it reports as "evacuated" and can be removed. Progress is visible in the Operations page and the sidebar status band. Always evacuate before removing a device — pulling one without evacuating can put the filesystem into degraded mode.',
				},
				{
					term: 'Fsck',
					summary: 'A filesystem check that verifies bcachefs metadata is consistent.',
					detail: 'Fsck scans the filesystem\'s metadata for structural issues — missing extents, incorrect reference counts, orphaned blocks. NASty offers two modes from the Operations page: a safe read-only check (reports issues without touching anything), and a repair pass that rewrites metadata to fix what was found. Run a check after an unclean shutdown or power loss. Unlike scrub (which verifies data checksums at the block level), fsck checks the structural integrity of the filesystem itself.',
				},
				{
					term: 'Copy GC',
					summary: 'A background process that reclaims free space by compacting fragmented data.',
					detail: 'Copy GC (copy garbage collection) reads partially-empty data blocks and rewrites their live data into fuller ones so the empties can be freed — the main mechanism for reclaiming space after deletions and overwrites. It runs automatically and most users never need to touch it. Under Operations you can pause it during performance-sensitive workloads. When a "needs_gc" flag appears on the filesystem, a full GC pass can be triggered manually.',
				},
				{
					term: 'Erasure Coding',
					summary: 'Parity-based redundancy that uses less space than mirroring.',
					detail: 'Instead of storing N full copies, erasure coding stores data plus parity blocks across multiple devices. Roughly: with replicas=2 and EC on, the layout is RAID-5-like (one parity); replicas=3 with EC is RAID-6-like (two parity). Usable capacity scales with (devices − parity) / devices, which is much better than 1/replicas mirroring once you have several disks. Trade-off: rebuilds and small writes are more expensive. Toggle when creating the filesystem.',
				},
				{
					term: 'Encryption',
					summary: 'At-rest encryption of every block written to the filesystem.',
					detail: 'bcachefs encrypts data and metadata with a key derived from a passphrase you provide at filesystem creation. The passphrase is required to unlock the filesystem at boot. Encryption is set at filesystem creation and cannot be added or removed later — pick it up front if you need it.',
				},
			],
		},
		{
			title: 'Disk Management',
			entries: [
				{
					term: 'Disk / Device',
					summary: 'A physical or virtual storage device (SSD, HDD, NVMe drive).',
					detail: 'NASty discovers all block devices in the system. Before a disk can be used in a filesystem, it may need to be wiped to remove existing partition tables or filesystem signatures.',
				},
				{
					term: 'Partition',
					summary: 'A section of a disk, divided at the hardware level.',
					detail: 'A single physical disk can be split into multiple partitions, each acting as a separate device. Most NAS setups use whole disks rather than partitions. Partitions are mainly relevant when a disk has an existing OS or data you want to preserve.',
				},
				{
					term: 'Wipe',
					summary: 'Erasing signatures and partition tables from a disk.',
					detail: 'Wiping removes filesystem signatures and partition tables so bcachefs can use the disk. This is destructive — all existing data on the disk is lost. Required when a disk was previously used by another system or filesystem.',
				},
				{
					term: 'Durability',
					summary: 'How reliable a device is considered for replication purposes.',
					detail: '0 = cache only (data is not durable), 1 = normal disk, 2 = hardware RAID or highly reliable storage. bcachefs uses this to decide where to place replicas — it won\'t put two replicas on devices with the same durability group.',
				},
			],
		},
		{
			title: 'System & Updates',
			entries: [
				{
					term: 'NixOS',
					summary: 'The Linux distribution that NASty runs on.',
					detail: 'NixOS is a declarative operating system — the entire system configuration is defined in code and rebuilt atomically. This means updates are safe and rollback is always possible. You don\'t need to know NixOS to use NASty, but it\'s why updates and rollbacks work so reliably.',
				},
				{
					term: 'Generation',
					summary: 'A snapshot of the entire system configuration.',
					detail: 'Every time NASty updates, NixOS creates a new generation — a complete, bootable system state. If an update causes problems, you can roll back to a previous generation from the Update page or the boot menu. Old generations can be garbage-collected to free disk space.',
				},
				{
					term: 'Firmware',
					summary: 'Low-level software embedded in hardware devices.',
					detail: 'Disk drives, network cards, and motherboards all have firmware. NASty can update firmware for supported devices through the fwupd service. Keeping firmware up to date improves stability and security.',
				},
			],
		},
		{
			title: 'Apps & Virtualization',
			entries: [
				{
					term: 'VM (Virtual Machine)',
					summary: 'A full computer emulated in software.',
					detail: 'VMs run a complete operating system with its own kernel, isolated from the host. Use VMs when you need a different OS (e.g., Windows), full isolation, or software that can\'t run in containers. Requires KVM support in the CPU.',
				},
				{
					term: 'App',
					summary: 'A self-contained application running in a Docker container.',
					detail: 'NASty\'s Apps page lets you deploy and manage Docker containers — pre-packaged applications like media servers, download managers, or home automation. Each app runs isolated from the host system.',
				},
				{
					term: 'Docker',
					summary: 'A container runtime for running isolated applications.',
					detail: 'Docker packages an application and all its dependencies into a container — a lightweight, portable unit that runs the same everywhere. Containers share the host kernel, making them much lighter than VMs.',
				},
				{
					term: 'Docker Compose',
					summary: 'A tool for defining multi-container applications.',
					detail: 'Some apps need multiple containers working together (e.g., a web app + database). Docker Compose defines these in a single YAML file, managing networking and dependencies between containers automatically.',
				},
				{
					term: 'Managed Startup (Startup Order)',
					summary: 'Have NASty bring compose stacks up at boot in a set order, with a delay after each.',
					detail: 'By default Docker starts compose stacks in arbitrary order at boot, per each stack\'s own restart policy. Enroll a stack into managed startup (Apps → Compose Startup Order) and the NASty engine owns its boot startup instead: managed stacks come up in the order you choose, with a configurable settle delay after each — handy when a "network" stack must create shared Docker networks before the stacks that depend on them. Managed stacks are pinned to restart: "no" through a generated compose override (your own compose file is left untouched) so Docker doesn\'t race the engine; unenroll a stack and it reverts to its own restart policy. If a stack fails to start, it\'s logged and the sequence continues with the rest.',
				},
				{
					term: 'allow_unsafe',
					summary: 'Per-app opt-in for privileged or host-impacting container options.',
					detail: 'NASty sandboxes app deploys by default — capabilities, host-path mounts, and other "escape hatch" options are stripped from compose files and rejected on simple installs. Set allow_unsafe on an app when you genuinely need things like privileged mode, host networking, or mounting /var. This is logged and visible on the app list so you remember which apps are running with extra trust.',
				},
				{
					term: 'Network Bridge',
					summary: 'A virtual L2 switch that lets VMs (and apps) share the host LAN.',
					detail: 'A bridge ties one or more host interfaces together so guests attached to it appear as ordinary devices on your physical network — they can pull DHCP from your router and be reached directly by IP. Configured under Network → Bridges; VMs select a bridge as their NIC backing instead of the default user-mode networking.',
				},
				{
					term: 'Ingress',
					summary: 'Reverse proxy routing that gives apps a public URL with automatic TLS.',
					detail: 'When you install an app, NASty can expose it through the Caddy reverse proxy — giving it a hostname or subpath on your NASty domain. The Ingress page lists every route Caddy is serving (host matches, path prefixes, catch-all). Routes are managed per-app under the app\'s settings: toggle the subdomain, pick a port, or add custom path rules. All ingress traffic goes through port 443 with TLS handled automatically.',
				},
			],
		},
		{
			title: 'Networking & Services',
			entries: [
				{
					term: 'SSH',
					summary: 'Secure Shell — encrypted remote terminal access.',
					detail: 'SSH lets you connect to NASty\'s command line from another computer. Used for advanced administration, scripting, and debugging. Can be configured with password or key-based authentication.',
				},
				{
					term: 'Avahi (mDNS)',
					summary: 'Automatic network discovery — makes NASty findable by name.',
					detail: 'Avahi broadcasts NASty\'s hostname on the local network using mDNS (multicast DNS). This is why you can reach your NAS at nasty.local instead of memorizing an IP address. Works out of the box on macOS and most Linux desktops. Windows may need Bonjour installed.',
				},
				{
					term: 'SMART',
					summary: 'Self-Monitoring, Analysis and Reporting Technology for disks.',
					detail: 'SMART is built into every modern disk drive. It tracks health indicators like temperature, error counts, and hours of operation. NASty monitors SMART data and can alert you when a disk shows signs of failure — often before data loss occurs.',
				},
				{
					term: 'Terminal',
					summary: 'A command-line shell running directly on NASty.',
					detail: 'The built-in terminal gives you a bash shell on the NAS, accessible from the web UI. Useful for running bcachefs commands, inspecting logs, or anything the web UI doesn\'t cover. Commands like nasty-top are available here.',
				},
				{
					term: 'Caddy',
					summary: 'The reverse proxy and TLS terminator running in front of the engine.',
					detail: 'Caddy serves the web UI on port 443, terminates HTTPS, and proxies /api/* and /ws/* to the engine on 127.0.0.1:2137. Certificates come from either Let\'s Encrypt (ACME) when you\'ve set a real domain, or from Caddy\'s built-in "internal" CA when you haven\'t — that\'s the self-signed cert you\'ll see on nasty.local and on the box\'s IP addresses. The engine talks to Caddy through its admin API on 127.0.0.1:2019 to push per-app routes and TLS automation policies at runtime, so app installs and TLS settings changes apply without restarting anything. Replaced nginx in 0.0.8. Logs: journalctl -u caddy.',
				},
				{
					term: 'ACME / Let\'s Encrypt',
					summary: 'Automatic TLS certificates for the web UI.',
					detail: 'NASty can request a free, trusted TLS certificate for your hostname from Let\'s Encrypt and renew it automatically. Two challenge types are supported: TLS-ALPN (works when NASty is reachable on port 443 from the internet) and DNS-01 (works behind a NAT / on a private network, but needs API credentials for your DNS provider). Issuance is handled by Caddy. Configure under Settings → TLS.',
				},
				{
					term: 'Tailscale',
					summary: 'Mesh VPN for reaching NASty from anywhere.',
					detail: 'Tailscale builds a private network between your devices over WireGuard. Once you connect from NASty\'s VPN page, your NAS gets a stable Tailscale IP and a *.ts.net hostname reachable from any of your other Tailscale-enabled machines — phone, laptop, server — without exposing it to the public internet. Useful for offsite backups and remote access. Tailscale package updates are applied atomically from System → Update because native self-updates are unsupported on NixOS.',
				},
				{
					term: 'UPS / NUT',
					summary: 'Talks to a battery backup so NASty can shut down cleanly on power loss.',
					detail: 'NUT (Network UPS Tools) lets NASty read state from a USB- or network-attached UPS. When the UPS reports low battery, NASty shuts down gracefully so you don\'t lose data to a hard power-off. Optional — enable it under Services if you have a UPS connected.',
				},
				{
					term: 'Firewall',
					summary: 'A packet filter that controls which traffic reaches NASty and its services.',
					detail: 'NASty uses nftables for firewall rules, managed from the Firewall page. Each service (NFS, SMB, SSH, etc.) lists its port and current rule — open to all, restricted to specific source IPs or networks, or closed. Rules apply to selected interfaces (LAN, VPN) independently. Published NASty app ports are allowed automatically; other inbound port forwards are blocked. For a network_mode: host app, independently managed container, virtualized service, or software you run directly on the box, add a custom port rule with optional source and interface restrictions.',
				},
			],
		},
		{
			title: 'Security & Access',
			entries: [
				{
					term: 'Access Control',
					summary: 'Managing who can log into and administer NASty.',
					detail: 'NASty supports local user accounts for the web UI and SMB shares. Access control settings let you manage passwords, permissions, and authentication methods.',
				},
				{
					term: 'Token',
					summary: 'A credential used for API authentication.',
					detail: 'API tokens let external programs (scripts, CSI drivers, automation tools) authenticate with NASty without using a username and password. Tokens can be created and revoked from the Access Control page.',
				},
				{
					term: 'API',
					summary: 'Application Programming Interface — how software talks to NASty.',
					detail: 'NASty\'s engine exposes a JSON-RPC 2.0 API over WebSocket. Everything the web UI does goes through this API, and you can use it directly for scripting and automation. Connect to ws://<nasty-ip>/ws/api with a valid token.',
				},
				{
					term: 'SSO / OIDC',
					summary: 'Sign in to NASty with an external identity provider.',
					detail: 'OpenID Connect lets you delegate web UI login to a provider like Authentik, Keycloak, Google, or any OIDC-compliant IdP. Users log in once at the provider and are redirected back. Configure under Access Control → Identity Provider; existing local accounts keep working alongside SSO.',
				},
				{
					term: 'Audit Log',
					summary: 'Append-only record of every action operators take on the box.',
					detail: 'Lives at /var/lib/nasty/audit.log (mode 0600) and is mirrored to journald with target "audit", so tampering with the file still leaves a trail. Every state-changing RPC the engine accepts is recorded with the username, client IP, method name, and a safelist-filtered parameter summary (secrets like passwords / API tokens / TLS DNS credentials never make it in). Logged in addition to mutations: every login attempt (success and failure), permission denials, terminal / VM-console / log-stream opens, and unsafe app deploys — anything an auditor would want to reconstruct after the fact. Read it via the audit.list RPC or the Logs page in the WebUI; rotated by logrotate at 10 MB.',
				},
				{
					term: 'WebAuthn / Passkey / Security Key',
					summary: 'A non-password authentication factor — Touch ID, YubiKey, Windows Hello, etc.',
					detail: 'WebAuthn is a browser standard for proving who you are without a password. The "credential" lives on a device you control: a hardware key (YubiKey, Solo 2, Trezor — sometimes called a security key), a platform authenticator (Touch ID on a Mac, Windows Hello on a PC), or a syncable passkey (iCloud Keychain, 1Password, Bitwarden). NASty supports them as a third login backend alongside local password and SSO. Each credential is bound to one origin (a hostname like nasty.local) — moving NASty to a different hostname silently invalidates registered credentials, and IP-based access can never use them by spec. Register and manage your own under Access Control → Tokens & Keys.',
				},
				{
					term: 'TPM2',
					summary: 'A small chip on the motherboard that holds secrets and measurements.',
					detail: 'The Trusted Platform Module v2.0 is a discrete security chip (or firmware-emulated equivalent like Intel PTT, AMD fTPM, swtpm on QEMU) that stores keys in tamper-resistant hardware and can release them only when system state matches what you sealed against. NASty uses it to "seal" the bcachefs encryption key so an encrypted filesystem can auto-unlock at boot without the operator typing a passphrase — but only when the box looks the way it did at seal time. Without a TPM2 chip the auto-unlock can\'t work; manual passphrase entry stays as a fallback.',
				},
				{
					term: 'PCR (Platform Configuration Register)',
					summary: 'TPM-internal registers that record what booted, in a way that can\'t be rewound or faked.',
					detail: 'PCRs are 24 (or 32) hash values inside the TPM that accumulate measurements during boot. Every component — firmware, bootloader, kernel, initrd — gets hashed and "extended" into one of these registers. Once a value is extended you can\'t set it back; the only way for a PCR to read a given value is for the boot chain to produce that exact value organically. NASty can seal encryption keys against selected PCRs so auto-unlock only succeeds when the measured boot state matches.',
				},
			],
		},
		{
			title: 'Backup',
			entries: [
				{
					term: 'Backup Profile',
					summary: 'A reusable definition of what to back up, where, and how often.',
					detail: 'A profile bundles a set of source paths (subvolumes or filesystem dirs), a target (local, S3, SFTP, REST, or Backblaze B2), an encryption password, a schedule, and a retention policy. Backups are deduplicated and incremental — only changed blocks travel over the network.',
				},
				{
					term: 'Retention',
					summary: 'How many snapshots to keep, by age class.',
					detail: 'A retention policy says e.g. "keep the last 7 snapshots, plus 7 daily, 4 weekly, 6 monthly." After every backup, snapshots that don\'t match any class are pruned. Tune this per profile based on how much history you want versus storage cost at the target.',
				},
				{
					term: 'Restore',
					summary: 'Bring data back from a backup — onto the same box, or a brand-new one.',
					detail: 'Pick a snapshot from a backup profile and restore it to a folder on your storage. Restores merge: existing files are only replaced when you explicitly allow overwriting, and nothing else is deleted. Because backup repositories are self-contained, disaster recovery works the same way — on a fresh NASty, add a profile pointing at your existing repository (S3, SFTP, local, …), list its snapshots, and restore.',
				},
			],
		},
		{
			title: 'Which Protocol Should I Use?',
			entries: [
				{
					term: 'I want to share files with Windows PCs',
					summary: 'Use SMB.',
				},
				{
					term: 'I want to share files with Linux servers or containers',
					summary: 'Use NFS.',
				},
				{
					term: 'I want to serve a virtual disk for a VM or database',
					summary: 'Use iSCSI (compatible) or NVMe-oF (fastest).',
				},
				{
					term: 'I want Kubernetes persistent volumes',
					summary: 'Use NFS for ReadWriteMany, iSCSI or NVMe-oF for ReadWriteOnce.',
				},
				{
					term: 'I want to stream media (Plex, Jellyfin)',
					summary: 'Use NFS or SMB — either works, NFS has less overhead.',
				},
				{
					term: 'I want a camera, scanner, or legacy FTP client to upload files',
					summary: 'Use FTP. Prefer SFTP if the client supports it — FTP is unencrypted.',
				},
				{
					term: 'I want encrypted file transfer without SMB',
					summary: 'Use SFTP (port 2022 by default).',
				},
				{
					term: 'I want an S3-compatible endpoint for apps or backup tools',
					summary: 'Use S3. Share names become bucket names (path-style).',
				},
				{
					term: 'I want to give a file to someone without a NASty account',
					summary: 'Create a Guest Share link from the Files page.',
				},
				{
					term: 'I\'m not sure',
					summary: 'Start with SMB — it works with everything.',
				},
			],
		},
	];

	let expandedTerm = $state<string | null>(null);

	function toggle(term: string) {
		expandedTerm = expandedTerm === term ? null : term;
	}
</script>

<div class="space-y-6">
	<div>
		<p class="text-sm text-muted-foreground mt-0.5">Storage terms, protocols, and guidance for getting started with NASty.</p>
	</div>

	{#each sections as section}
		<div>
			<h2 class="mb-3 text-lg font-semibold">{section.title}</h2>
			<div class="space-y-1.5">
				{#each section.entries as entry}
					{@const hasDetail = !!entry.detail}
					<Card class="overflow-hidden">
						<button
							class="w-full text-left px-4 py-3 flex items-start gap-3 {hasDetail ? 'cursor-pointer hover:bg-accent/50' : 'cursor-default'} transition-colors"
							onclick={() => hasDetail && toggle(entry.term)}
						>
							<div class="flex-1 min-w-0">
								<span class="font-medium">{entry.term}</span>
								<span class="ml-2 text-sm text-muted-foreground">{entry.summary}</span>
							</div>
							{#if hasDetail}
								<span class="text-xs text-muted-foreground mt-1 shrink-0">{expandedTerm === entry.term ? '−' : '+'}</span>
							{/if}
						</button>
						{#if expandedTerm === entry.term && entry.detail}
							<div class="border-t border-border bg-secondary/20 px-4 py-3 text-sm leading-relaxed text-muted-foreground">
								{entry.detail}
							</div>
						{/if}
					</Card>
				{/each}
			</div>
		</div>
	{/each}
</div>
