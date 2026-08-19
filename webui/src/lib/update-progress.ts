export interface UpdatePhase {
	label: string;
	marker: string;
}

export const versionUpdatePhases = [
	{ label: 'Snapshot', marker: '==> Creating snapper pre-snapshot' },
	{ label: 'Update', marker: '==> apt-get update' },
	{ label: 'Upgrade', marker: '==> apt-get upgrade' },
	{ label: 'Done', marker: '==> Upgrade finished' }
] as const satisfies readonly UpdatePhase[];

export function reachedUpdatePhase(log: string, phases: readonly UpdatePhase[]): number {
	let reached = -1;
	for (let i = 0; i < phases.length; i++) {
		if (log.includes(phases[i].marker)) reached = i;
	}
	return reached;
}

export function shouldShowUpdateStatus(state: string | null): boolean {
	return state !== null && state !== 'idle';
}
