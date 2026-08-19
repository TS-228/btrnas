<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { goto } from '$app/navigation';
	import { getClient } from '$lib/client';
	import {
		BCACHEFS_FS_OVERHEAD,
		estimateUsableBytes,
		formatBytes,
		formatPercent
	} from '$lib/format';
	import { withToast } from '$lib/toast.svelte';

	let pageTab = $state<'manage' | 'diagnostics'>(
		typeof window !== 'undefined' && window.location.hash === '#diagnostics' ? 'diagnostics' : 'manage'
	);
	import { confirm } from '$lib/confirm.svelte';
	import { confirmDangerous } from '$lib/confirm-dangerous.svelte';
	import { summarizeDependents } from '$lib/fs-dependents';
	import type { Filesystem, UnavailableFilesystem, FilesystemDevice, BlockDevice, ScrubStatus, FsckStatus, FsDependents } from '$lib/types';
	import { Button } from '$lib/components/ui/button';
	import SortTh from '$lib/components/SortTh.svelte';
	import { Card, CardContent } from '$lib/components/ui/card';
	import { Badge } from '$lib/components/ui/badge';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';

	let filesystems: Filesystem[] = $state([]);
	let unavailableFilesystems: UnavailableFilesystem[] = $state([]);
	let devices: BlockDevice[] = $state([]);
	/** Per-FS scrub state populated by `refresh()` alongside fs.list, and
	 * re-polled while any scrub is running. Drives the inline chip +
	 * Scrub button on every FS row. */
	let scrubStatuses: Record<string, ScrubStatus> = $state({});
	/** Interval handle for the running-scrub poller. Lazily started
	 * when any FS reports `running`, cleared when none do. */
	let scrubPoll: ReturnType<typeof setInterval> | null = null;
	let wizardStep: 0 | 1 | 2 = $state(0); // 0=hidden, 1=name+devices, 2=review
	let loading = $state(true);

	// Wizard state
	let newName = $state('first');
	let selectedPaths: string[] = $state([]);
	let replicas = $state(1);
	let compression = $state('');
	let compressionLevel = $state('');
	let showPartitions = $state(false);

	let expandedFs: string | null = $state(null);
	let editOptionsFs: string | null = $state(null);
	let editCompression = $state('');
	let editCompressionLevel = $state('');

	let showCreateCommands = $state(false);

	const client = getClient();

	function handleEvent(_: string, params: unknown) {
		const p = params as { collection?: string };
		if (p?.collection === 'filesystem') refresh();
	}


	function updateScrubPolling() {
		const anyRunning = Object.values(scrubStatuses).some((s) => s.running);
		if (anyRunning && !scrubPoll) {
			// 2s while a scrub is in flight so the streamed percent
			// feels live without hammering the engine. The
			// scrub_status RPC is cheap (in-memory hashmap lookup +
			// one pgrep per call); the prior 5s cadence was too slow
			// to make the new percent-chip feel responsive.
			scrubPoll = setInterval(async () => {
				const updates = await Promise.all(
					filesystems
						.filter((fs) => scrubStatuses[fs.name]?.running)
						.map(async (fs) => {
							try {
								const s = await client.call<ScrubStatus>('fs.scrub.status', { name: fs.name });
								return [fs.name, s] as const;
							} catch { return null; }
						})
				);
				const next = { ...scrubStatuses };
				for (const r of updates) if (r) next[r[0]] = r[1];
				scrubStatuses = next;
				updateScrubPolling();
			}, 2000);
		} else if (!anyRunning && scrubPoll) {
			clearInterval(scrubPoll);
			scrubPoll = null;
		}
	}

	async function startScrubInline(fsName: string) {
		await withToast(
			() => client.call('fs.scrub.start', { name: fsName }),
			`Scrub started on "${fsName}"`
		);
		try {
			const s = await client.call<ScrubStatus>('fs.scrub.status', { name: fsName });
			scrubStatuses = { ...scrubStatuses, [fsName]: s };
		} catch { /* poll will catch up */ }
		updateScrubPolling();
	}

	/** Render the scrub state as a short human chip suitable for the
	 * row's descriptor line. Designed for the typical case (last run
	 * was minutes / hours / days ago) — avoids exact timestamps which
	 * are noisy at a glance. Operators wanting precision can click
	 * through to the Diagnostics tab. */
	function scrubChip(s: ScrubStatus | undefined): { label: string; cls: string; title: string } {
		if (!s) return { label: 'scrub: —', cls: 'text-muted-foreground', title: 'No scrub data' };
		if (s.running) {
			// Prefer the parsed scrub percent when we have one;
			// fall back to elapsed time so a tools build that doesn't
			// print percent (or hasn't yet) still shows motion.
			const since = s.started_at ? humanAgo(s.started_at) : 'now';
			const label = s.progress_percent != null
				? `scrubbing ${s.progress_percent.toFixed(s.progress_percent >= 10 ? 0 : 1)}% (${since})`
				: `scrubbing (${since})`;
			return { label, cls: 'text-blue-400', title: 'Scrub in progress' };
		}
		if (!s.last_run_at) {
			return { label: 'never scrubbed', cls: 'text-muted-foreground', title: 'This filesystem has not been scrubbed since the engine started tracking.' };
		}
		const ago = humanAgo(s.last_run_at);
		const outcome = s.last_outcome ?? 'ok';
		const dur = s.last_duration_secs ? humanDuration(s.last_duration_secs) : '';
		const cls =
			outcome === 'ok' ? 'text-green-500' :
			outcome === 'errors' ? 'text-amber-500' :
			outcome === 'cancelled' ? 'text-muted-foreground' :
			'text-red-500';
		const label = outcome === 'ok'
			? `scrubbed ${ago}, ok`
			: outcome === 'errors'
			? `scrubbed ${ago}, errors`
			: outcome === 'cancelled'
			? `scrub cancelled ${ago}`
			: `scrub failed ${ago}`;
		return { label, cls, title: `${dur ? `Took ${dur}. ` : ''}Click Diagnostics for full output.` };
	}

	function humanAgo(unixSecs: number): string {
		const delta = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs);
		if (delta < 60) return `${delta}s ago`;
		if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
		if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
		return `${Math.floor(delta / 86400)}d ago`;
	}

	function humanDuration(secs: number): string {
		if (secs < 60) return `${secs}s`;
		if (secs < 3600) return `${Math.floor(secs / 60)}m`;
		const h = Math.floor(secs / 3600);
		const m = Math.floor((secs % 3600) / 60);
		return m ? `${h}h${m}m` : `${h}h`;
	}

	// ── fsck (#440) — offline check, only valid while unmounted ──
	let fsckStatuses: Record<string, FsckStatus> = $state({});
	let fsckPoll: ReturnType<typeof setInterval> | null = null;
	// Which filesystem's fsck output `<pre>` is expanded.
	let fsckOutputShown = $state<string | null>(null);

	function updateFsckPolling() {
		const anyRunning = Object.values(fsckStatuses).some((s) => s.running);
		if (anyRunning && !fsckPoll) {
			fsckPoll = setInterval(async () => {
				const updates = await Promise.all(
					filesystems
						.filter((fs) => fsckStatuses[fs.name]?.running)
						.map(async (fs) => {
							try {
								const s = await client.call<FsckStatus>('fs.fsck.status', { name: fs.name });
								return [fs.name, s] as const;
							} catch { return null; }
						})
				);
				const next = { ...fsckStatuses };
				for (const r of updates) if (r) next[r[0]] = r[1];
				fsckStatuses = next;
				updateFsckPolling();
			}, 2000);
		} else if (!anyRunning && fsckPoll) {
			clearInterval(fsckPoll);
			fsckPoll = null;
		}
	}

	async function startFsckInline(fsName: string, repair: boolean) {
		if (repair && !await confirm(
			`Repair "${fsName}" with fsck?`,
			`This runs "btrfs check --repair" and will modify the filesystem to correct errors it finds. Run a dry run first if you're unsure. Make sure you have backups of irreplaceable data.`
		)) return;
		const ok = await withToast(
			() => client.call('fs.fsck.start', { name: fsName, repair }),
			`fsck ${repair ? '(repair)' : '(dry run)'} started on "${fsName}"`
		);
		if (ok === undefined) return;
		fsckOutputShown = fsName;
		try {
			const s = await client.call<FsckStatus>('fs.fsck.status', { name: fsName });
			fsckStatuses = { ...fsckStatuses, [fsName]: s };
		} catch { /* poll will catch up */ }
		updateFsckPolling();
	}

	function fsckChip(s: FsckStatus | undefined): { label: string; cls: string } {
		if (!s || (!s.running && !s.last_run_at)) {
			return { label: 'not checked', cls: 'text-muted-foreground' };
		}
		if (s.running) {
			const since = s.started_at ? humanAgo(s.started_at) : 'now';
			const verb = s.repair ? 'repairing' : 'checking';
			const label = s.progress_percent != null
				? `${verb} ${s.progress_percent.toFixed(s.progress_percent >= 10 ? 0 : 1)}% (${since})`
				: `${verb} (${since})`;
			return { label, cls: 'text-blue-400' };
		}
		const ago = humanAgo(s.last_run_at!);
		const dur = s.last_duration_secs ? ` in ${humanDuration(s.last_duration_secs)}` : '';
		const kind = s.last_repair ? 'repair' : 'check';
		const outcome = s.last_outcome ?? 'clean';
		if (outcome === 'clean') return { label: `${kind} ${ago}: clean${dur}`, cls: 'text-green-500' };
		if (outcome === 'errors') return { label: `${kind} ${ago}: errors${dur}`, cls: 'text-amber-500' };
		return { label: `${kind} failed ${ago}`, cls: 'text-red-500' };
	}

	onMount(async () => {
		client.onEvent(handleEvent);
		try {
			const saved = localStorage.getItem('fsDeviceCols');
			if (saved) {
				const parsed = JSON.parse(saved);
				if (Array.isArray(parsed)) visibleDeviceCols = parsed.filter((c) => typeof c === 'string');
		}
		} catch { /* ignore malformed pref */ }
		await refresh();
		loading = false;
		if (typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('create')) {
			openWizard();
		}
	});

	onDestroy(() => {
		client.offEvent(handleEvent);
		if (scrubPoll) { clearInterval(scrubPoll); scrubPoll = null; }
		if (fsckPoll) { clearInterval(fsckPoll); fsckPoll = null; }
	});

	async function refresh() {
		const [liveResult, unavailableResult, deviceResult] = await Promise.all([
			withToast(() => client.call<Filesystem[]>('fs.list')),
			withToast(() => client.call<UnavailableFilesystem[]>('fs.unavailable.list')),
			withToast(() => client.call<BlockDevice[]>('device.list')),
		]);
		if (liveResult) filesystems = liveResult;
		if (unavailableResult) unavailableFilesystems = unavailableResult;
		if (deviceResult) devices = deviceResult;

		// Pull scrub state for every FS. Errors surface as "Never scrubbed".
		const scrubResults = await Promise.all(
			filesystems.map(async (fs) => {
				try {
					const s = await client.call<ScrubStatus>('fs.scrub.status', { name: fs.name });
					return [fs.name, s] as const;
				} catch { return null; }
			})
		);
		const nextScrub: Record<string, ScrubStatus> = {};
		for (const r of scrubResults) if (r) nextScrub[r[0]] = r[1];
		scrubStatuses = nextScrub;
		updateScrubPolling();

		// fsck state is only meaningful for unmounted filesystems.
		const fsckResults = await Promise.all(
			filesystems.filter((fs) => !fs.mounted).map(async (fs) => {
				try {
					const s = await client.call<FsckStatus>('fs.fsck.status', { name: fs.name });
					return [fs.name, s] as const;
				} catch { return null; }
			})
		);
		const nextFsck: Record<string, FsckStatus> = {};
		for (const r of fsckResults) if (r) nextFsck[r[0]] = r[1];
		fsckStatuses = nextFsck;
		updateFsckPolling();
	}

	function buildFormatCommand(): string[] {
		const args = ['mkfs.btrfs', '-f', '-L', newName];
		if (replicas >= 2) {
			args.push('-d', 'raid1', '-m', 'raid1');
		} else if (selectedPaths.length === 1) {
			args.push('-d', 'single', '-m', 'dup');
		} else {
			args.push('-d', 'single', '-m', 'raid1');
		}
		args.push(...selectedPaths);
		return args;
	}

	function buildMountCommand(): string[] {
		const deviceArg = selectedPaths[0] ?? '';
		return ['mount', '-t', 'btrfs', '-o', 'prjquota', deviceArg, `/fs/${newName}`];
	}

	function formatCommandLines(args: string[]): string {
		if (args.length <= 6) return args.join(' ');
		const parts: string[] = [args[0]];
		for (let i = 1; i < args.length; i++) {
			parts.push('  ' + args[i]);
		}
		return parts.join(' \\\n');
	}

	async function createFs() {
		if (!newName || selectedPaths.length === 0) return;
		if (replicas === 2 && selectedPaths.length < 2) return;
		const ok = await withToast(
			() => client.call('fs.create', {
				name: newName,
				devices: selectedPaths.map(path => ({ path })),
				replicas,
				compression: combineCompression(compression, compressionLevel) || undefined,
			}),
			`Filesystem "${newName}" created`
		);
		if (ok !== undefined) {
			wizardStep = 0;
			newName = 'first';
			selectedPaths = [];
			replicas = 1;
			compression = '';
			compressionLevel = '';
			await refresh();
		}
	}

	function openWizard() {
		newName = 'first';
		selectedPaths = [];
		replicas = 1;
		compression = '';
		compressionLevel = '';
		const hasFullDisks = devices.some(d => !d.in_use && d.dev_type !== 'part');
		showPartitions = !hasFullDisks;
		showCreateCommands = false;
		wizardStep = 1;
	}

	function wizardNext() {
		wizardStep = (wizardStep + 1) as 1 | 2;
	}

	async function destroyFs(name: string) {
		if (!await confirmDangerous(
			`Destroy Filesystem "${name}"`,
			`This will unmount the filesystem and wipe all device superblocks. Type the filesystem name to confirm.`,
			name,
		)) return;
		await withToast(
			() => client.call('fs.destroy', { name, confirm_name: name }),
			`Filesystem "${name}" destroyed`
		);
		await refresh();
	}

	async function forgetFs(fs: UnavailableFilesystem) {
		let dependents: string | null = null;
		try {
			dependents = summarizeDependents(
				await client.call<FsDependents>('fs.dependents', { name: fs.name }),
			);
		} catch { /* The backend still validates dependents before forgetting. */ }

		let message = `This removes only NASty's saved configuration for "${fs.name}" and stops its auto-mount attempts and alerts. It does not erase or modify any disks. If the member disks are reconnected, they retain their btrfs data.`;
		if (dependents) {
			message += `\n\nNASty reports these dependents; the backend will block forgetting while they remain:\n\n${dependents}`;
		}
		message += `\n\nType the filesystem name to confirm.`;

		if (!await confirmDangerous(
			`Forget Filesystem "${fs.name}"`,
			message,
			fs.name,
			{ confirmLabel: 'Forget filesystem' },
		)) return;

		await withToast(
			() => client.call('fs.forget', {
				name: fs.name,
				expected_uuid: fs.uuid,
				confirm_name: fs.name,
			}),
			`Filesystem "${fs.name}" forgotten`,
		);
		await refresh();
	}

	let mountingFs = $state<string | null>(null);
	// Which filesystem's raw mount-failure output (#451 banner) is expanded.
	let rawErrShown = $state<string | null>(null);

	// Bring a pool up degraded (without its missing member), from the
	// mount-failure banner. Persists the degraded flag for subsequent boots.
	async function mountDegraded(fs: Filesystem) {
		if (!await confirm(
			`Mount "${fs.name}" degraded?`,
			`The pool will mount without its missing member device(s). This is safe only if enough replicas remain; writes made while degraded reduce redundancy. The pool will keep mounting degraded until you restore the device and turn it off in Options.`
		)) return;
		mountingFs = fs.name;
		await withToast(
			() => client.call('fs.mount', { name: fs.name, degraded: true }, 120000),
			`Filesystem "${fs.name}" mounted (degraded)`
		);
		mountingFs = null;
		await refresh();
	}

	async function toggleMount(fs: Filesystem) {
		if (fs.mounted) {
			if (!await confirm(`Unmount Filesystem "${fs.name}"`, `Any active NFS, SMB, FTP, SFTP, S3, iSCSI, and NVMe-oF shares on this filesystem will be stopped first.`)) return;
		}
		const action = fs.mounted ? 'unmount' : 'mount';
		mountingFs = fs.name;
		await withToast(
			() => fs.mounted
				? client.call('fs.unmount', { name: fs.name }, 120000)
				: client.call('fs.mount', { name: fs.name }, 120000),
			`Filesystem "${fs.name}" ${action}ed`
		);
		mountingFs = null;
		await refresh();
	}

	function openEditOptions(fs: Filesystem) {
		if (editOptionsFs === fs.name) {
			editOptionsFs = null;
			return;
		}
		editOptionsFs = fs.name;
		({ algo: editCompression, level: editCompressionLevel } = splitCompression(fs.options.compression));
	}

	async function saveOptions(fsName: string) {
		await withToast(
			() => client.call('fs.options.update', {
				name: fsName,
				compression: combineCompression(editCompression, editCompressionLevel) || 'none',
			}, 60_000),
			`Options updated for "${fsName}"`
		);
		editOptionsFs = null;
		await refresh();
	}

	function toggleDevice(path: string) {
		if (selectedPaths.includes(path)) {
			selectedPaths = selectedPaths.filter(p => p !== path);
		} else {
			selectedPaths = [...selectedPaths, path];
		}
		if (selectedPaths.length <= 1) replicas = 1;
		else if (replicas > 2) replicas = 2;
	}

	function selectedDeviceObjects(): BlockDevice[] {
		return selectedPaths
			.map(p => devices.find(d => d.path === p))
			.filter(Boolean) as BlockDevice[];
	}

	function availableDevices(): BlockDevice[] {
		return devices.filter(d => !d.in_use && (showPartitions || d.dev_type !== 'part'));
	}

	// ── Compression algorithm:level helpers ──
	// Only zstd and gzip have a level knob; lz4 doesn't.
	function compressionMaxLevel(algo: string): number | null {
		return algo === 'zstd' ? 22 : algo === 'gzip' ? 9 : null;
	}
	/** Split a stored spec ("zstd:15") into its algorithm and level. */
	function splitCompression(spec: string | null | undefined): { algo: string; level: string } {
		if (!spec || spec === 'none') return { algo: '', level: '' };
		const [algo, level] = spec.split(':');
		return { algo, level: level ?? '' };
	}
	/** Combine an algorithm + level back into a spec the engine accepts. */
	function combineCompression(algo: string, level: string): string {
		if (!algo) return '';
		return level && compressionMaxLevel(algo) !== null ? `${algo}:${level}` : algo;
	}

	function deviceBlock(path: string): BlockDevice | undefined {
		return devices.find((d) => d.path === path);
	}
	/** Total IO errors across read/write/checksum, or null if unknown (unmounted). */
	function devErrorTotal(dev: FilesystemDevice): number | null {
		if (dev.read_errors == null && dev.write_errors == null && dev.checksum_errors == null)
			return null;
		return (dev.read_errors ?? 0) + (dev.write_errors ?? 0) + (dev.checksum_errors ?? 0);
	}

	function stateColor(state: string | null): string {
		switch (state) {
			case 'rw': return 'bg-green-950 text-green-400';
			case 'ro': return 'bg-blue-950 text-blue-400';
			case 'failed': return 'bg-red-950 text-red-400';
			case 'spare': return 'bg-amber-950 text-amber-400';
			default: return 'bg-secondary text-muted-foreground';
		}
	}

	type DevSortKey =
		| 'path' | 'label' | 'state' | 'slot' | 'uuid' | 'size' | 'type'
		| 'rotational' | 'model' | 'serial' | 'clean'
		| 'read_err' | 'write_err' | 'csum_err' | 'data_allowed' | 'has_data';
	let devSortKey = $state<DevSortKey>('slot');
	let devSortDir = $state<'asc' | 'desc'>('asc');
	function toggleDevSort(key: DevSortKey) {
		if (devSortKey === key) devSortDir = devSortDir === 'asc' ? 'desc' : 'asc';
		else { devSortKey = key; devSortDir = 'asc'; }
	}
	function devSortVal(dev: FilesystemDevice, key: DevSortKey): string | number {
		switch (key) {
			case 'path': return dev.path ?? '';
			case 'label': return dev.label ?? '';
			case 'state': return dev.missing ? 'missing' : (dev.state ?? '');
			case 'slot': return dev.member_index ?? -1;
			case 'uuid': return dev.uuid ?? '';
			case 'size': return deviceBlock(dev.path)?.size_bytes ?? -1;
			case 'type': return deviceBlock(dev.path)?.device_class ?? '';
			case 'rotational': return dev.rotational == null ? -1 : (dev.rotational ? 1 : 0);
			case 'model': return deviceBlock(dev.path)?.model ?? '';
			case 'serial': return deviceBlock(dev.path)?.serial ?? '';
			case 'clean': return devErrorTotal(dev) ?? -1;
			case 'read_err': return dev.read_errors ?? -1;
			case 'write_err': return dev.write_errors ?? -1;
			case 'csum_err': return dev.checksum_errors ?? -1;
			case 'data_allowed': return dev.data_allowed ?? '';
			case 'has_data': return dev.has_data ?? '';
		}
	}
	function sortedDevices(devs: FilesystemDevice[]): FilesystemDevice[] {
		const sign = devSortDir === 'asc' ? 1 : -1;
		return [...devs].sort((a, b) => {
			const av = devSortVal(a, devSortKey);
			const bv = devSortVal(b, devSortKey);
			let cmp =
				typeof av === 'number' && typeof bv === 'number'
					? av - bv
					: String(av).localeCompare(String(bv), undefined, { numeric: true });
			if (cmp === 0) cmp = (a.path ?? '').localeCompare(b.path ?? '', undefined, { numeric: true });
			return sign * cmp;
		});
	}

	// ── Filesystem device table: user-selectable columns ──
	// Device / Label / State are always shown; these are optional and
	// persisted per-browser. Size/Type/Model/Serial join device.list;
	// Clean + error counts come from per-device IO error counters.
	const DEVICE_COLUMNS = [
		{ id: 'slot', label: 'Slot' },
		{ id: 'uuid', label: 'UUID' },
		{ id: 'size', label: 'Size' },
		{ id: 'type', label: 'Type' },
		{ id: 'rotational', label: 'Rotational' },
		{ id: 'model', label: 'Model' },
		{ id: 'serial', label: 'Serial' },
		{ id: 'clean', label: 'Clean' },
		{ id: 'read_err', label: 'Read err' },
		{ id: 'write_err', label: 'Write err' },
		{ id: 'csum_err', label: 'Cksum err' },
		{ id: 'data_allowed', label: 'Data allowed' },
		{ id: 'has_data', label: 'Has data' }
	] as const;
	const DEFAULT_DEVICE_COLS = ['slot', 'size', 'type', 'clean', 'has_data'];
	let visibleDeviceCols = $state<string[]>([...DEFAULT_DEVICE_COLS]);

	function colOn(id: string): boolean {
		return visibleDeviceCols.includes(id);
	}
	function toggleCol(id: string) {
		visibleDeviceCols = colOn(id)
			? visibleDeviceCols.filter((c) => c !== id)
			: [...visibleDeviceCols, id];
		try { localStorage.setItem('fsDeviceCols', JSON.stringify(visibleDeviceCols)); } catch { /* ignore */ }
	}

	function classColor(cls: string): string {
		switch (cls) {
			case 'nvme': return 'bg-violet-950 text-violet-300';
			case 'ssd':  return 'bg-blue-950 text-blue-300';
			case 'mmc':  return 'bg-amber-950 text-amber-300';
			case 'hdd':  return 'bg-emerald-950 text-emerald-300';
			default:     return 'bg-secondary text-muted-foreground';
		}
	}
</script>


<!-- Page-level tabs -->
<div class="mb-4 flex items-center gap-4 border-b border-border">
	<button
		onclick={() => { pageTab = 'manage'; history.replaceState(null, '', '#manage'); }}
		class="px-3 py-2 text-sm font-medium transition-colors border-b-2 -mb-px
			{pageTab === 'manage' ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'}"
	>Manage</button>
	<button
		onclick={() => { pageTab = 'diagnostics'; history.replaceState(null, '', '#diagnostics'); }}
		class="px-3 py-2 text-sm font-medium transition-colors border-b-2 -mb-px
			{pageTab === 'diagnostics' ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'}"
	>Diagnostics</button>
</div>

{#if pageTab === 'diagnostics'}
	{#await import('$lib/components/BcachefsDiagnostics.svelte')}
		<p class="p-6 text-sm text-muted-foreground">Loading diagnostics…</p>
	{:then module}
		<module.default />
	{:catch}
		<!-- A failed dynamic import is almost always a stale app shell
		     after an in-place upgrade: the cached page references asset
		     filenames that no longer exist, so the chunk 404s. Without
		     this catch the panel silently rendered blank (#503). -->
		<div class="rounded-lg border border-border bg-card p-6 text-sm">
			<p class="font-medium">Couldn't load the diagnostics panel.</p>
			<p class="mt-1 text-muted-foreground">NASty was likely updated in the background — reload to pick up the new version.</p>
			<Button size="sm" class="mt-3" onclick={() => location.reload()}>Reload</Button>
		</div>
	{/await}
{:else}

<div class="mb-4">
	<Button size="sm" onclick={() => wizardStep === 0 ? openWizard() : (wizardStep = 0)}>
		{wizardStep !== 0 ? 'Cancel' : 'Create Filesystem'}
	</Button>
</div>

{#if wizardStep !== 0}
	<Card class="mb-6 max-w-4xl">
		<CardContent class="pt-6">
			<!-- Step indicator -->
			<div class="mb-6 flex items-center gap-0">
				{#each [['1', 'Devices'], ['2', 'Review']] as [num, label], i}
					<div class="flex items-center">
						<div class="flex items-center gap-2">
							<div class="flex h-6 w-6 items-center justify-center rounded-full text-xs font-semibold
								{wizardStep > i + 1 ? 'bg-primary text-primary-foreground' :
								 wizardStep === i + 1 ? 'bg-primary text-primary-foreground' :
								 'bg-secondary text-muted-foreground'}">
								{num}
							</div>
							<span class="text-xs {wizardStep === i + 1 ? 'text-foreground font-medium' : 'text-muted-foreground'}">{label}</span>
						</div>
						{#if i < 1}
							<div class="mx-3 h-px w-8 bg-border"></div>
						{/if}
					</div>
				{/each}
			</div>

			<!-- Step 1: Name + Devices -->
			{#if wizardStep === 1}
				<div class="mb-4">
					<Label for="fs-name">Filesystem Name</Label>
					<Input id="fs-name" bind:value={newName} class="mt-1 max-w-xs" />
				</div>
				<div class="mb-4">
					<div class="mb-2 flex items-center justify-between gap-3">
						<Label>Select Devices</Label>
						<div class="flex items-center gap-4">
							<div class="flex items-center gap-2 text-xs text-muted-foreground">
								<span>Need more devices?</span>
								<Button size="xs" onclick={() => goto('/disks')}>Disks</Button>
							</div>
							<label class="flex cursor-pointer items-center gap-1.5 text-xs text-muted-foreground">
								<input type="checkbox" bind:checked={showPartitions} class="h-3.5 w-3.5" />
								Show partitions
							</label>
						</div>
					</div>
					{#if availableDevices().length === 0}
						<p class="text-sm text-muted-foreground">No available devices. Reclaim disks held by old RAID/LVM on the Disks page (Wipe/Reclaim), then try again.</p>
						<Button size="sm" class="mt-2" onclick={() => goto('/disks')}>Disks</Button>
					{:else}
						<div class="space-y-1.5">
							{#each availableDevices() as dev}
								{#if dev.dev_type === 'free'}
									{@const diskPath = dev.path.replace(':free', '')}
									{@const existingParts = devices.filter(d => d.dev_type === 'part' && d.path.startsWith(diskPath))}
									<div class="rounded-lg border border-border overflow-hidden">
										<label class="flex cursor-pointer flex-col gap-1 px-3 py-2 text-sm
											{selectedPaths.includes(dev.path) ? 'border-primary bg-primary/5' : 'hover:bg-secondary/50'}">
											<div class="flex items-center gap-3">
												<input type="checkbox" checked={selectedPaths.includes(dev.path)}
													onchange={() => toggleDevice(dev.path)} class="h-4 w-4 shrink-0" />
												<span class="font-mono text-xs shrink-0">{diskPath}</span>
												<span class="rounded bg-green-900 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-green-300">free space</span>
												<span class="text-muted-foreground">{formatBytes(dev.size_bytes)}</span>
												<span class="text-xs text-muted-foreground">(new partition will be created)</span>
											</div>
											{#if dev.model || dev.vendor || dev.transport || dev.serial}
												<div class="ml-7 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground/80">
													{#if dev.model}<span>{dev.model}</span>{/if}
													{#if dev.vendor && dev.vendor !== dev.model}<span class="opacity-70">{dev.vendor}</span>{/if}
													{#if dev.transport}<span class="uppercase opacity-70">{dev.transport}</span>{/if}
													{#if dev.serial}<span class="font-mono opacity-70">SN: {dev.serial}</span>{/if}
												</div>
											{/if}
										</label>
										{#if existingParts.length > 0}
											<div class="border-t border-border bg-muted/20 px-3 py-1.5">
												{#each existingParts as part}
													<div class="flex items-center gap-2 text-xs text-muted-foreground/60 py-0.5">
														<span class="font-mono">{part.path}</span>
														<span>{formatBytes(part.size_bytes)}</span>
														{#if part.mount_point}<span>mounted at {part.mount_point}</span>{/if}
														{#if part.fs_type}<span class="font-mono">{part.fs_type}</span>{/if}
														<span class="italic">not touched</span>
													</div>
												{/each}
											</div>
										{/if}
									</div>
								{:else}
									<label class="flex cursor-pointer flex-col gap-1 rounded-lg border border-border px-3 py-2 text-sm
										{selectedPaths.includes(dev.path) ? 'border-primary bg-primary/5' : 'hover:bg-secondary/50'}">
										<div class="flex items-center gap-3">
											<input type="checkbox" checked={selectedPaths.includes(dev.path)}
												onchange={() => toggleDevice(dev.path)} class="h-4 w-4 shrink-0" />
											<span class="font-mono text-xs shrink-0">{dev.path}</span>
											<span class="rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase {classColor(dev.device_class)}">
												{dev.device_class}
											</span>
											<span class="text-muted-foreground">{formatBytes(dev.size_bytes)}</span>
											{#if dev.fs_type}
												<span class="rounded border border-amber-700 px-1.5 py-0.5 text-[10px] text-amber-400">has signatures · wipe first</span>
											{/if}
										</div>
										{#if dev.model || dev.vendor || dev.transport || dev.serial}
											<div class="ml-7 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground/80">
												{#if dev.model}<span>{dev.model}</span>{/if}
												{#if dev.vendor && dev.vendor !== dev.model}<span class="opacity-70">{dev.vendor}</span>{/if}
												{#if dev.transport}<span class="uppercase opacity-70">{dev.transport}</span>{/if}
												{#if dev.serial}<span class="font-mono opacity-70">SN: {dev.serial}</span>{/if}
											</div>
										{/if}
									</label>
								{/if}
							{/each}
						</div>
					{/if}
				</div>
				<div class="flex gap-2">
					<Button size="sm" onclick={wizardNext} disabled={!newName || selectedPaths.length === 0}>
						Next: Review →
					</Button>
				</div>

			<!-- Step 2: Review + Options -->
			{:else if wizardStep === 2}
				<div class="mb-5 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
					<span class="text-muted-foreground">Name</span>
					<span class="font-mono">{newName}</span>
					<span class="text-muted-foreground">Devices</span>
					<div class="flex flex-wrap gap-1.5">
						{#each selectedDeviceObjects() as dev}
							<span class="flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-xs">
								{#if dev.dev_type === 'free'}
									<span class="rounded bg-green-900 px-1 py-0.5 text-[10px] font-semibold uppercase text-green-300">free</span>
									<span class="font-mono">{dev.path.replace(':free', '')} (new partition)</span>
								{:else}
									<span class="rounded px-1 py-0.5 text-[10px] font-semibold uppercase {classColor(dev.device_class)}">{dev.device_class}</span>
									<span class="font-mono">{dev.path}</span>
								{/if}
							</span>
						{/each}
					</div>
				</div>

				<div class="mb-5 grid grid-cols-2 gap-4">
					<div>
						<Label for="replicas">Data profile</Label>
						<select id="replicas" bind:value={replicas} disabled={selectedPaths.length <= 1}
							class="mt-1 h-9 w-full rounded-md border border-input bg-transparent px-3 text-sm">
							<option value={1}>single (no redundancy)</option>
							<option value={2}>raid1 (mirrored)</option>
						</select>
						{#if selectedPaths.length <= 1}
							<span class="text-xs text-muted-foreground">raid1 requires multiple devices</span>
						{/if}
					</div>
					<div>
						<Label for="compression">Compression</Label>
						<div class="mt-1 flex gap-2">
							<select id="compression" bind:value={compression}
								onchange={() => compressionLevel = ''}
								class="h-9 flex-1 rounded-md border border-input bg-transparent px-3 text-sm">
								<option value="">None</option>
								<option value="lz4">LZ4</option>
								<option value="zstd">Zstd</option>
								<option value="gzip">Gzip</option>
							</select>
							{#if compressionMaxLevel(compression) !== null}
								<input type="number" bind:value={compressionLevel}
									min="1" max={compressionMaxLevel(compression)}
									placeholder="level"
									title="Optional {compression} level (1–{compressionMaxLevel(compression)}). Leave blank for the default."
									class="h-9 w-24 rounded-md border border-input bg-transparent px-3 text-sm" />
							{/if}
						</div>
					</div>
				</div>

				{#if selectedPaths.length > 0}
					{@const rawTotal = selectedDeviceObjects().reduce(
						(sum, d) => sum + d.size_bytes,
						0
					)}
					{@const fsCapacity = Math.floor(rawTotal * BCACHEFS_FS_OVERHEAD)}
					{@const usable = estimateUsableBytes(
						fsCapacity,
						selectedPaths.length,
						replicas,
						false
					)}
					<div class="mb-5 rounded-lg border border-border bg-secondary/20 p-4 text-sm">
						<div class="mb-2 flex items-center gap-2">
							<span class="font-medium">Storage estimate</span>
							<span class="text-xs text-muted-foreground">approximate</span>
						</div>
						<div class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
							<span class="text-muted-foreground">Raw selected</span>
							<span class="font-mono">{formatBytes(rawTotal)} across {selectedPaths.length} device{selectedPaths.length === 1 ? '' : 's'}</span>
							<span class="text-muted-foreground">After filesystem overhead</span>
							<span class="font-mono">~{formatBytes(fsCapacity)}</span>
							<span class="text-muted-foreground">Layout</span>
							<span>
								{#if replicas === 1}
									No redundancy (single)
								{:else}
									raid1 mirror
								{/if}
							</span>
							<span class="text-muted-foreground">Estimated usable</span>
							<span class="font-mono font-semibold">~{formatBytes(usable)}</span>
						</div>
						<p class="mt-2 text-[11px] text-muted-foreground/80">
							Rough estimate for a fresh btrfs filesystem; actual usable space varies with metadata and profile.
						</p>
					</div>
				{/if}

				<div class="mb-5">
					<button
						type="button"
						onclick={() => showCreateCommands = !showCreateCommands}
						class="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground"
					>
						<span class="inline-block w-3 text-xs">{showCreateCommands ? '▾' : '▸'}</span>
						Show format / mount commands
					</button>
					{#if showCreateCommands}
						<pre class="mt-2 rounded-md border border-border bg-black/40 p-3 text-xs font-mono text-muted-foreground overflow-x-auto whitespace-pre-wrap">{formatCommandLines(buildFormatCommand())}

{buildMountCommand().join(' ')}</pre>
					{/if}
				</div>

				<div class="flex gap-2">
					<Button variant="secondary" size="sm" onclick={() => wizardStep = 1}>← Back</Button>
					<Button size="sm" onclick={createFs} disabled={replicas === 2 && selectedPaths.length < 2}>Create Filesystem</Button>
				</div>
			{/if}
		</CardContent>
	</Card>
{/if}

{#if loading}
	<p class="text-muted-foreground">Loading...</p>
{:else if filesystems.length === 0 && unavailableFilesystems.length === 0}
	<div class="flex flex-col items-center justify-center py-12 text-center">
		<p class="text-muted-foreground">No filesystems configured yet.</p>
		<p class="mt-1 text-sm text-muted-foreground">Use the <strong>Create Filesystem</strong> button above to get started.</p>
	</div>
{:else}
	{#each unavailableFilesystems as fs (fs.uuid)}
		<Card class="mb-4 border-destructive/40 bg-destructive/[0.03]">
			<CardContent class="pt-4">
				<div class="flex flex-wrap items-start justify-between gap-3">
					<div class="min-w-0">
						<div class="flex flex-wrap items-center gap-2">
							<strong class="text-lg">{fs.name}</strong>
							<Badge variant="destructive">Unavailable</Badge>
						</div>
						<p class="mt-1 text-sm text-muted-foreground">No member of this saved filesystem is currently discoverable.</p>
					</div>
					<Button variant="destructive" size="xs" onclick={() => forgetFs(fs)}>Forget</Button>
				</div>

				<dl class="mt-3 grid gap-x-6 gap-y-3 text-sm sm:grid-cols-2">
					<div class="min-w-0 sm:col-span-2">
						<dt class="text-xs font-medium uppercase tracking-wide text-muted-foreground">UUID</dt>
						<dd class="mt-0.5 break-all font-mono text-xs">{fs.uuid}</dd>
					</div>
					<div class="min-w-0">
						<dt class="text-xs font-medium uppercase tracking-wide text-muted-foreground">Last-known members</dt>
						<dd class="mt-1">
							{#if fs.devices.length}
								<ul class="space-y-0.5">
									{#each fs.devices as path}
										<li class="break-all font-mono text-xs">{path}</li>
									{/each}
								</ul>
							{:else}
								<span class="text-xs text-muted-foreground">No paths recorded</span>
							{/if}
						</dd>
					</div>
					<div>
						<dt class="text-xs font-medium uppercase tracking-wide text-muted-foreground">Auto-mount configured</dt>
						<dd class="mt-0.5">{fs.auto_mount ? 'Yes' : 'No'}</dd>
					</div>
				</dl>

				{#if fs.last_mount_error?.message}
					<div class="mt-3 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm">
						<span class="font-medium text-destructive">Last mount failure:</span>
						<span class="ml-1">{fs.last_mount_error.message}</span>
					</div>
				{/if}
			</CardContent>
		</Card>
	{/each}

	{#each filesystems as fs (fs.uuid)}
		<Card class="mb-4">
			<CardContent class="pt-4">
				<div class="flex flex-wrap items-center justify-between gap-4">
					<div class="flex cursor-pointer items-center gap-3" role="button" tabindex="0"
						onclick={() => expandedFs = expandedFs === fs.name ? null : fs.name}
						onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') expandedFs = expandedFs === fs.name ? null : fs.name; }}>
						<strong class="text-lg">{fs.name}</strong>
						<Badge variant={fs.mounted ? 'default' : 'destructive'}>
							{fs.mounted ? 'Mounted' : 'Unmounted'}
						</Badge>
						{#if fs.mounted && fs.mount_point}
							<span class="font-mono text-xs text-muted-foreground">{fs.mount_point}</span>
						{/if}
					</div>
					<div class="flex gap-2">
						<Button variant="secondary" size="xs" onclick={() => expandedFs = expandedFs === fs.name ? null : fs.name}>
							{expandedFs === fs.name ? 'Hide Details' : 'Details'}
						</Button>
						{#if fs.mounted}
							<Button variant="secondary" size="xs" onclick={() => openEditOptions(fs)}>
								{editOptionsFs === fs.name ? 'Hide Options' : 'Options'}
							</Button>
						{/if}
						<Button variant="secondary" size="xs" onclick={() => toggleMount(fs)}
							disabled={mountingFs === fs.name}>
							{mountingFs === fs.name ? (fs.mounted ? 'Unmounting...' : 'Mounting...') : (fs.mounted ? 'Unmount' : 'Mount')}
						</Button>
						{#if fs.mounted}
							{@const sc = scrubStatuses[fs.name]}
							<Button
								variant="secondary"
								size="xs"
								onclick={() => startScrubInline(fs.name)}
								disabled={sc?.running}
								title={sc?.running
									? `Scrub in progress${sc.started_at ? ` (started ${humanAgo(sc.started_at)})` : ''}.`
									: 'Run a full data scrub on this filesystem. Takes hours on multi-TB pools; runs in the background.'}
							>
								{sc?.running ? 'Scrubbing…' : 'Scrub'}
							</Button>
						{/if}
						<Button variant="destructive" size="xs" onclick={() => destroyFs(fs.name)}>Destroy</Button>
					</div>
				</div>

				{#if !fs.mounted && fs.last_mount_error}
					{@const e = fs.last_mount_error}
					<div class="mt-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm">
						<p class="font-medium text-destructive">Last mount failed</p>
						<p class="mt-0.5 text-foreground">{e.message}</p>
						{#if e.missing_devices.length}
							<ul class="mt-1.5 list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
								{#each e.missing_devices as d}
									<li>
										<span class="font-mono">{d.path}</span>{#if d.member_index != null} · member {d.member_index}{/if}{#if d.label} · <span class="font-mono">{d.label}</span>{/if}
									</li>
								{/each}
							</ul>
						{/if}
						<div class="mt-2 flex flex-wrap items-center gap-2">
							{#if e.reason === 'missing_device'}
								<Button variant="default" size="xs" onclick={() => mountDegraded(fs)}
									disabled={mountingFs === fs.name}>
									{mountingFs === fs.name ? 'Mounting…' : 'Mount degraded'}
								</Button>
							{/if}
							{#if e.reason === 'needs_check'}
								<Button variant="default" size="xs" onclick={() => startFsckInline(fs.name, false)}
									disabled={fsckStatuses[fs.name]?.running}>
									{fsckStatuses[fs.name]?.running ? 'Checking…' : 'Run check (fsck)'}
								</Button>
							{/if}
							<button type="button" class="text-xs text-muted-foreground underline underline-offset-2"
								onclick={() => rawErrShown = rawErrShown === fs.name ? null : fs.name}>
								{rawErrShown === fs.name ? 'Hide mount output' : 'Show mount output'}
							</button>
							<span class="ml-auto text-[0.65rem] text-muted-foreground">attempted {humanAgo(e.attempted_at)}</span>
						</div>
						{#if rawErrShown === fs.name}
							<pre class="mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 text-[0.7rem] font-mono">{e.raw}</pre>
						{/if}
					</div>
				{/if}

				{#if !fs.mounted}
					{@const fsck = fsckStatuses[fs.name]}
					<div class="mt-3 rounded-md border border-border p-3">
						<div class="flex flex-wrap items-center gap-2">
							<span class="text-xs font-medium">Filesystem check (fsck)</span>
							<span class="text-xs {fsckChip(fsck).cls}">· {fsckChip(fsck).label}</span>
							{#if fsck?.running && fsck.progress_percent != null}
								<div class="ml-1 h-1.5 w-24 overflow-hidden rounded-full bg-secondary">
									<div class="h-full rounded-full bg-blue-400" style="width: {fsck.progress_percent}%"></div>
								</div>
							{/if}
							<div class="ml-auto flex gap-2">
								<Button variant="secondary" size="xs"
									onclick={() => startFsckInline(fs.name, false)}
									disabled={fsck?.running}
									title="Read-only check (btrfs check) — reports problems without changing anything.">
									{fsck?.running && !fsck.repair ? 'Checking…' : 'Dry run'}
								</Button>
								<Button variant="secondary" size="xs"
									onclick={() => startFsckInline(fs.name, true)}
									disabled={fsck?.running}
									title="Check and repair (btrfs check --repair) — modifies the filesystem to correct errors.">
									{fsck?.running && fsck.repair ? 'Repairing…' : 'Run & repair'}
								</Button>
							</div>
						</div>
						{#if fsck?.last_output}
							<button type="button" class="mt-2 text-xs text-muted-foreground underline underline-offset-2"
								onclick={() => fsckOutputShown = fsckOutputShown === fs.name ? null : fs.name}>
								{fsckOutputShown === fs.name ? 'Hide output' : 'Show output'}
							</button>
							{#if fsckOutputShown === fs.name}
								<pre class="mt-2 max-h-60 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 text-[0.7rem] font-mono">{fsck.last_output}</pre>
							{/if}
						{/if}
					</div>
				{/if}

				{#if fs.total_bytes > 0}
					{@const fsReplicas = fs.options.data_replicas ?? 1}
					{@const showUsable = fsReplicas > 1}
					{@const fsUsable = showUsable
						? estimateUsableBytes(fs.total_bytes, fs.devices.length, fsReplicas, false)
						: fs.total_bytes}
					{@const deviceCount = fs.devices.length}
					{@const chip = scrubChip(scrubStatuses[fs.name])}
					<div class="mt-3">
						<div class="mb-1 h-1.5 overflow-hidden rounded-full bg-secondary">
							<div class="h-full rounded-full bg-primary" style="width: {(fs.used_bytes / fs.total_bytes) * 100}%"></div>
						</div>
						<span class="text-xs text-muted-foreground">
							{formatBytes(fs.used_bytes)} / {formatBytes(fs.total_bytes)} ({formatPercent(fs.used_bytes, fs.total_bytes)})
							{#if showUsable} · <span title="Estimate: total ÷ replicas for raid1.">~{formatBytes(fsUsable)} usable</span>{/if}
							· {deviceCount} {deviceCount === 1 ? 'device' : 'devices'}
							{#if fsReplicas > 1} · raid1{/if}
							{#if fs.options.compression} · {fs.options.compression}{/if}
							· <span class={chip.cls} title={chip.title}>{chip.label}</span>
						</span>
					</div>
				{/if}

				{#if editOptionsFs === fs.name}
				<div class="mt-4 border-t border-border pt-4">
					<h4 class="mb-4 text-xs uppercase tracking-wide text-muted-foreground">Edit Options</h4>
					<fieldset class="max-w-md rounded-md border border-border p-3">
						<legend class="px-1.5 text-[0.65rem] uppercase tracking-wide text-muted-foreground">Compression</legend>
						<label for="edit-compression-{fs.name}" class="mb-1 block text-xs text-muted-foreground">Algorithm</label>
						<div class="flex gap-2">
							<select id="edit-compression-{fs.name}" bind:value={editCompression} onchange={() => editCompressionLevel = ''} class="h-8 flex-1 rounded-md border border-input bg-transparent px-2 text-sm">
								<option value="">None</option>
								<option value="lz4">LZ4</option>
								<option value="zstd">Zstd</option>
								<option value="gzip">Gzip</option>
							</select>
							{#if compressionMaxLevel(editCompression) !== null}
								<input type="number" bind:value={editCompressionLevel} min="1" max={compressionMaxLevel(editCompression)} placeholder="lvl" title="Optional {editCompression} level (1–{compressionMaxLevel(editCompression)}). Blank = default." class="h-8 w-16 rounded-md border border-input bg-transparent px-2 text-sm" />
							{/if}
						</div>
					</fieldset>
					<div class="mt-4 flex gap-2">
						<Button size="xs" onclick={() => saveOptions(fs.name)}>Save</Button>
						<Button variant="secondary" size="xs" onclick={() => editOptionsFs = null}>Cancel</Button>
					</div>
				</div>
			{/if}

				{#if expandedFs === fs.name}
					<div class="mt-4 border-t border-border pt-4">
						<div class="mb-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-0.5 text-xs self-start max-w-md">
							<span class="text-muted-foreground">Data profile</span>
							<span>{(fs.options.data_replicas ?? 1) > 1 ? 'raid1' : 'single'}</span>
							<span class="text-muted-foreground">Compression</span>
							<span>{fs.options.compression ?? 'none'}</span>
						</div>

						<div class="mb-1 flex justify-end">
							<details class="relative text-xs">
								<summary class="cursor-pointer list-none rounded border border-border px-2 py-0.5 text-muted-foreground hover:bg-secondary">Columns ▾</summary>
								<div class="absolute right-0 z-10 mt-1 w-40 rounded-md border border-border bg-popover p-2 shadow-md">
									{#each DEVICE_COLUMNS as col}
										<label class="flex cursor-pointer items-center gap-2 py-0.5">
											<input type="checkbox" checked={colOn(col.id)} onchange={() => toggleCol(col.id)} class="h-3.5 w-3.5" />
											<span>{col.label}</span>
										</label>
									{/each}
								</div>
							</details>
						</div>
						<table class="w-full text-sm">
							<thead>
								<tr>
									<SortTh label="Device" active={devSortKey === 'path'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('path')} />
									<SortTh label="Label" active={devSortKey === 'label'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('label')} />
									<SortTh label="State" active={devSortKey === 'state'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('state')} />
									{#if colOn('slot')}<SortTh label="Slot" active={devSortKey === 'slot'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('slot')} />{/if}
									{#if colOn('uuid')}<SortTh label="UUID" active={devSortKey === 'uuid'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('uuid')} />{/if}
									{#if colOn('size')}<SortTh label="Size" active={devSortKey === 'size'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('size')} />{/if}
									{#if colOn('type')}<SortTh label="Type" active={devSortKey === 'type'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('type')} />{/if}
									{#if colOn('rotational')}<SortTh label="Rotational" active={devSortKey === 'rotational'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('rotational')} />{/if}
									{#if colOn('model')}<SortTh label="Model" active={devSortKey === 'model'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('model')} />{/if}
									{#if colOn('serial')}<SortTh label="Serial" active={devSortKey === 'serial'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('serial')} />{/if}
									{#if colOn('clean')}<SortTh label="Clean" active={devSortKey === 'clean'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('clean')} />{/if}
									{#if colOn('read_err')}<SortTh label="Read err" active={devSortKey === 'read_err'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('read_err')} />{/if}
									{#if colOn('write_err')}<SortTh label="Write err" active={devSortKey === 'write_err'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('write_err')} />{/if}
									{#if colOn('csum_err')}<SortTh label="Cksum err" active={devSortKey === 'csum_err'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('csum_err')} />{/if}
									{#if colOn('data_allowed')}<SortTh label="Data Allowed" active={devSortKey === 'data_allowed'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('data_allowed')} />{/if}
									{#if colOn('has_data')}<SortTh label="Has Data" active={devSortKey === 'has_data'} dir={devSortDir} thClass="p-2" onclick={() => toggleDevSort('has_data')} />{/if}
								</tr>
							</thead>
							<tbody>
								{#each sortedDevices(fs.devices) as dev (dev.path)}
									<tr class="border-b border-border">
										<td class="p-2 font-mono text-xs">
											{dev.path}
											{#if dev.durability !== null && dev.durability !== undefined && dev.durability !== 1}
												<span class="ml-1 rounded bg-secondary px-1 py-0.5 text-[10px] text-muted-foreground">durability={dev.durability}</span>
											{/if}
											{#if dev.discard}
												<span class="ml-1 rounded bg-secondary px-1 py-0.5 text-[10px] text-muted-foreground">discard</span>
											{/if}
										</td>
										<td class="p-2 text-xs {dev.label ? '' : 'text-muted-foreground'}">{dev.label ?? '—'}</td>
										<td class="p-2">
											{#if dev.missing}
												<span class="rounded bg-red-950 px-2 py-0.5 text-xs font-semibold text-red-400" title="Member device is not currently detected">missing</span>
											{:else if dev.state !== null}
												<span class="rounded px-2 py-0.5 text-xs font-semibold {stateColor(dev.state)}">
													{dev.state}
												</span>
											{:else}
												<span class="text-muted-foreground">—</span>
											{/if}
										</td>
										{#if colOn('slot')}
											<td class="p-2 font-mono text-xs text-muted-foreground">{dev.member_index ?? '—'}</td>
										{/if}
										{#if colOn('uuid')}
											<td class="p-2 font-mono text-xs text-muted-foreground" title={dev.uuid ?? ''}>{dev.uuid ? dev.uuid.slice(0, 8) : '—'}</td>
										{/if}
										{#if colOn('size')}
											{@const blk = deviceBlock(dev.path)}
											<td class="p-2 font-mono text-xs text-muted-foreground">{blk ? formatBytes(blk.size_bytes) : '—'}</td>
										{/if}
										{#if colOn('type')}
											{@const blk = deviceBlock(dev.path)}
											<td class="p-2 text-xs text-muted-foreground uppercase">{blk?.device_class ?? '—'}</td>
										{/if}
										{#if colOn('rotational')}
											{@const blk = deviceBlock(dev.path)}
											{@const mismatch = dev.rotational === true && blk != null && !blk.rotational}
											<td class="p-2 text-xs">
												{#if dev.rotational == null}
													<span class="text-muted-foreground">—</span>
												{:else if mismatch}
													<span class="text-amber-500" title="Filesystem marks this device rotational, but the hardware is solid-state.">yes ⚠</span>
												{:else}
													<span class="text-muted-foreground">{dev.rotational ? 'yes' : 'no'}</span>
												{/if}
											</td>
										{/if}
										{#if colOn('model')}
											{@const blk = deviceBlock(dev.path)}
											<td class="p-2 text-xs text-muted-foreground">{blk?.model ?? '—'}</td>
										{/if}
										{#if colOn('serial')}
											{@const blk = deviceBlock(dev.path)}
											<td class="p-2 font-mono text-xs text-muted-foreground">{blk?.serial ?? '—'}</td>
										{/if}
										{#if colOn('clean')}
											{@const errTotal = devErrorTotal(dev)}
											<td class="p-2 text-xs">
												{#if errTotal === null}
													<span class="text-muted-foreground">—</span>
												{:else if errTotal === 0}
													<span class="text-green-500" title="No read/write/checksum errors since creation">✓</span>
												{:else}
													<span class="text-red-500" title="read {dev.read_errors ?? 0}, write {dev.write_errors ?? 0}, checksum {dev.checksum_errors ?? 0}">{errTotal} ✗</span>
												{/if}
											</td>
										{/if}
										{#if colOn('read_err')}
											<td class="p-2 font-mono text-xs {dev.read_errors ? 'text-red-500' : 'text-muted-foreground'}">{dev.read_errors ?? '—'}</td>
										{/if}
										{#if colOn('write_err')}
											<td class="p-2 font-mono text-xs {dev.write_errors ? 'text-red-500' : 'text-muted-foreground'}">{dev.write_errors ?? '—'}</td>
										{/if}
										{#if colOn('csum_err')}
											<td class="p-2 font-mono text-xs {dev.checksum_errors ? 'text-red-500' : 'text-muted-foreground'}">{dev.checksum_errors ?? '—'}</td>
										{/if}
										{#if colOn('data_allowed')}
											<td class="p-2 font-mono text-xs text-muted-foreground">{dev.data_allowed ?? '—'}</td>
										{/if}
										{#if colOn('has_data')}
											<td class="p-2 font-mono text-xs text-muted-foreground">{dev.has_data ?? '—'}</td>
										{/if}
									</tr>
								{/each}
							</tbody>
						</table>

					</div>
				{/if}
			</CardContent>
		</Card>
	{/each}
{/if}

{/if}
<!-- end pageTab === 'manage' -->
