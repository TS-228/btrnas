import { describe, expect, it } from 'vitest';
import { isShareCollection } from './share-events';

describe('isShareCollection', () => {
	it.each(['share.nfs', 'share.smb', 'share.ftp', 'share.sftp', 'share.s3', 'share.iscsi', 'share.nvmeof'])(
		'accepts the engine collection %s',
		(collection) => expect(isShareCollection(collection)).toBe(true),
	);

	it.each(['nfs', 'smb', 'iscsi', 'nvmeof', 'subvolume', undefined])(
		'rejects non-share collection %s',
		(collection) => expect(isShareCollection(collection)).toBe(false),
	);
});
