/** FTP / SFTP / S3 share state (rclone serve).
 *
 * Same shape as the SMB module: one `$state` object per protocol so the
 * Sharing page, panel, and wizard share a source of truth. Credentials
 * live at protocol level (rclone serve is single-user), not per share. */

import { getClient, registerSessionReset } from '$lib/client';
import { withToast } from '$lib/toast.svelte';
import { confirm } from '$lib/confirm.svelte';
import type { RcloneProto, RcloneSettings, RcloneShare, Subvolume, ProtocolStatus } from '$lib/types';

export type RcloneSortKey = 'name' | 'path' | 'status';

export interface RcloneState {
	shares: RcloneShare[];
	loading: boolean;
	protocol: ProtocolStatus | null;
	settings: RcloneSettings | null;
	settingsLoading: boolean;
	settingsOpen: boolean;
	passwordRevealed: boolean;
	listen: string;
	port: string;
	username: string;
	password: string;
	anonymous: boolean;
	passiveMin: string;
	passiveMax: string;
	showCreate: boolean;
	subvolumes: Subvolume[];
	newSubvolume: string;
	newName: string;
	newComment: string;
	newReadOnly: boolean;
	expanded: Record<string, boolean>;
	search: string;
	sortKey: RcloneSortKey | null;
	sortDir: 'asc' | 'desc';
}

function initialRcloneState(): RcloneState {
	return {
		shares: [],
		loading: true,
		protocol: null,
		settings: null,
		settingsLoading: false,
		settingsOpen: false,
		passwordRevealed: false,
		listen: '0.0.0.0',
		port: '',
		username: '',
		password: '',
		anonymous: false,
		passiveMin: '30000',
		passiveMax: '30100',
		showCreate: false,
		subvolumes: [],
		newSubvolume: '',
		newName: '',
		newComment: '',
		newReadOnly: false,
		expanded: {},
		search: '',
		sortKey: null,
		sortDir: 'asc',
	};
}

export const ftp = $state(initialRcloneState());
export const sftp = $state(initialRcloneState());
export const s3 = $state(initialRcloneState());

const stores: Record<RcloneProto, RcloneState> = { ftp, sftp, s3 };

export function rcloneStore(kind: RcloneProto): RcloneState {
	return stores[kind];
}

export function rcloneLabel(kind: RcloneProto): string {
	return kind === 'ftp' ? 'FTP' : kind === 'sftp' ? 'SFTP' : 'S3';
}

export function resetRcloneState() {
	Object.assign(ftp, initialRcloneState());
	Object.assign(sftp, initialRcloneState());
	Object.assign(s3, initialRcloneState());
}

registerSessionReset(resetRcloneState);

export function rcloneToggleSort(kind: RcloneProto, key: RcloneSortKey) {
	const s = stores[kind];
	if (s.sortKey === key) {
		s.sortDir = s.sortDir === 'asc' ? 'desc' : 'asc';
	} else {
		s.sortKey = key;
		s.sortDir = 'asc';
	}
}

export async function rcloneRefresh(kind: RcloneProto) {
	await withToast(async () => {
		stores[kind].shares = await getClient().call<RcloneShare[]>(`share.${kind}.list`);
	});
}

export async function rcloneLoadProtocol(kind: RcloneProto) {
	try {
		const all = await getClient().call<ProtocolStatus[]>('service.protocol.list');
		stores[kind].protocol = all.find(p => p.name === kind) ?? null;
	} catch { /* ignore */ }
}

export async function rcloneLoadSettings(kind: RcloneProto) {
	const s = stores[kind];
	s.settingsLoading = true;
	try {
		await withToast(async () => {
			const settings = await getClient().call<RcloneSettings>(`share.${kind}.settings.get`);
			s.settings = settings;
			s.listen = settings.listen;
			s.port = String(settings.port);
			s.username = settings.username;
			s.password = settings.password;
			s.anonymous = settings.anonymous;
			s.passiveMin = String(settings.passive_port_min);
			s.passiveMax = String(settings.passive_port_max);
		});
	} finally {
		s.settingsLoading = false;
	}
}

export async function rcloneSaveSettings(kind: RcloneProto) {
	const s = stores[kind];
	const params: Record<string, unknown> = {
		listen: s.listen.trim() || '0.0.0.0',
		port: parseInt(s.port, 10),
		username: s.username.trim(),
		anonymous: kind === 'ftp' ? s.anonymous : false,
	};
	if (kind === 'ftp') {
		params.passive_port_min = parseInt(s.passiveMin, 10);
		params.passive_port_max = parseInt(s.passiveMax, 10);
	}
	if (s.password && s.password !== s.settings?.password) {
		params.password = s.password;
	}
	const ok = await withToast(
		() => getClient().call(`share.${kind}.settings.update`, params),
		`${rcloneLabel(kind)} server settings saved`,
	);
	if (ok !== undefined) await rcloneLoadSettings(kind);
}

export async function rcloneLoadSubvolumes(kind: RcloneProto) {
	await withToast(async () => {
		const all = await getClient().call<Subvolume[]>('subvolume.list_all');
		stores[kind].subvolumes = all.filter(sv => sv.subvolume_type === 'filesystem');
	});
}

export function rcloneOnSubvolumeSelect(kind: RcloneProto) {
	const s = stores[kind];
	if (s.newSubvolume && !s.newName) {
		const sv = s.subvolumes.find(v => v.path === s.newSubvolume);
		if (sv) s.newName = sv.name.replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^[^A-Za-z0-9]+/, '');
	}
}

export async function rcloneCreate(kind: RcloneProto) {
	const s = stores[kind];
	if (!s.newName || !s.newSubvolume) return;
	const ok = await withToast(
		() => getClient().call(`share.${kind}.create`, {
			name: s.newName,
			path: s.newSubvolume,
			comment: s.newComment || undefined,
			read_only: s.newReadOnly,
		}),
		`${rcloneLabel(kind)} share created`,
	);
	if (ok !== undefined) {
		s.showCreate = false;
		s.newSubvolume = '';
		s.newName = '';
		s.newComment = '';
		s.newReadOnly = false;
		await rcloneRefresh(kind);
	}
}

export async function rcloneToggleEnabled(kind: RcloneProto, share: RcloneShare) {
	await withToast(
		() => getClient().call(`share.${kind}.update`, { id: share.id, enabled: !share.enabled }),
		`Share ${share.enabled ? 'disabled' : 'enabled'}`,
	);
	await rcloneRefresh(kind);
}

export async function rcloneRemove(kind: RcloneProto, id: string) {
	if (!await confirm(`Delete this ${rcloneLabel(kind)} share?`)) return;
	await withToast(() => getClient().call(`share.${kind}.delete`, { id }), `${rcloneLabel(kind)} share deleted`);
	await rcloneRefresh(kind);
}

export async function rcloneToggleReadOnly(kind: RcloneProto, share: RcloneShare) {
	await withToast(
		() => getClient().call(`share.${kind}.update`, { id: share.id, read_only: !share.read_only }),
		'Share updated',
	);
	await rcloneRefresh(kind);
}
