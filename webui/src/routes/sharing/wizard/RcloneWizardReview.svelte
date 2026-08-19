<script lang="ts">
	import type { RcloneProto } from '$lib/types';

	interface Props {
		kind: RcloneProto;
		name: string;
		fallbackName: string;
		readOnly: boolean;
	}
	let { kind, name, fallbackName, readOnly }: Props = $props();
</script>

<span class="text-muted-foreground">{kind === 's3' ? 'Bucket' : 'Share Name'}</span>
<span>{name || fallbackName}</span>
{#if readOnly}
	<span class="text-muted-foreground">Access</span>
	<span>Read-only</span>
{/if}
