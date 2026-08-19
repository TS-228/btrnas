<script lang="ts">
	import { goto } from '$app/navigation';
	import { Button } from '$lib/components/ui/button';
	import { Badge } from '$lib/components/ui/badge';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { Card, CardContent } from '$lib/components/ui/card';
	import SortTh from '$lib/components/SortTh.svelte';
	import { requiredFieldCls } from '$lib/utils';
	import type { RcloneProto } from '$lib/types';
	import {
		rcloneStore,
		rcloneLabel,
		rcloneToggleSort,
		rcloneCreate,
		rcloneToggleEnabled,
		rcloneRemove,
		rcloneToggleReadOnly,
		rcloneOnSubvolumeSelect,
		rcloneLoadSubvolumes,
		rcloneLoadSettings,
		rcloneSaveSettings,
	} from '$lib/sharing/rclone.svelte';

	interface Props {
		kind: RcloneProto;
	}
	let { kind }: Props = $props();

	const s = $derived(rcloneStore(kind));
	const label = $derived(rcloneLabel(kind));
	const userLabel = $derived(kind === 's3' ? 'Access key' : 'Username');
	const passLabel = $derived(kind === 's3' ? 'Secret key' : 'Password');
	const endpointHint = $derived.by(() => {
		if (kind === 'ftp') return `ftp://${s.listen === '0.0.0.0' ? '<host>' : s.listen}:${s.port || 21}/`;
		if (kind === 'sftp') return `sftp://${s.listen === '0.0.0.0' ? '<host>' : s.listen}:${s.port || 2022}/`;
		return `http://${s.listen === '0.0.0.0' ? '<host>' : s.listen}:${s.port || 9000}/  (path-style buckets)`;
	});

	let createTried = $state(false);
	async function createGuarded() {
		if (!s.newName || !s.newSubvolume) { createTried = true; return; }
		createTried = false;
		await rcloneCreate(kind);
	}

	$effect(() => { if (s.showCreate) rcloneLoadSubvolumes(kind); });

	const filtered = $derived(
		s.search.trim()
			? s.shares.filter(share =>
				share.name.toLowerCase().includes(s.search.toLowerCase()) ||
				share.path.toLowerCase().includes(s.search.toLowerCase()) ||
				share.comment?.toLowerCase().includes(s.search.toLowerCase()))
			: s.shares
	);

	const sorted = $derived.by(() => {
		if (!s.sortKey) return filtered;
		return [...filtered].sort((a, b) => {
			let cmp = 0;
			if (s.sortKey === 'name') cmp = a.name.localeCompare(b.name);
			else if (s.sortKey === 'path') cmp = a.path.localeCompare(b.path);
			else if (s.sortKey === 'status') cmp = Number(b.enabled) - Number(a.enabled);
			return s.sortDir === 'asc' ? cmp : -cmp;
		});
	});

	async function copy(text: string) {
		try { await navigator.clipboard.writeText(text); } catch { /* ignore */ }
	}
</script>

<div class="mb-4 flex flex-wrap items-center gap-3">
	<Input bind:value={s.search} placeholder="Search..." class="h-9 w-48" />
	<Button size="xs" variant="secondary" onclick={() => { s.settingsOpen = !s.settingsOpen; if (s.settingsOpen) rcloneLoadSettings(kind); }}>
		{s.settingsOpen ? 'Hide server settings' : 'Server settings'}
	</Button>
</div>

{#if s.settingsOpen}
	<Card class="mb-6 max-w-2xl">
		<CardContent class="pt-6">
			<h3 class="mb-1 text-lg font-semibold">{label} server</h3>
			<p class="mb-4 text-xs text-muted-foreground">
				One {label} listener for every enabled share. Shares appear as
				{kind === 's3' ? 'S3 buckets' : 'top-level folders'}.
				Endpoint: <code class="font-mono">{endpointHint}</code>
			</p>
			{#if s.settingsLoading && !s.settings}
				<p class="mb-4 text-sm text-muted-foreground">Loading server settings…</p>
			{/if}
			<div class="mb-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
				<div>
					<Label for="{kind}-listen">Listen address</Label>
					<Input id="{kind}-listen" bind:value={s.listen} class="mt-1 font-mono" />
				</div>
				<div>
					<Label for="{kind}-port">Port</Label>
					<Input id="{kind}-port" type="number" bind:value={s.port} class="mt-1 font-mono" />
				</div>
			</div>
			{#if kind === 'ftp'}
				<div class="mb-4">
					<label class="flex cursor-pointer items-center gap-2 text-sm">
						<input type="checkbox" bind:checked={s.anonymous} class="h-4 w-4" />
						Allow anonymous login
					</label>
					<p class="mt-1 text-xs text-muted-foreground">rclone accepts any password for user <code class="font-mono">anonymous</code>.</p>
				</div>
				<div class="mb-4 grid grid-cols-2 gap-4">
					<div>
						<Label for="{kind}-pasv-min">Passive port min</Label>
						<Input id="{kind}-pasv-min" type="number" bind:value={s.passiveMin} class="mt-1 font-mono" />
					</div>
					<div>
						<Label for="{kind}-pasv-max">Passive port max</Label>
						<Input id="{kind}-pasv-max" type="number" bind:value={s.passiveMax} class="mt-1 font-mono" />
					</div>
				</div>
			{/if}
			{#if kind !== 'ftp' || !s.anonymous}
				<div class="mb-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
					<div>
						<Label for="{kind}-user">{userLabel}</Label>
						<div class="mt-1 flex gap-2">
							<Input id="{kind}-user" bind:value={s.username} class="font-mono" />
							<Button size="xs" variant="secondary" onclick={() => copy(s.username)}>Copy</Button>
						</div>
					</div>
					<div>
						<Label for="{kind}-pass">{passLabel}</Label>
						<div class="mt-1 flex gap-2">
							<Input id="{kind}-pass" type={s.passwordRevealed ? 'text' : 'password'} bind:value={s.password} class="font-mono" />
							<Button size="xs" variant="secondary" onclick={() => s.passwordRevealed = !s.passwordRevealed}>
								{s.passwordRevealed ? 'Hide' : 'Show'}
							</Button>
							<Button size="xs" variant="secondary" onclick={() => copy(s.password)}>Copy</Button>
						</div>
					</div>
				</div>
			{/if}
			<Button size="sm" onclick={() => rcloneSaveSettings(kind)} disabled={s.settingsLoading || !s.settings}>Save</Button>
		</CardContent>
	</Card>
{/if}

{#if s.showCreate}
	<Card class="mb-6 max-w-2xl">
		<CardContent class="pt-6">
			<h3 class="mb-4 text-lg font-semibold">New {label} Share</h3>
			<div class="mb-4">
				<Label for="{kind}-subvol">Subvolume {#if !s.newSubvolume && createTried}<span class="text-xs font-normal text-amber-500">required</span>{/if}</Label>
				<select id="{kind}-subvol" bind:value={s.newSubvolume} onchange={() => rcloneOnSubvolumeSelect(kind)} class="mt-1 h-9 w-full rounded-md border border-input bg-transparent px-3 text-sm {requiredFieldCls(!s.newSubvolume, createTried)}">
					<option value="">Select a subvolume...</option>
					{#each s.subvolumes as sv}
						<option value={sv.path}>{sv.filesystem}/{sv.name} ({sv.path})</option>
					{/each}
				</select>
				{#if s.subvolumes.length === 0}
					<span class="mt-1 block text-xs text-muted-foreground">No filesystem subvolumes found.</span>
					<Button size="xs" class="mt-1" onclick={() => goto('/subvolumes')}>Subvolumes</Button>
				{/if}
			</div>
			<div class="mb-4">
				<Label for="{kind}-name">{kind === 's3' ? 'Bucket name' : 'Share name'} {#if !s.newName && createTried}<span class="text-xs font-normal text-amber-500">required</span>{/if}</Label>
				<Input id="{kind}-name" bind:value={s.newName} placeholder="documents" class="mt-1 {requiredFieldCls(!s.newName, createTried)}" />
				<span class="mt-1 block text-xs text-muted-foreground">Letters, digits, <code class="font-mono">.</code> <code class="font-mono">_</code> <code class="font-mono">-</code> — no spaces.</span>
			</div>
			<div class="mb-4">
				<Label for="{kind}-comment">Comment</Label>
				<Input id="{kind}-comment" bind:value={s.newComment} placeholder="Optional description" class="mt-1" />
			</div>
			<div class="mb-4">
				<label class="flex cursor-pointer items-center gap-2">
					<input type="checkbox" bind:checked={s.newReadOnly} class="h-4 w-4" /> Read-only
				</label>
			</div>
			<Button onclick={createGuarded}>Create</Button>
		</CardContent>
	</Card>
{/if}

{#if s.loading}
	<p class="text-muted-foreground">Loading...</p>
{:else if s.shares.length === 0}
	<p class="text-muted-foreground">No shares configured.</p>
{:else}
	<table class="w-full text-sm">
		<thead>
			<tr>
				<SortTh label="Name" active={s.sortKey === 'name'} dir={s.sortDir} onclick={() => rcloneToggleSort(kind, 'name')} />
				<SortTh label="Path" active={s.sortKey === 'path'} dir={s.sortDir} onclick={() => rcloneToggleSort(kind, 'path')} />
				<th class="border-b-2 border-border p-3 text-left text-xs uppercase text-muted-foreground">Access</th>
				<SortTh label="Status" active={s.sortKey === 'status'} dir={s.sortDir} onclick={() => rcloneToggleSort(kind, 'status')} />
				<th class="border-b-2 border-border p-3 text-left text-xs uppercase text-muted-foreground w-px whitespace-nowrap">Actions</th>
			</tr>
		</thead>
		<tbody>
			{#each sorted as share (share.id)}
				<tr
					class="border-b border-border cursor-pointer hover:bg-muted/30 transition-colors"
					onclick={() => s.expanded[share.id] = !s.expanded[share.id]}
				>
					<td class="p-3">
						<strong>{share.name}</strong>
						{#if share.comment}<br /><span class="text-xs text-muted-foreground">{share.comment}</span>{/if}
					</td>
					<td class="p-3 font-mono text-sm">{share.path}</td>
					<td class="p-3">
						<span class="mr-1 inline-block rounded bg-secondary px-1.5 py-0.5 text-xs">{share.read_only ? 'RO' : 'RW'}</span>
					</td>
					<td class="p-3">
						<Badge variant={share.enabled ? 'default' : 'secondary'}>
							{share.enabled ? 'Enabled' : 'Disabled'}
						</Badge>
					</td>
					<td class="p-3" onclick={(e) => e.stopPropagation()}>
						<div class="flex gap-2">
							<Button variant="secondary" size="xs" onclick={() => s.expanded[share.id] = !s.expanded[share.id]}>
								{s.expanded[share.id] ? 'Hide' : 'Details'}
							</Button>
							<Button variant="secondary" size="xs" onclick={() => rcloneToggleEnabled(kind, share)}>
								{share.enabled ? 'Disable' : 'Enable'}
							</Button>
							<Button variant="destructive" size="xs" onclick={() => rcloneRemove(kind, share.id)}>Delete</Button>
						</div>
					</td>
				</tr>
				{#if s.expanded[share.id]}
					<tr class="border-b border-border bg-muted/20">
						<td colspan="5" class="px-6 py-4">
							<label class="flex cursor-pointer items-center gap-2 text-sm">
								<input type="checkbox" checked={share.read_only} onchange={() => rcloneToggleReadOnly(kind, share)} class="h-4 w-4" />
								Read-only
							</label>
						</td>
					</tr>
				{/if}
			{/each}
		</tbody>
	</table>
{/if}
