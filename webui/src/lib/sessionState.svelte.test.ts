import { beforeEach, describe, expect, test } from 'vitest';
import { resetClient } from './client';
import { rollbackState } from './rollbackState.svelte';
import { terminalStatus } from './terminalStatus.svelte';
import { confirm, confirmState } from './confirm.svelte';
import { confirmDangerous, confirmDangerousState } from './confirm-dangerous.svelte';
import { error as toastError, getToasts } from './toast.svelte';
import { unlockFs, unlockFsState } from './unlock-fs.svelte';
import { rdma } from './sharing/rdma.svelte';
import { nfs } from './sharing/nfs.svelte';
import { smb } from './sharing/smb.svelte';
import { iscsi } from './sharing/iscsi.svelte';
import { nvme } from './sharing/nvmeof.svelte';
import { ftp, sftp, s3 } from './sharing/rclone.svelte';

beforeEach(() => resetClient());

describe('session state reset', () => {
	test('clears loaded stores and plaintext credentials without replacing proxies', () => {
		const identities = { rdma, nfs, smb, iscsi, nvme, ftp, sftp, s3 };
		rdma.loading = true;
		nfs.newHost = '192.0.2.1';
		smb.newName = 'private';
		ftp.newName = 'media';
		ftp.password = 'ftp-secret';
		sftp.newName = 'media';
		sftp.password = 'sftp-secret';
		s3.newName = 'media';
		s3.password = 's3-secret';
		iscsi.addAclPass = 'chap secret';
		nvme.addHostNqn = 'nqn.2026-01.test:host';
		rollbackState.set({ txnId: 'txn-1', revertAtUnix: 1234, riskReason: null });
		terminalStatus.set('connected');

		resetClient();

		expect(rdma).toBe(identities.rdma);
		expect(nfs).toBe(identities.nfs);
		expect(smb).toBe(identities.smb);
		expect(iscsi).toBe(identities.iscsi);
		expect(nvme).toBe(identities.nvme);
		expect(ftp).toBe(identities.ftp);
		expect(sftp).toBe(identities.sftp);
		expect(s3).toBe(identities.s3);
		expect(rdma.loading).toBe(false);
		expect(nfs.newHost).toBe('');
		expect(smb.newName).toBe('');
		expect(ftp.newName).toBe('');
		expect(ftp.password).toBe('');
		expect(sftp.newName).toBe('');
		expect(sftp.password).toBe('');
		expect(s3.newName).toBe('');
		expect(s3.password).toBe('');
		expect(iscsi.addAclPass).toBe('');
		expect(nvme.addHostNqn).toBe('');
		expect(rollbackState.pending).toBeNull();
		expect(terminalStatus.value).toBe('idle');
	});

	test('settles dialogs and clears transient messages', async () => {
		const confirmation = confirm('Delete private data?');
		const dangerous = confirmDangerous('Destroy pool?', 'Type pool name', 'private');
		const unlock = unlockFs('private');
		toastError('private path failed');

		resetClient();

		await expect(confirmation).resolves.toBe(false);
		await expect(dangerous).resolves.toBe(false);
		await expect(unlock).resolves.toBe(false);
		expect(confirmState.open).toBe(false);
		expect(confirmDangerousState.open).toBe(false);
		expect(confirmDangerousState.expectedValue).toBe('');
		expect(unlockFsState.open).toBe(false);
		expect(unlockFsState.fsName).toBe('');
		expect(getToasts()).toEqual([]);
	});
});
