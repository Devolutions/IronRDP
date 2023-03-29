<script lang="ts">
    import Login from '$lib/login/login.svelte';
    import RemoteScreen from '$lib/remote-screen/remote-screen.svelte';
    import Message from '$lib/messages/message.svelte';
    import {showLogin} from "$lib/login/login-store.ts";

</script>

{#if $showLogin}
    <Login/>
{/if}
<RemoteScreen/>

<Message></Message>

