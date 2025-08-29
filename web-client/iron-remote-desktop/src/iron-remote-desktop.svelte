<svelte:options
    customElement={{
        tag: 'iron-remote-desktop',
        shadow: 'none',
        extend: (elementConstructor) => {
            return class extends elementConstructor {
                constructor() {
                    super();
                    this.attachShadow({ mode: 'open', delegatesFocus: true });
                }
            };
        },
    }}
/>

<script lang="ts">
    import { onDestroy, onMount } from 'svelte';
    import { loggingService } from './services/logging.service';
    import { RemoteDesktopService } from './services/remote-desktop.service';
    import type { ResizeEvent } from './interfaces/ResizeEvent';
    import { PublicAPI } from './services/PublicAPI';
    import { ScreenScale } from './enums/ScreenScale';
    import type { RemoteDesktopModule } from './interfaces/RemoteDesktopModule';
    import { isComponentDestroyed } from './lib/stores/componentLifecycleStore';
    import { runWhenFocusedQueue } from './lib/stores/runWhenFocusedStore';
    import { ClipboardService } from './services/clipboard.service';

    let {
        scale,
        verbose,
        flexcenter,
        module,
    }: {
        scale: string;
        verbose: 'true' | 'false';
        flexcenter: string;
        module: RemoteDesktopModule;
    } = $props();

    let isVisible = $state(false);
    let capturingInputs = () => {
        loggingService.info(`
            capturingInputs: ${document.activeElement === canvas}
            current active element: ${document.activeElement}
        `);
        return document.activeElement?.shadowRoot?.firstElementChild === inner;
    };

    let inner: HTMLDivElement;
    let wrapper: HTMLDivElement;
    let canvas: HTMLCanvasElement;

    let viewerStyle = $state('');
    let wrapperStyle = $state('');
    let remoteDesktopService = new RemoteDesktopService(module);
    let clipboardService = new ClipboardService(remoteDesktopService, module);
    let publicAPI = new PublicAPI(remoteDesktopService, clipboardService);

    let currentScreenScale = ScreenScale.Fit;

    function initListeners() {
        serverBridgeListeners();
        userInteractionListeners();

        function captureKeys(evt: KeyboardEvent) {
            if (capturingInputs()) {
                keyboardEvent(evt);
            }
        }

        window.addEventListener('keydown', captureKeys, false);
        window.addEventListener('keyup', captureKeys, false);

        window.addEventListener('focus', focusEventHandler);
    }

    function resetHostStyle() {
        if (flexcenter === 'true') {
            inner.style.flexGrow = '';
            inner.style.display = '';
            inner.style.justifyContent = '';
            inner.style.alignItems = '';
        }
    }

    function setHostStyle(full: boolean) {
        if (flexcenter === 'true') {
            if (!full) {
                inner.style.flexGrow = '1';
                inner.style.display = 'flex';
                inner.style.justifyContent = 'center';
                inner.style.alignItems = 'center';
            } else {
                inner.style.flexGrow = '1';
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

    function setWrapperStyle(height: string, width: string, overflow: string) {
        wrapperStyle = `height: ${height}; width: ${width}; overflow: ${overflow}`;
    }

    const resizeHandler = (_evt: UIEvent) => {
        scaleSession(scale);
    };

    function serverBridgeListeners() {
        remoteDesktopService.resizeObservable.subscribe((evt: ResizeEvent) => {
            loggingService.info(`Resize canvas to: ${evt.desktopSize.width}x${evt.desktopSize.height}`);
            canvas.width = evt.desktopSize.width;
            canvas.height = evt.desktopSize.height;
            scaleSession(scale);
        });
    }

    function userInteractionListeners() {
        window.addEventListener('resize', resizeHandler);

        remoteDesktopService.scaleObservable.subscribe((s) => {
            loggingService.info('Change scale!');
            scaleSession(s);
        });

        remoteDesktopService.dynamicResizeObservable.subscribe((evt) => {
            loggingService.info(`Dynamic resize!, width: ${evt.width}, height: ${evt.height}`);
            setViewerStyle(evt.height.toString() + 'px', evt.width.toString() + 'px', true);
        });

        remoteDesktopService.changeVisibilityObservable.subscribe((val) => {
            isVisible = val;
            if (val) {
                //Enforce first scaling and delay the call to scaleSession to ensure Dom is ready.
                setWrapperStyle('100%', '100%', 'hidden');
                setTimeout(() => scaleSession(scale), 150);
            }
        });
    }

    function canvasResized() {
        scaleSession(currentScreenScale);
    }

    function scaleSession(screenScale: ScreenScale | string) {
        resetHostStyle();
        if (isVisible) {
            switch (screenScale) {
                case 'fit':
                case ScreenScale.Fit:
                    loggingService.info('Size to fit');
                    currentScreenScale = ScreenScale.Fit;
                    scale = 'fit';
                    fitResize();
                    break;
                case 'full':
                case ScreenScale.Full:
                    loggingService.info('Size to full');
                    currentScreenScale = ScreenScale.Full;
                    fullResize();
                    scale = 'full';
                    break;
                case 'real':
                case ScreenScale.Real:
                    loggingService.info('Size to real');
                    currentScreenScale = ScreenScale.Real;
                    realResize();
                    scale = 'real';
                    break;
            }
        }
    }

    function fullResize() {
        const windowSize = getWindowSize();

        const containerWidth = windowSize.x;
        const containerHeight = windowSize.y;

        let width = canvas.width;
        let height = canvas.height;

        const ratio = Math.min(containerWidth / canvas.width, containerHeight / canvas.height);
        width = width * ratio;
        height = height * ratio;

        setWrapperStyle(`${containerHeight}px`, `${containerWidth}px`, 'hidden');

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
            setWrapperStyle(
                `${Math.min(containerHeight, canvas.height)}px`,
                `${Math.min(containerWidth, canvas.width)}px`,
                'auto',
            );
        } else {
            setWrapperStyle('initial', 'initial', 'initial');
        }

        setViewerStyle(`${canvas.height}px`, `${canvas.width}px`, true);
        setHostStyle(false);
    }

    function getMousePos(evt: MouseEvent) {
        const rect = canvas?.getBoundingClientRect(),
            scaleX = canvas?.width / rect.width,
            scaleY = canvas?.height / rect.height;

        const coord = {
            x: Math.round((evt.clientX - rect.left) * scaleX),
            y: Math.round((evt.clientY - rect.top) * scaleY),
        };

        remoteDesktopService.updateMousePosition(coord);
    }

    function setMouseButtonState(state: MouseEvent, isDown: boolean) {
        remoteDesktopService.mouseButtonState(state, isDown, true);
    }

    function mouseWheel(evt: WheelEvent) {
        remoteDesktopService.mouseWheel(evt);
    }

    function setMouseIn(evt: MouseEvent) {
        canvas.focus({ preventScroll: true });
        remoteDesktopService.mouseIn(evt);
    }

    function setMouseOut(evt: MouseEvent) {
        remoteDesktopService.mouseOut(evt);
    }

    function keyboardEvent(evt: KeyboardEvent) {
        remoteDesktopService.sendKeyboardEvent(evt);

        // Propagate further
        return true;
    }

    function getWindowSize() {
        const win = window;
        const doc = document;
        const docElem = doc.documentElement;
        const body = doc.getElementsByTagName('body')[0];
        const x = win.innerWidth ?? docElem.clientWidth ?? body.clientWidth;
        const y = win.innerHeight ?? docElem.clientHeight ?? body.clientHeight;
        return { x, y };
    }

    async function initcanvas() {
        loggingService.info('Start canvas initialization...');

        // Set a default canvas size. Need more test to know if i can remove it.
        canvas.width = 800;
        canvas.height = 600;

        remoteDesktopService.setCanvas(canvas);
        remoteDesktopService.setOnCanvasResized(canvasResized);

        initListeners();

        let result = { irgUserInteraction: publicAPI.getExposedFunctions() };

        loggingService.info('Component ready');
        loggingService.info('Dispatching ready event');

        // bubbles:true is significant here, all our consumer code expect this specific event
        // but they only listen to the event on the custom element itself, not on the inner div
        // in Svelte 3, we had direct access to the customelement, but now in Svelte5, we have to
        // dispatch the event on the inner div, and bubble it up to the custom element.
        inner.dispatchEvent(new CustomEvent('ready', { detail: result, bubbles: true, composed: true }));
    }

    function focusEventHandler() {
        try {
            while (runWhenFocusedQueue.length() > 0) {
                const fn = runWhenFocusedQueue.shift();
                fn?.();
            }
        } catch (err) {
            console.error('Failed to run the function queued for execution when the window received focus: ' + err);
        }
    }

    onMount(async () => {
        isComponentDestroyed.set(false);
        loggingService.verbose = verbose === 'true';
        loggingService.info('Dom ready');
        await initcanvas();
        clipboardService.initClipboard();
    });

    onDestroy(() => {
        window.removeEventListener('resize', resizeHandler);
        window.removeEventListener('focus', focusEventHandler);
        isComponentDestroyed.set(true);
    });
</script>

<div bind:this={inner}>
    <div
        bind:this={wrapper}
        class="screen-wrapper scale-{scale}"
        class:hidden={!isVisible}
        class:capturing-inputs={capturingInputs}
        style={wrapperStyle}
    >
        <div class="screen-viewer" style={viewerStyle}>
            <canvas
                bind:this={canvas}
                onmousemove={getMousePos}
                onmousedown={(event) => setMouseButtonState(event, true)}
                onmouseup={(event) => setMouseButtonState(event, false)}
                onmouseleave={(event) => {
                    setMouseButtonState(event, false);
                    setMouseOut(event);
                }}
                onmouseenter={(event) => {
                    setMouseIn(event);
                }}
                oncontextmenu={(event) => event.preventDefault()}
                onwheel={mouseWheel}
                onselectstart={(event) => {
                    event.preventDefault();
                }}
                id="renderer"
                tabindex="0"
            ></canvas>
        </div>
    </div>
</div>

<style>
    .screen-wrapper {
        position: relative;
    }

    .capturing-inputs {
        outline: 1px solid rgba(0, 97, 166, 0.7);
        outline-offset: -1px;
    }

    canvas {
        width: 100%;
        height: 100%;
    }

    ::selection {
        background-color: transparent;
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
