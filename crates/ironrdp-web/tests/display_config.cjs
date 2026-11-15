const assert = require('node:assert/strict');
const { beforeEach, test } = require('node:test');

// An RDCleanPath response selecting Standard RDP Security and extended client data.
const extendedResponse = Buffer.from(
    '302ea00402020d3ea6150413030000130ed000000000000201080000000000a7023000a90b0c093132372e302e302e31',
    'hex',
);
const legacyResponse = Buffer.from(
    '302ea00402020d3ea6150413030000130ed000000000000200080000000000a7023000a90b0c093132372e302e302e31',
    'hex',
);
const sent = [];
let response;

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

const { DesktopSize, Extension, SessionBuilder } = require('../../../target/ironrdp-web-tests/ironrdp_web.js');

beforeEach(() => {
    sent.length = 0;
    response = extendedResponse;
});

async function clientCore(t, extensions, desktopSize) {
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
    if (desktopSize) {
        builder.desktopSize(new DesktopSize(...desktopSize)).free();
    }
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
    let monitorLayout = null;
    for (let position = offset; position + 4 <= frame.length; ) {
        const type = frame.readUInt16LE(position);
        if (type < 0xc001 || 0xc00a < type) break;
        const blockLength = frame.readUInt16LE(position + 2);
        assert.ok(blockLength >= 4 && position + blockLength <= frame.length);
        if (type === 0xc005) {
            const block = frame.subarray(position, position + blockLength);
            const count = block.readUInt32LE(8);
            assert.equal(blockLength, 12 + count * 20);
            monitorLayout = [];
            for (let i = 0; i < count; i++) {
                const start = 12 + i * 20;
                monitorLayout.push(
                    block.readInt32LE(start),
                    block.readInt32LE(start + 4),
                    block.readInt32LE(start + 8),
                    block.readInt32LE(start + 12),
                    block.readUInt32LE(start + 16),
                );
            }
        }
        position += blockLength;
    }
    // oracle: spec MS-RDPBCGR §2.2.1.3.2 defines these fields and the 32-bpp flag.
    const highColorDepth = core.readUInt16LE(140);
    const wants32Bpp = (core.readUInt16LE(144) & 0x0002) !== 0;
    return {
        desktopWidth: core.readUInt16LE(8),
        desktopHeight: core.readUInt16LE(10),
        desktopScale: core.readUInt32LE(226),
        deviceScale: core.readUInt32LE(230),
        colorDepth: wants32Bpp ? 32 : highColorDepth,
        monitorLayout,
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

test('a valid monitor layout is advertised in client monitor data', { timeout: 5000 }, async (t) => {
    // oracle: spec MS-RDPBCGR §2.2.1.3.6.1 defines inclusive coordinates and primary flags.
    const layout = [0, 0, 799, 599, 1, -800, 0, -1, 599, 0];
    const core = await clientCore(t, [['monitor_layout', layout]]);
    assert.deepEqual(core.monitorLayout, layout);
});

test('monitor data is omitted when the server does not support extended client data', { timeout: 5000 }, async (t) => {
    // oracle: spec MS-RDPBCGR §2.2.1.3.6 forbids monitor data without extended-client-data support.
    response = legacyResponse;
    const core = await clientCore(t, [['monitor_layout', [0, 0, 799, 599, 1]]]);
    assert.equal(core.monitorLayout, null);
});

test('the requested desktop size covers the monitor layout', { timeout: 5000 }, async (t) => {
    // oracle: regression #2423 requires the desktop size to describe the monitor union.
    const layout = [0, 0, 799, 599, 1, -800, 0, -1, 599, 0];
    const core = await clientCore(t, [['monitor_layout', layout]], [900, 700]);
    assert.deepEqual([core.desktopWidth, core.desktopHeight], [1600, 600]);
});

test('a session without a monitor layout keeps its configured desktop size', { timeout: 5000 }, async (t) => {
    // oracle: regression #2423 adds an optional monitor layout without changing ordinary sessions.
    const core = await clientCore(t, [], [900, 700]);
    assert.equal(core.monitorLayout, null);
    assert.deepEqual([core.desktopWidth, core.desktopHeight], [900, 700]);
});

test('an invalid first layout does not change the configured desktop', { timeout: 5000 }, async (t) => {
    // oracle: regression #2423 specifies that an invalid layout is ignored.
    const core = await clientCore(t, [['monitor_layout', []]], [900, 700]);
    assert.equal(core.monitorLayout, null);
    assert.deepEqual([core.desktopWidth, core.desktopHeight], [900, 700]);
});

test('the maximum number of monitors and desktop dimension are accepted', { timeout: 5000 }, async (t) => {
    // oracle: spec MS-RDPBCGR §2.2.1.3.6 permits 16 monitors and a desktop width of 32,766.
    const layout = Array.from({ length: 16 }, (_, i) => [
        i * 200,
        0,
        i === 15 ? 32765 : i * 200 + 199,
        199,
        i === 0 ? 1 : 0,
    ]).flat();
    const core = await clientCore(t, [['monitor_layout', layout]]);
    assert.deepEqual(core.monitorLayout, layout);
    assert.deepEqual([core.desktopWidth, core.desktopHeight], [32766, 200]);
});

test('unknown high monitor flag bits survive the JavaScript boundary', { timeout: 5000 }, async (t) => {
    // oracle: regression #1144 preserves unknown flags in monitor definitions.
    const layout = [0, 0, 199, 199, 0x80000001];
    const core = await clientCore(t, [['monitor_layout', layout]]);
    assert.deepEqual(core.monitorLayout, layout);
});

for (const [name, invalid] of [
    ['null', null],
    ['undefined', undefined],
    ['non-callable iterator', { [Symbol.iterator]: 0 }],
    [
        'throwing iterator',
        {
            [Symbol.iterator]() {
                throw new Error('invalid iterator');
            },
        },
    ],
    ['empty', []],
    ['incomplete', [0, 0, 799, 599]],
    [
        'more than 16 monitors',
        Array.from({ length: 17 }, (_, i) => [i * 200, 0, i * 200 + 199, 199, i === 0 ? 1 : 0]).flat(),
    ],
    ['non-numeric coordinate', [0, 0, '799', 599, 1]],
    ['fractional coordinate', [0, 0, 799.5, 599, 1]],
    ['non-finite coordinate', [0, 0, Infinity, 599, 1]],
    ['out-of-range coordinate', [0, 0, 2147483648, 599, 1]],
    ['negative flags', [0, 0, 799, 599, -1]],
    ['out-of-range flags', [0, 0, 799, 599, 4294967296]],
    ['missing primary', [0, 0, 799, 599, 0]],
    ['multiple primaries', [0, 0, 799, 599, 1, 0, 0, 799, 599, 1]],
    ['primary away from the origin', [1, 0, 800, 599, 1]],
    ['inverted rectangle', [0, 0, 799, 599, 1, 100, 0, 99, 199, 0]],
    ['desktop too narrow', [0, 0, 198, 199, 1]],
    ['desktop too short', [0, 0, 199, 198, 1]],
    ['desktop too wide', [0, 0, 32766, 199, 1]],
    ['desktop too tall', [0, 0, 199, 32766, 1]],
    ['extreme desktop coordinates', [0, 0, 199, 199, 1, -2147483648, 0, -2147483449, 199, 0]],
]) {
    test(`${name} layout leaves the previous layout unchanged`, { timeout: 5000 }, async (t) => {
        // oracle: spec MS-RDPBCGR §2.2.1.3.6 and §2.2.1.3.6.1 define the layout bounds and fields.
        // oracle: regression #2423 specifies that an invalid layout is ignored.
        const layout = [0, 0, 799, 599, 1];
        const core = await clientCore(t, [
            ['monitor_layout', layout],
            ['monitor_layout', invalid],
        ]);
        assert.deepEqual(core.monitorLayout, layout);
    });
}

test('an oversized iterable is rejected without consuming it indefinitely', { timeout: 5000 }, async (t) => {
    // oracle: spec MS-RDPBCGR §2.2.1.3.6 limits the layout to 16 monitors.
    let consumed = 0;
    const invalid = {
        *[Symbol.iterator]() {
            while (true) {
                consumed++;
                if (consumed > 81) throw new Error('monitor list was not bounded');
                yield 0;
            }
        },
    };
    const layout = [0, 0, 799, 599, 1];
    const core = await clientCore(t, [
        ['monitor_layout', layout],
        ['monitor_layout', invalid],
    ]);
    assert.deepEqual(core.monitorLayout, layout);
    // oracle: mutant unbounded iteration of layouts exceeding the protocol limit.
    assert.ok(consumed <= 81);
});

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
    ['below the supported depths', 14],
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
