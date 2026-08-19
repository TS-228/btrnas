<script lang="ts">
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import type { RcloneProto } from '$lib/types';

	interface Props {
		kind: RcloneProto;
		name: string;
		readOnly: boolean;
	}
	let { kind, name = $bindable(), readOnly = $bindable() }: Props = $props();
</script>

<div class="mb-4">
	<Label>{kind === 's3' ? 'Bucket Name' : 'Share Name'}</Label>
	<Input bind:value={name} placeholder="documents" class="mt-1" />
	<p class="mt-1 text-xs text-muted-foreground">Letters, digits, <code class="font-mono">.</code> <code class="font-mono">_</code> <code class="font-mono">-</code> — no spaces.</p>
</div>
<div class="mb-4">
	<label class="flex items-center gap-2 text-sm cursor-pointer">
		<input type="checkbox" bind:checked={readOnly} class="rounded border-input" />
		Read-only
	</label>
</div>
