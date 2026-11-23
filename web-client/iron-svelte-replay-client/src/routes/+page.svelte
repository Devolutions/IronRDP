<div class="select-screen">
    <h1>Replay Player Demo</h1>
    <p>Choose a recording source</p>

    <div class="cards">
        <a href="/http" class="card">
            <h2>HTTP Byte-Range</h2>
            <p>Stream a recording from a URL.</p>
        </a>

        <a href="/file" class="card">
            <h2>Local File</h2>
            <p>Upload a <code>.bin</code> recording file.</p>
        </a>
    </div>
</div>

<style>
    .select-screen {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        min-height: 100vh;
    }

    h1 {
        margin: 0 0 4px;
    }

    .cards {
        display: flex;
        gap: 16px;
        margin-top: 32px;
    }

    .card {
        padding: 24px;
        border: 1px solid #ddd;
        border-radius: 8px;
        text-decoration: none;
        color: inherit;
        width: 200px;
    }

    .card:hover {
        border-color: #888;
    }

    .card h2 {
        margin: 0 0 4px;
        font-size: 16px;
    }

    .card p {
        margin: 0;
        font-size: 14px;
        color: #666;
    }
</style>
