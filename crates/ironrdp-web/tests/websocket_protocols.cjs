const assert = require('node:assert/strict');
const { beforeEach, test } = require('node:test');

const calls = [];
let failure;

// A closed socket stops connect() before the RDP handshake, while recording
// the constructor arguments passed by the real WASM implementation.
global.HTMLCanvasElement = class HTMLCanvasElement {};
global.WebSocket = class WebSocket {
    constructor(url, protocols) {
        if (failure === 'constructor') {
            throw new SyntaxError('invalid protocols');
        }
        calls.push({ url, argc: arguments.length, protocols: protocols && [...protocols] });
        this.readyState = 3;
    }

    addEventListener() {
        if (failure === 'setup') {
            throw new Error('listener setup failed');
        }
    }

    removeEventListener() {}
    close() {}
};

const { SessionBuilder } = require('../../../target/ironrdp-web-tests/ironrdp_web.js');

beforeEach(() => {
    calls.length = 0;
    failure = undefined;
});

function builder(t) {
    const b = new SessionBuilder();
    t.after(() => b.free());
    b.username('user').free();
    b.password('password').free();
    b.destination('server:3389').free();
    b.proxyAddress('wss://example.invalid').free();
    b.authToken('test').free();
    b.renderCanvas(new HTMLCanvasElement()).free();
    b.setCursorStyleCallback(() => {}).free();
    b.setCursorStyleCallbackContext(null).free();
    return b;
}

async function rejection(b) {
    try {
        await b.connect();
    } catch (error) {
        if (typeof error.backtrace !== 'function') {
            throw error;
        }
        try {
            return error.backtrace();
        } finally {
            error.free();
        }
    }
    assert.fail('expected connect() to reject');
}

async function offered(b) {
    const message = await rejection(b);
    assert.match(message, /failed to connect.*WebSocket is `Closed`/);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, 'wss://example.invalid');
    return calls[0];
}

test('omitted protocols use the one-argument WebSocket constructor', async (t) => {
    assert.equal((await offered(builder(t))).argc, 1);
});

test('empty protocols use the one-argument WebSocket constructor', async (t) => {
    const b = builder(t);
    b.webSocketProtocols([]).free();
    assert.equal((await offered(b)).argc, 1);
});

test('configured protocols preserve their values and order', async (t) => {
    const b = builder(t);
    b.webSocketProtocols(['binary', 'v2']).free();
    const call = await offered(b);
    assert.equal(call.argc, 2);
    assert.deepEqual(call.protocols, ['binary', 'v2']);
});

test('caller replacement and append cannot change saved protocols', async (t) => {
    const b = builder(t);
    const input = ['binary'];
    b.webSocketProtocols(input).free();
    input[0] = 42;
    input.push('extra');
    assert.deepEqual((await offered(b)).protocols, ['binary']);
});

test('clearing the caller array cannot clear saved protocols', async (t) => {
    const b = builder(t);
    const input = ['binary'];
    b.webSocketProtocols(input).free();
    input.length = 0;
    assert.deepEqual((await offered(b)).protocols, ['binary']);
});

test('non-string entries reject before constructing a WebSocket', async (t) => {
    for (const value of [42, false, null, undefined, {}, ['binary']]) {
        const b = builder(t);
        b.webSocketProtocols(['binary', value]).free();
        assert.equal(await rejection(b), 'websocket protocols must be strings');
        assert.equal(calls.length, 0);
    }
});

test('invalid input replaces valid settings and remains an error across connections', async (t) => {
    const b = builder(t);
    const input = [false];
    b.webSocketProtocols(['binary']).free();
    b.webSocketProtocols(input).free();
    input[0] = 'fixed by caller';
    for (let attempt = 0; attempt < 2; attempt++) {
        assert.equal(await rejection(b), 'websocket protocols must be strings');
        assert.equal(calls.length, 0);
    }
});

test('a valid setter recovers from invalid input', async (t) => {
    const b = builder(t);
    b.webSocketProtocols([null]).free();
    b.webSocketProtocols(['v2']).free();
    assert.deepEqual((await offered(b)).protocols, ['v2']);
});

test('an empty setter clears an error and restores the default constructor', async (t) => {
    const b = builder(t);
    b.webSocketProtocols([undefined]).free();
    b.webSocketProtocols([]).free();
    assert.equal((await offered(b)).argc, 1);
});

test('an empty setter clears previously valid protocols', async (t) => {
    const b = builder(t);
    b.webSocketProtocols(['binary']).free();
    b.webSocketProtocols([]).free();
    assert.equal((await offered(b)).argc, 1);
});

test('appending to an empty caller array keeps the default constructor', async (t) => {
    const b = builder(t);
    const input = [];
    b.webSocketProtocols(input).free();
    input.push('binary');
    assert.equal((await offered(b)).argc, 1);
});

test('a subsequent valid setter replaces the offered protocols', async (t) => {
    const b = builder(t);
    b.webSocketProtocols(['v1']).free();
    b.webSocketProtocols(['v2']).free();
    assert.deepEqual((await offered(b)).protocols, ['v2']);
});

test('each entry is read once and its validated value is stored', async (t) => {
    const b = builder(t);
    const input = ['binary'];
    let reads = 0;
    Object.defineProperty(input, 0, {
        get() {
            return ++reads === 1 ? 'binary' : 42;
        },
    });
    b.webSocketProtocols(input).free();
    assert.deepEqual((await offered(b)).protocols, ['binary']);
    assert.equal(reads, 1);
});

test('constructor failure has one WebSocket error prefix', async (t) => {
    const b = builder(t);
    b.webSocketProtocols(['binary']).free();
    failure = 'constructor';
    const message = await rejection(b);
    assert.equal((message.match(/couldn't open WebSocket/g) || []).length, 1);
    assert.match(message, /invalid protocols/);
});

test('wrapper setup failure has one WebSocket error prefix', async (t) => {
    const b = builder(t);
    b.webSocketProtocols(['binary']).free();
    failure = 'setup';
    const message = await rejection(b);
    assert.equal((message.match(/couldn't open WebSocket/g) || []).length, 1);
    assert.match(message, /listener setup failed/);
});
