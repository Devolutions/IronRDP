<script lang="ts">
    import { onMount } from 'svelte';
    import { setCurrentSessionActive, userInteractionService } from '../../services/session.service';
    import type { UserInteraction } from '../../../static/iron-remote-desktop';
    import RemoteDesktop from '../../../static/iron-remote-desktop-rdp';

    let uiService: UserInteraction;
    let cursorOverrideActive = false;
    let showUtilityBar = false;

    userInteractionService.subscribe((uis) => {
        if (uis != null) {
            uiService = uis;
            uiService.onSessionEvent((event) => {
                if (event.type === 0) {
                    uiService.setVisibility(true);
                } else if (event.type === 1) {
                    setCurrentSessionActive(false);
                }
            });
        }
    });

    userInteractionService.subscribe((uis) => {
        if (uis != null) {
            uiService = uis;
            //read query params named data
            const urlParams = new URLSearchParams(window.location.search);
            const data = urlParams.get('data');
            if (data == null) {
                console.warn('No data found in query params');
                return;
            }

            const parsedData = JSON.parse(atob(data));
            const { hostname, gatewayAddress, domain, username, password, authtoken, kdc_proxy_url, pcb, desktopSize } =
                parsedData;
            uiService
                .connect(
                    username,
                    password,
                    hostname,
                    gatewayAddress,
                    domain,
                    authtoken,
                    desktopSize,
                    pcb,
                    kdc_proxy_url,
                    true,
                )
                .then(() => {
                    uiService.setVisibility(true);
                    window.onresize = onWindowResize;
                });
        }
    });

    function onWindowResize() {
        const innerWidth = window.innerWidth;
        const innerHeight = window.innerHeight;
        uiService.resize(innerWidth, innerHeight);
    }

    function onUnicodeModeChange(e: MouseEvent) {
        if (e.target == null) {
            return;
        }

        const element = e.target as HTMLInputElement;

        if (element == null) {
            return;
        }

        uiService.setKeyboardUnicodeMode(element.checked);
    }

    function toggleCursorKind() {
        if (cursorOverrideActive) {
            uiService.setCursorStyleOverride(null);
        } else {
            uiService.setCursorStyleOverride('url("crosshair.png") 7 7, default');
        }

        cursorOverrideActive = !cursorOverrideActive;
    }

    function toggleFullScreen() {
        if (document.fullscreenElement) {
            document.exitFullscreen();
        } else {
            document.documentElement.requestFullscreen();
        }
    }

    onMount(async () => {
        const el = document.querySelector('iron-remote-desktop');

        if (el == null) {
            throw '`iron-remote-desktop` element not found';
        }

        el.addEventListener('ready', (e) => {
            const event = e as CustomEvent;
            userInteractionService.set(event.detail.irgUserInteraction);
        });
    });
</script>

<div
    id="popup-screen"
    style="display: flex; height: 100%; flex-direction: column; background-color: #2e2e2e; position: relative"
    on:mousemove={(event) => {
        showUtilityBar = event.clientY < 100;
    }}
>
    <div class="tool-bar" class:hidden={!showUtilityBar}>
        <div class="toolbar-container">
            <button on:click={() => toggleFullScreen()}>Full Screen</button>
            <button on:click={() => uiService.ctrlAltDel()}>Ctrl+Alt+Del</button>
            <button on:click={() => uiService.metaKey()}>
                Meta
                <svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 512 512">
                    <title>ionicons-v5_logos</title>
                    <path d="M480,265H232V444l248,36V265Z" />
                    <path d="M216,265H32V415l184,26.7V265Z" />
                    <path d="M480,32,232,67.4V249H480V32Z" />
                    <path d="M216,69.7,32,96V249H216V69.7Z" />
                </svg>
            </button>
            <button on:click={() => toggleCursorKind()}>Toggle cursor kind</button>
            <button on:click={() => uiService.shutdown()}>Terminate Session</button>
            <label style="color: white;">
                <input on:click={(e) => onUnicodeModeChange(e)} type="checkbox" />
                Unicode keyboard mode
            </label>
        </div>
    </div>
    <iron-remote-desktop debugwasm="INFO" verbose="true" scale="fit" flexcenter="true" module={RemoteDesktop} />
</div>

<style>
    .tool-bar {
        position: absolute;
        top: 0;
        left: 50%;
        transform: translateX(-50%);
        width: 50%;
        background: rgba(0, 0, 0, 0.7); /* 70% opacity */
        color: white;
        z-index: 100;
        display: flex;
        justify-content: center;
        padding: 10px;
        border-radius: 8px;
    }

    .toolbar-container {
        display: flex;
        gap: 10px; /* Spacing between buttons */
    }

    button {
        background-color: #444;
        color: white;
        padding: 8px 12px;
        border: none;
        border-radius: 4px;
        font-size: 0.9em; /* Smaller button size */
        cursor: pointer;
    }

    button svg {
        vertical-align: middle;
    }

    button:hover {
        background-color: #666;
    }

    .hidden {
        display: none;
    }
</style>
