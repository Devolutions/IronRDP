<script lang="ts">
    import { initPlayer, type IronReplayPlayerElement } from '$lib/initPlayer.js';
    import { LocalFileDataSource } from '$lib/LocalFileDataSource.js';

    let loaded = $state(false);
    let error = $state('');
    let playerEl: IronReplayPlayerElement | null = $state(null);

    async function handleFile(e: Event) {
        const input = e.target as HTMLInputElement;
        const file = input.files?.[0];
        if (!file) return;

        error = '';
        loaded = true;

        const err = await initPlayer(() => playerEl, new LocalFileDataSource(file));
        if (err) {
            error = err;
            loaded = false;
        }
    }
</script>

{#if !loaded}
    <div class="input-screen">
        <h1>Local File</h1>
        <p>Select a <code>.bin</code> recording file.</p>
        <p class="hint">A sample recording is available at <code>samples/sample.bin</code></p>

        <label class="file-picker">
            <input type="file" accept=".bin" onchange={handleFile} />
            <span>Choose File</span>
        </label>

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

    .file-picker {
        cursor: pointer;
    }

    .file-picker input {
        display: none;
    }

    .file-picker span {
        display: inline-block;
        padding: 8px 16px;
        font-size: 14px;
        border: 1px solid #ddd;
        border-radius: 4px;
        background: #fafafa;
    }

    .file-picker span:hover {
        border-color: #888;
    }

    .player-container {
        width: 100vw;
        height: calc(100vh - 45px);
    }
</style>
