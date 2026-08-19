const SHARE_COLLECTIONS = new Set([
	'share.nfs',
	'share.smb',
	'share.ftp',
	'share.sftp',
	'share.s3',
	'share.iscsi',
	'share.nvmeof',
]);

export function isShareCollection(collection: unknown): boolean {
	return typeof collection === 'string' && SHARE_COLLECTIONS.has(collection);
}
