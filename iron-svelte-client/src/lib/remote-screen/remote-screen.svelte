<script>
    import {onMount} from 'svelte';
    import {userInteractionService} from '../../services/session.service';

    let uiService;

    userInteractionService.subscribe(uis => uiService = uis);

    onMount(async () => {
        let el = document.querySelector('iron-remote-gui');
        el.addEventListener('ready', (e) => {
            console.log("ready");
            userInteractionService.set(e.detail.irgUserInteraction);
        });
    });
</script>
<div style="display: flex; height: 100%; flex-direction: column">
    <div>
        <button on:click={() => uiService.setScale(1)}>Fit</button>
        <button on:click={() => uiService.setScale(2)}>Full</button>
        <button on:click={() => uiService.setScale(3)}>Real</button>
    </div>
    <iron-remote-gui debugwasm="INFO" verbose="false" scale="fit" flexcenter="true"
                     targetplatform="{import.meta.env.MODE}"/>
</div>