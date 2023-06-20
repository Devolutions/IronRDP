<script lang="ts">
    import {onMount} from 'svelte';
    import {
        setCurrentSessionActive,
        userInteractionService
    } from '../../services/session.service';
    import {showLogin} from '$lib/login/login-store';

    let uiService;

    userInteractionService.subscribe(uis => {
        if (uis) {
            uiService = uis
            uiService.sessionListener.subscribe(event => {
                if (event.type === 0) {
                    uiService.setVisibility(true);
                } else if (event.type === 1) {
                    setCurrentSessionActive(false);
                    showLogin.set(true);
                }
            })
        }
    });

    onMount(async () => {
        let el = document.querySelector('iron-remote-gui');
        el.addEventListener('ready', (e) => {
            userInteractionService.set(e.detail.irgUserInteraction);
        });
    });
</script>
<div style="display: flex; height: 100%; flex-direction: column; background-color: #2e2e2e;" class:hideall={$showLogin}>
    <div style="text-align: center; padding: 10px; background: black;">
        <button on:click={() => uiService.setScale(1)}>Fit</button>
        <button on:click={() => uiService.setScale(2)}>Full</button>
        <button on:click={() => uiService.setScale(3)}>Real</button>
        <button on:click={() => uiService.ctrlAltDel()}>Ctrl+Alt+Del</button>
        <button on:click={() => uiService.metaKey()}>Meta
            <svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 512 512"><title>
                ionicons-v5_logos</title>
                <path d="M480,265H232V444l248,36V265Z"/>
                <path d="M216,265H32V415l184,26.7V265Z"/>
                <path d="M480,32,232,67.4V249H480V32Z"/>
                <path d="M216,69.7,32,96V249H216V69.7Z"/>
            </svg>
        </button>
        <button on:click={() => uiService.shutdown()}>Terminate Session</button>
    </div>
    <iron-remote-gui debugwasm="INFO" verbose="true" scale="fit" flexcenter="true"
                     targetplatform="{import.meta.env.MODE}"/>
</div>

<style>
    .hideall {
        display: none !important;
    }
</style>