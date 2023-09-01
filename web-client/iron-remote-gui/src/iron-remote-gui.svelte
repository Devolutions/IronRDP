<svelte:options tag="iron-remote-gui"/>

<script lang="ts">
    import {onMount} from 'svelte';
    import {get_current_component} from "svelte/internal";
    import {loggingService} from "./services/logging.service";
    import {WasmBridgeService} from './services/wasm-bridge.service';
    import {LogType} from './enums/LogType';
    import type {ResizeEvent} from './interfaces/ResizeEvent';
    import {PublicAPI} from './services/PublicAPI';
    import {ScreenScale} from './enums/ScreenScale';

    export let scale = 'real';
    export let verbose = 'false';
    export let debugwasm: "OFF" | "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" = 'INFO';
    export let flexcenter = 'true';

    let isVisible = false;
    let capturingInputs = false;
    let currentComponent = get_current_component();
    let canvas;
    let canvasCtx;

    let wrapper;
    let viewer;

    let viewerStyle;
    let wrapperStyle;
    
    let wasmService = new WasmBridgeService();
    let publicAPI = new PublicAPI(wasmService);

    function initListeners() {
        serverBridgeListeners();
        userInteractionListeners();

        window.addEventListener('keydown', keyboardEvent, false);
        window.addEventListener('keyup', keyboardEvent, false);
    }

    function resetHostStyle() {
        if (flexcenter === 'true') {
            currentComponent.style.flexGrow = null;
            currentComponent.style.display = null;
            currentComponent.style.justifyContent = null;
            currentComponent.style.alignItems = null;
        }
    }

    function setHostStyle(full: boolean) {
        if (flexcenter === 'true') {
            if (!full) {
                currentComponent.style.flexGrow = 1;
                currentComponent.style.display = "flex";
                currentComponent.style.justifyContent = "center";
                currentComponent.style.alignItems = "center";
            } else {
                currentComponent.style.flexGrow = 1;
            }
        }
    }

    function setViewerStyle(height: string, width: string, forceMinAndMax: boolean) {
        let newStyle = `height: ${height}; width: ${width}`;
        if (forceMinAndMax) {
            newStyle = forceMinAndMax
                ? `${newStyle}; max-height: ${height}; max-width: ${width}; min-height: ${height}; min-width: ${width}`
                : `${newStyle}; max-height: initial; max-width: initial; min-height: initial; min-width: initial`;
        }
        viewerStyle = newStyle;
    }

    function setWrapperStyle(height, width, overflow) {
        wrapperStyle = `height: ${height}; width: ${width}; overflow: ${overflow}`;
    }

    function serverBridgeListeners() {
        wasmService.resize.subscribe((evt: ResizeEvent) => {
            loggingService.info(`Resize canvas to: ${evt.desktop_size.width}x${evt.desktop_size.height}`);
            canvas.width = evt.desktop_size.width;
            canvas.height = evt.desktop_size.height;
            scaleSession(scale);
        });
    }

    function userInteractionListeners() {
        window.addEventListener('resize', (_evt) => {
            scaleSession(scale);
        });

        wasmService.scaleObserver.subscribe(s => {
            loggingService.info("Change scale!");
            scaleSession(s);
        });

        wasmService.changeVisibilityObservable.subscribe(val => {
            isVisible = val;
            if (val) {
                //Enforce first scaling and delay the call to scaleSession to ensure Dom is ready.
                setWrapperStyle("100%", "100%", "hidden");
                setTimeout(() => scaleSession(scale), 150);
            }
        });
    }

    function scaleSession(currentSize: ScreenScale | string) {
        resetHostStyle();
        if (isVisible) {
            switch (currentSize) {
                case 'fit':
                case ScreenScale.Fit:
                    loggingService.info("Size to fit");
                    scale = 'fit';
                    fitResize();
                    break;
                case 'full':
                case ScreenScale.Full:
                    loggingService.info("Size to full");
                    fullResize();
                    scale = 'full';
                    break;
                case 'real':
                case ScreenScale.Real:
                    loggingService.info("Size to real");
                    realResize();
                    scale = 'real';
                    break
            }
        }
    }

    function fullResize() {
        const windowSize = getWindowSize();
        const wrapperBoundingBox = wrapper.getBoundingClientRect();

        const containerWidth = windowSize.x - wrapperBoundingBox.x;
        const containerHeight = windowSize.y - wrapperBoundingBox.y;

        let width = canvas.width;
        let height = canvas.height;

        const ratio = Math.max(containerWidth / canvas.width, containerHeight / canvas.height);
        width = width * ratio;
        height = height * ratio;

        setWrapperStyle(`${containerHeight}px`, `${containerWidth}px`, 'auto');

        width = width > 0 ? width : 0;
        height = height > 0 ? height : 0;

        setViewerStyle(`${height}px`, `${width}px`, true);
    }

    function fitResize(realSizeLimit = false) {
        const windowSize = getWindowSize();
        const wrapperBoundingBox = wrapper.getBoundingClientRect();

        const containerWidth = windowSize.x - wrapperBoundingBox.x;
        const containerHeight = windowSize.y - wrapperBoundingBox.y;

        let width = canvas.width;
        let height = canvas.height;

        if (!realSizeLimit || containerWidth < canvas.width || containerHeight < canvas.height) {
            const ratio = Math.min(containerWidth / canvas.width, containerHeight / canvas.height);
            width = width * ratio;
            height = height * ratio;
        }

        width = width > 0 ? width : 0;
        height = height > 0 ? height : 0;

        setWrapperStyle('initial', 'initial', 'hidden');
        setViewerStyle(`${height}px`, `${width}px`, true);
        setHostStyle(false);
    }

    function realResize() {
        const windowSize = getWindowSize();
        const wrapperBoundingBox = wrapper.getBoundingClientRect();

        const containerWidth = windowSize.x - wrapperBoundingBox.x;
        const containerHeight = windowSize.y - wrapperBoundingBox.y;

        if (containerWidth < canvas.width || containerHeight < canvas.height) {
            setWrapperStyle(`${Math.min(containerHeight, canvas.height)}px`, `${Math.min(containerWidth, canvas.width)}px`, 'auto');
        } else {
            setWrapperStyle('initial', 'initial', 'initial');
        }

        setViewerStyle(`${canvas.height}px`, `${canvas.width}px`, true);
        setHostStyle(false);
    }

    function getMousePos(evt) {
        const rect = canvas?.getBoundingClientRect(),
            scaleX = canvas?.width / rect.width,
            scaleY = canvas?.height / rect.height;

        const coord = {
            x: Math.round((evt.clientX - rect.left) * scaleX),
            y: Math.round((evt.clientY - rect.top) * scaleY)
        };

        wasmService.updateMousePosition(coord);
    }

    function setMouseButtonState(state, isDown) {
        wasmService.mouseButtonState(state, isDown);
    }

    function mouseWheel(evt) {
        wasmService.mouseWheel(evt);
    }

    function setMouseIn(evt) {
        capturingInputs = true;
        wasmService.mouseIn(evt);
    }

    function setMouseOut(evt) {
        capturingInputs = false;
        wasmService.mouseOut(evt);
    }

    function keyboardEvent(evt) {
        wasmService.sendKeyboardEvent(evt);
    }

    function getWindowSize() {
        const win = window,
            doc = document,
            docElem = doc.documentElement,
            body = doc.getElementsByTagName('body')[0],
            x = win.innerWidth || docElem.clientWidth || body.clientWidth,
            y = win.innerHeight || docElem.clientHeight || body.clientHeight;
        return {x, y};
    }

    async function initcanvas() {
        loggingService.info('Start canvas initialization.')
        canvas = currentComponent.shadowRoot.getElementById('renderer');
        canvasCtx = canvas?.getContext('2d', {alpha: false});

        // Set a default canvas size. Need more test to know if i can remove it.
        canvas.width = 800;
        canvas.height = 600;

        await wasmService.init(LogType[debugwasm] || LogType.INFO);
        wasmService.setCanvas(canvas);

        initListeners();

        let result = {
            irgUserInteraction: publicAPI.getExposedFunctions()
        };

        loggingService.info('Component ready');
        currentComponent.dispatchEvent(new CustomEvent("ready", {detail: result}));
    }

    onMount(async () => {
        loggingService.verbose = verbose === 'true';
        loggingService.info('Dom ready');
        await initcanvas();
    });
</script>

<div bind:this={wrapper} class="screen-wrapper scale-{scale}" class:hidden="{!isVisible}"
     class:capturing-inputs="{capturingInputs}"
     style="{wrapperStyle}">
    <div bind:this={viewer} class="screen-viewer" style="{viewerStyle}">
        <canvas
                on:mousemove={getMousePos}
                on:mousedown={(event) => setMouseButtonState(event, true)}
                on:mouseup={(event) => setMouseButtonState(event, false)}
                on:mouseleave={(event) => {
                        setMouseButtonState(event, false);
                        setMouseOut(event);
                    }
                }
                on:mouseenter={(event) => {
                        setMouseIn(event);
                    }
                }
                on:contextmenu={(event) => event.preventDefault()}
                on:wheel={mouseWheel}
                id="renderer"
        />
    </div>
</div>

<style>
    .screen-wrapper {
        position: relative;
    }

    .capturing-inputs {
        outline: 1px solid rgba(0, 97, 166, .7);
        outline-offset: -1px;
    }

    canvas {
        width: 100%;
        height: 100%;
    }

    .screen-wrapper.hidden {
        pointer-events: none !important;
        position: absolute !important;
        visibility: hidden;
        height: 100%;
        width: 100%;
        transform: translate(-100%, -100%);
    }
</style>
