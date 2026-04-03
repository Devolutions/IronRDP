<script lang="ts">
    import { initPlayer, type IronReplayPlayerElement } from '$lib/initPlayer.js';
    import { HttpRangeDataSource } from '$lib/HttpRangeDataSource.js';

    // Demo default — served by: npm run replay-server
    const DEFAULT_URL = 'http://localhost:8000/sample.bin';

    let url = $state(DEFAULT_URL);
    let loaded = $state(false);
    let error = $state('');
    let playerEl: IronReplayPlayerElement | null = $state(null);

    async function load() {
        if (!url.trim()) return;

        error = '';
        loaded = true;

        const err = await initPlayer(() => playerEl, new HttpRangeDataSource(url.trim()));
        if (err) {
            error = err;
            loaded = false;
        }
    }

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === 'Enter') load();
    }
</script>

{#if !loaded}
    <div class="input-screen">
        <h1>HTTP Byte-Range</h1>
        <p>Enter the URL of a <code>.bin</code> recording.</p>
        <p class="hint">Start the replay server: <code>npm run replay-server</code></p>

        <div class="input-row">
            <input type="text" bind:value={url} onkeydown={handleKeydown} />
            <button onclick={load}>Load</button>
        </div>

        {#if error}
            <p class="error">{error}</p>
        {/if}
    </div>
{:else}
    <div class="player-container">
        <iron-replay-player bind:this={playerEl}></iron-replay-player>
    </div>
{/if}

<style>
    .input-screen {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        min-height: calc(100vh - 45px);
    }

    h1 {
        margin: 0 0 4px;
    }

    p {
        color: #666;
        font-size: 14px;
        margin: 0 0 16px;
    }

    .hint {
        color: #999;
        font-size: 12px;
        font-family: monospace;
        margin: 0 0 12px;
    }

    .hint code {
        background: #f5f5f5;
        padding: 2px 4px;
        border-radius: 2px;
    }

    .error {
        color: #c00;
        font-size: 13px;
        margin: 12px 0 0;
        max-width: 600px;
        word-break: break-word;
    }

    .input-row {
        display: flex;
        gap: 8px;
        width: 100%;
        max-width: 600px;
        padding: 0 16px;
        box-sizing: border-box;
    }

    input {
        flex: 1;
        padding: 8px;
        font-family: monospace;
        font-size: 14px;
        border: 1px solid #ddd;
        border-radius: 4px;
    }

    button {
        padding: 8px 16px;
        font-size: 14px;
        border: 1px solid #ddd;
        border-radius: 4px;
        background: #fafafa;
        cursor: pointer;
    }

    button:hover {
        border-color: #888;
    }

    .player-container {
        width: 100vw;
        height: calc(100vh - 45px);
    }
</style>
