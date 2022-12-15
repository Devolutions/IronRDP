<script>
	import Login from '$lib/login/login.svelte';
	import RemoteScreen from '$lib/remote-screen/remote-screen.svelte';
    import { currentSession } from '../../services/session.service';
</script>
{#if !$currentSession?.active}
	<Login />
{:else}
	<RemoteScreen />
{/if}
