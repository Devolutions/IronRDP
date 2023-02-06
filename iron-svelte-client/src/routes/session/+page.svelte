<script>
	import Login from '$lib/login/login.svelte';
	import RemoteScreen from '$lib/remote-screen/remote-screen.svelte';
	import { currentSession } from '../../services/session.service';

	let showLogin = true;
	
	currentSession.subscribe(v => {
		showLogin = !v.active
	});
</script>
	{#if showLogin}
		<Login />
	{/if}
	<RemoteScreen />
