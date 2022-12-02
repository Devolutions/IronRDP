<script>
	import { throttleTime } from 'rxjs';
	import { onMount } from 'svelte';
	import { serverBridge } from '../../services/services-injector';
	import { currentSession } from '../../services/session.service';
	import * as userInteractionService from '../../services/user-interaction-service';

	export const scale = 'full';

	let canvas;
	let canvasCtx;

	function getMousePos(evt) {
		const rect = canvas?.getBoundingClientRect(),
			scaleX = canvas?.width / rect.width,
			scaleY = canvas?.height / rect.height;

		const coord = {
			x: Math.round((evt.clientX - rect.left) * scaleX),
			y: Math.round((evt.clientY - rect.top) * scaleY)
		};

		userInteractionService.setMousePosition(coord);
	}

	function setMouseState(state) {
		userInteractionService.setMouseLeftClickState(state);
	}

	async function draw(bytesArray, imageInfo) {
		const pixels = new Uint8ClampedArray(await bytesArray);
		if (pixels.buffer.byteLength > 0) {
			const imageData = new ImageData(pixels, imageInfo.width, imageInfo.height);
			canvasCtx?.putImageData(imageData, imageInfo.left, imageInfo.top);
		}
	}

	function initcanvas() {
		canvas = document.getElementById('renderer');
		canvasCtx = canvas?.getContext('2d', { alpha: false });

		canvas.width = $currentSession.desktopSize.width;
		canvas.height = $currentSession.desktopSize.height;

		serverBridge.resize.subscribe((desktopSize) => {
			canvas.width = desktopSize.desktop_size.width;
			canvas.height = desktopSize.desktop_size.height;
		});
		serverBridge.updateImage.pipe(throttleTime(1000 / 60)).subscribe(({ pixels, infos }) => {
			draw(pixels, infos);
		});
	}

	onMount(async () => {
		initcanvas();
	});
</script>

<div class="screen-wrapper scale-{scale}">
	<div class="screen-viewer">
		<canvas
			on:mousemove={getMousePos}
			on:mousedown={() => setMouseState(1)}
			on:mouseup={() => setMouseState(0)}
			on:mouseleave={() => setMouseState(0)}
			id="renderer"
		/>
	</div>
</div>

<style>
	:root {
		--screen-padding: 30px;
	}

	.screen-wrapper {
		position: relative;
		width: calc(100% - var(--screen-padding) * 2);
		height: calc(100% - var(--screen-padding) * 2);
		padding-left: var(--screen-padding);
		padding-top: var(--screen-padding);
		max-height: 100%;
	}

	.screen-wrapper .scale-fit canvas {
		width: 100%;
	}

	.screen-viewer {
		width: 100%;
		height: 100%;
		overflow: auto;
		text-align: center;
	}
</style>
