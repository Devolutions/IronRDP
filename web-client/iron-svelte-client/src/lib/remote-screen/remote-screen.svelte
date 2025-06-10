<script lang="ts">
    import { onMount } from 'svelte';
    import { setCurrentSessionActive, userInteractionService } from '../../services/session.service';
    import { showLogin } from '$lib/login/login-store';
    import type { UserInteraction } from '../../../static/iron-remote-desktop';
    import { Backend } from '../../../static/iron-remote-desktop-rdp';

    let uiService: UserInteraction;
    let cursorOverrideActive = false;
    let showDebugPanel = false;

    userInteractionService.subscribe((uis) => {
        if (uis != null) {
            uiService = uis;
            uiService.onSessionEvent((event) => {
                if (event.type === 0) {
                    uiService.setVisibility(true);
                } else if (event.type === 1) {
                    setCurrentSessionActive(false);
                    showLogin.set(true);
                }
            });
        }
    });

    function onUnicodeModeChange(e: MouseEvent) {
        if (e.target == null) {
            return;
        }

        let element = e.target as HTMLInputElement;

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

    onMount(async () => {
        let el = document.querySelector('iron-remote-desktop');

        if (el == null) {
            throw '`iron-remote-desktop` element not found';
        }

        el.addEventListener('ready', (e) => {
            const event = e as CustomEvent;
            userInteractionService.set(event.detail.irgUserInteraction);
        });
    });
</script>

<div style="display: flex; height: 100%; flex-direction: column; background-color: #2e2e2e;" class:hideall={$showLogin}>
    <div>
        <div style="text-align: center; padding: 10px; background: black;">
            <button on:click={() => (showDebugPanel = !showDebugPanel)}>Toggle debug panel</button>
            <button on:click={() => uiService.setScale(1)}>Fit</button>
            <button on:click={() => uiService.setScale(2)}>Full</button>
            <button on:click={() => uiService.setScale(3)}>Real</button>
            <button on:click={() => uiService.ctrlAltDel()}>Ctrl+Alt+Del</button>
            <button on:click={() => uiService.metaKey()}
                >Meta
                <svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 512 512"
                    ><title> ionicons-v5_logos</title>
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

        {#if showDebugPanel}
            <div id="debug-panel" style="background: black; color: white; padding: 10px;">
                debug-panel
                <input
                    type="text"
                    id="debug-panel-input"
                    style="width: 100%; height: 100%; background: black; color: white;"
                    placeholder="see if focus moves correctly"
                />

                <p>see if text selection works correctly</p>
            </div>
        {/if}
    </div>
    <iron-remote-desktop verbose="true" scale="fit" flexcenter="true" module={Backend} />
</div>

<style>
    .hideall {
        display: none !important;
    }
</style>
