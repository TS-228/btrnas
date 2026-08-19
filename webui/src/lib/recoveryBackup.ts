export const CORE_RECOVERY_SOURCES = [
	{ path: '/var/lib/nasty', label: 'NASty settings and service definitions' },
	{ path: '/etc/nasty', label: 'installed system configuration overlays' },
	{ path: '/var/lib/caddy', label: 'TLS certificates and local CA identity' },
	{ path: '/var/lib/systemd/credential.secret', label: 'host key for encrypted credentials' },
] as const;

export const CORE_RECOVERY_PATHS = CORE_RECOVERY_SOURCES.map(source => source.path);

export const RECOVERY_BACKUP_CHANGED_EVENT = 'nasty:recovery-backup-changed';
