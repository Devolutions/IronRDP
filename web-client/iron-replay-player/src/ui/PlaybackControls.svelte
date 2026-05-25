<script lang="ts">
	import { formatTime } from './format-time.js';

	const SPEEDS = [3, 2, 1.75, 1.5, 1.25, 1];

	interface Props {
		paused: boolean;
		waiting: boolean;
		canPlay: boolean;
		elapsed: number;
		duration: number;
		speed: number;
		isFullscreen: boolean;
		onplay: () => void;
		onpause: () => void;
		onreset: () => void;
		onspeedchange: (speed: number) => void;
		onfullscreen: () => void;
	}

	let {
		paused,
		waiting,
		canPlay,
		elapsed,
		duration,
		speed,
		isFullscreen,
		onplay,
		onpause,
		onreset,
		onspeedchange,
		onfullscreen,
	}: Props = $props();

	let speedOpen = $state(false);

	function togglePlay(): void {
		if (paused) {
			onplay();
		} else {
			onpause();
		}
	}

	function selectSpeed(value: number): void {
		onspeedchange(value);
		speedOpen = false;
	}

	function formatSpeed(value: number): string {
		return `${value}`;
	}

	/** Close popup when clicking outside the speed selector */
	function clickOutside(node: HTMLElement, handler: () => void) {
		const handleClick = (e: MouseEvent) => {
			if (!node.contains(e.target as Node)) handler();
		};
		document.addEventListener('click', handleClick, true);
		return {
			destroy() {
				document.removeEventListener('click', handleClick, true);
			},
		};
	}
</script>

<div class="__irp-controls-bar">
	<!-- Left group: reset + play/pause + time -->
	<div class="__irp-controls-left">
		<button
			class="__irp-play-btn"
			onclick={onreset}
			disabled={!canPlay}
			aria-label="Reset to beginning"
		>
			⏮
		</button>
		<button
			class="__irp-play-btn"
			onclick={togglePlay}
			disabled={!canPlay}
			aria-label={paused ? 'Play' : 'Pause'}
		>
			{paused ? '▶' : '⏸'}
		</button>
		<span class="__irp-time-display">
			{formatTime(elapsed)} / {formatTime(duration)}
		</span>
	</div>

	<!-- Right group: speed + fullscreen -->
	<div class="__irp-controls-right">
		<!-- Speed selector -->
		<div class="__irp-speed-selector" use:clickOutside={() => (speedOpen = false)}>
			<button
				class="__irp-speed-btn"
				onclick={() => (speedOpen = !speedOpen)}
				aria-label="Playback speed"
				aria-expanded={speedOpen}
			>
				{formatSpeed(speed)}
			</button>
			{#if speedOpen}
				<div class="__irp-speed-popup" role="menu">
					<div class="__irp-speed-popup-heading">Playback speed</div>
					{#each SPEEDS as s}
						<button
							class="__irp-speed-popup-item"
							class:__irp-active={s === speed}
							role="menuitem"
							onclick={() => selectSpeed(s)}
						>
							<span class="__irp-speed-popup-check">{s === speed ? '✓' : ''}</span>
							{formatSpeed(s)}
						</button>
					{/each}
				</div>
			{/if}
		</div>

		<!-- Fullscreen -->
		<button
			class="__irp-fullscreen-btn"
			onclick={onfullscreen}
			aria-label={isFullscreen ? 'Exit fullscreen' : 'Enter fullscreen'}
		>
			{isFullscreen ? '✕' : '⛶'}
		</button>
	</div>
</div>
