<script lang="ts">
    import { page } from '$app/state';

    let { children } = $props();

    const isHome = $derived(page.url.pathname === '/');
</script>

{#if !isHome}
    <nav>
        <a href="/">&larr; Back</a>
    </nav>
{/if}

{@render children()}

<style>
    :global(body) {
        margin: 0;
        font-family: system-ui, -apple-system, sans-serif;
    }

    nav {
        padding: 12px 16px;
        border-bottom: 1px solid #ddd;
    }

    nav a {
        color: inherit;
        text-decoration: none;
        font-size: 14px;
    }

    nav a:hover {
        text-decoration: underline;
    }
</style>
