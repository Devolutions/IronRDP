const assert = require('node:assert/strict');
const { beforeEach, test } = require('node:test');

// An RDCleanPath response selecting Standard RDP Security with no certificate.
const response = Buffer.from(
    '302ea00402020d3ea6150413030000130ed000000000000200080000000000a7023000a90b0c093132372e302e302e31',
    'hex',
);
const sent = [];

global.HTMLCanvasElement = class HTMLCanvasElement {};
global.WebSocket = class WebSocket {
    constructor() {
        this.readyState = 1;
        this.listeners = new Map();
    }

    addEventListener(type, callback) {
        this.listeners.set(type, callback);
    }

    removeEventListener() {}
    close() {}

    send(data) {
        sent.push(data);
        if (sent.length === 1) {
            const reply = Uint8Array.from(response).buffer;
            queueMicrotask(() => this.listeners.get('message')(new MessageEvent('message', { data: reply })));
        } else {
            throw new Error('intercepted connection settings');
        }
    }
};

const { Extension, SessionBuilder } = require('../../../target/ironrdp-web-tests/ironrdp_web.js');

beforeEach(() => {
    sent.length = 0;
});

async function clientCore(t, extensions) {
    const builder = new SessionBuilder();
    t.after(() => builder.free());
    builder.username('user').free();
    builder.password('password').free();
    builder.destination('server:3389').free();
    builder.proxyAddress('wss://example.invalid').free();
    builder.authToken('test').free();
    builder.renderCanvas(new HTMLCanvasElement()).free();
    builder.setCursorStyleCallback(() => {}).free();
    builder.setCursorStyleCallbackContext(null).free();
    builder.extension(new Extension('enable_standard_rdp_security', true)).free();
    for (const [name, value] of extensions) {
        builder.extension(new Extension(name, value)).free();
    }

    try {
        await builder.connect();
        assert.fail('expected the intercepted connection to fail');
    } catch (error) {
        if (typeof error.backtrace !== 'function') {
            throw error;
        }
        const message = error.backtrace();
        error.free();
        assert.match(message, /intercepted connection settings/);
    }

    assert.equal(sent.length, 2);
    const frame = Buffer.from(await sent[1].arrayBuffer());
    const key = frame.indexOf(Buffer.from('Duca'));
    assert.notEqual(key, -1, 'missing client GCC data');
    const offset = frame.indexOf(Buffer.from([0x01, 0xc0]), key + 4);
    assert.notEqual(offset, -1, 'missing TS_UD_CS_CORE');
    const length = frame.readUInt16LE(offset + 2);
    assert.ok(length >= 234, 'missing display scale fields');
    const core = frame.subarray(offset, offset + length);
    assert.equal(core.length, length);
    // oracle: spec MS-RDPBCGR §2.2.1.3.2 defines these fields and the 32-bpp flag.
    const highColorDepth = core.readUInt16LE(140);
    const wants32Bpp = (core.readUInt16LE(144) & 0x0002) !== 0;
    return {
        desktopScale: core.readUInt32LE(226),
        deviceScale: core.readUInt32LE(230),
        colorDepth: wants32Bpp ? 32 : highColorDepth,
    };
}

for (const [desktopScale, deviceScale] of [
    [100, 100],
    [125, 100],
    [140, 140],
    [175, 140],
    [180, 180],
    [500, 180],
]) {
    test(`desktop scale ${desktopScale} uses device scale ${deviceScale}`, { timeout: 5000 }, async (t) => {
        // oracle: regression #2421 reports inconsistent device-scale plateaus.
        // oracle: spec MS-RDPBCGR §2.2.1.3.2 defines the valid desktop-scale range.
        const core = await clientCore(t, [['desktop_scale_factor', desktopScale]]);
        assert.equal(core.desktopScale, desktopScale);
        assert.equal(core.deviceScale, deviceScale);
    });
}

for (const [name, invalid] of [
    ['below the range', 99],
    ['above the range', 501],
    ['fractional', 125.5],
    ['NaN', NaN],
    ['infinite', Infinity],
    ['non-numeric', '150'],
]) {
    test(`${name} desktop scale leaves the previous setting unchanged`, { timeout: 5000 }, async (t) => {
        // oracle: regression #2421 specifies that invalid extensions are ignored.
        const core = await clientCore(t, [
            ['desktop_scale_factor', 125],
            ['desktop_scale_factor', invalid],
        ]);
        assert.equal(core.desktopScale, 125);
        assert.equal(core.deviceScale, 100);
    });
}

for (const [legacy, expected] of [
    [false, 16],
    [true, 32],
]) {
    test(`default color depth is ${expected} with legacy graphics ${legacy}`, { timeout: 5000 }, async (t) => {
        // oracle: regression #2421 states the existing color-depth defaults.
        const core = await clientCore(t, [['legacy_graphics', legacy]]);
        assert.equal(core.colorDepth, expected);
    });
}

for (const [colorDepth, legacy] of [
    [15, true],
    [16, true],
    [24, true],
    [32, false],
]) {
    test(`color depth ${colorDepth} overrides the graphics default`, { timeout: 5000 }, async (t) => {
        // oracle: spec MS-RDPBCGR §2.2.1.3.2 defines the requested color-depth encodings.
        const core = await clientCore(t, [
            ['legacy_graphics', legacy],
            ['color_depth', colorDepth],
        ]);
        assert.equal(core.colorDepth, colorDepth);
    });
}

for (const [name, invalid] of [
    ['unsupported', 17],
    ['fractional', 24.5],
    ['NaN', NaN],
    ['non-numeric', '32'],
]) {
    test(`${name} color depth leaves the previous setting unchanged`, { timeout: 5000 }, async (t) => {
        // oracle: regression #2421 specifies that invalid extensions are ignored.
        const core = await clientCore(t, [
            ['color_depth', 24],
            ['color_depth', invalid],
        ]);
        assert.equal(core.colorDepth, 24);
    });
}
