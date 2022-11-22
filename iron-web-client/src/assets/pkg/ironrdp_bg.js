import * as wasm from './ironrdp_bg.wasm';

const lTextDecoder = typeof TextDecoder === 'undefined' ? (0, module.require)('util').TextDecoder : TextDecoder;

let cachedTextDecoder = new lTextDecoder('utf-8', { ignoreBOM: true, fatal: true });

cachedTextDecoder.decode();

let cachedUint8Memory0 = new Uint8Array();

function getUint8Memory0() {
    if (cachedUint8Memory0.byteLength === 0) {
        cachedUint8Memory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8Memory0;
}

function getStringFromWasm0(ptr, len) {
    return cachedTextDecoder.decode(getUint8Memory0().subarray(ptr, ptr + len));
}

const heap = new Array(32).fill(undefined);

heap.push(undefined, null, true, false);

let heap_next = heap.length;

function addHeapObject(obj) {
    if (heap_next === heap.length) heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];

    heap[idx] = obj;
    return idx;
}

function getObject(idx) { return heap[idx]; }

function dropObject(idx) {
    if (idx < 36) return;
    heap[idx] = heap_next;
    heap_next = idx;
}

function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
}

let cachedInt32Memory0 = new Int32Array();

function getInt32Memory0() {
    if (cachedInt32Memory0.byteLength === 0) {
        cachedInt32Memory0 = new Int32Array(wasm.memory.buffer);
    }
    return cachedInt32Memory0;
}

function getArrayU8FromWasm0(ptr, len) {
    return getUint8Memory0().subarray(ptr / 1, ptr / 1 + len);
}
/**
* @returns {DesktopSize}
*/
export function get_desktop_size() {
    const ret = wasm.get_desktop_size();
    return DesktopSize.__wrap(ret);
}

/**
* @returns {Rect}
*/
export function dump_sample() {
    const ret = wasm.dump_sample();
    return Rect.__wrap(ret);
}

/**
* @returns {Rect}
*/
export function next_rect() {
    const ret = wasm.next_rect();
    return Rect.__wrap(ret);
}

let WASM_VECTOR_LEN = 0;

const lTextEncoder = typeof TextEncoder === 'undefined' ? (0, module.require)('util').TextEncoder : TextEncoder;

let cachedTextEncoder = new lTextEncoder('utf-8');

const encodeString = (typeof cachedTextEncoder.encodeInto === 'function'
    ? function (arg, view) {
    return cachedTextEncoder.encodeInto(arg, view);
}
    : function (arg, view) {
    const buf = cachedTextEncoder.encode(arg);
    view.set(buf);
    return {
        read: arg.length,
        written: buf.length
    };
});

function passStringToWasm0(arg, malloc, realloc) {

    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length);
        getUint8Memory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len);

    const mem = getUint8Memory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }

    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3);
        const view = getUint8Memory0().subarray(ptr + offset, ptr + len);
        const ret = encodeString(arg, view);

        offset += ret.written;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}
/**
* @param {string} username
* @param {string} password
* @param {string} address
*/
export function connect(username, password, address) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(username, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(password, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passStringToWasm0(address, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len2 = WASM_VECTOR_LEN;
        wasm.connect(retptr, ptr0, len0, ptr1, len1, ptr2, len2);
        var r0 = getInt32Memory0()[retptr / 4 + 0];
        var r1 = getInt32Memory0()[retptr / 4 + 1];
        if (r1) {
            throw takeObject(r0);
        }
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
*/
export function greet() {
    wasm.greet();
}

/**
*/
export function init() {
    wasm.init();
}

/**
*/
export class DesktopSize {

    static __wrap(ptr) {
        const obj = Object.create(DesktopSize.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_desktopsize_free(ptr);
    }
    /**
    * @returns {number}
    */
    get width() {
        const ret = wasm.__wbg_get_desktopsize_width(this.ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set width(arg0) {
        wasm.__wbg_set_desktopsize_width(this.ptr, arg0);
    }
    /**
    * @returns {number}
    */
    get height() {
        const ret = wasm.__wbg_get_desktopsize_height(this.ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set height(arg0) {
        wasm.__wbg_set_desktopsize_height(this.ptr, arg0);
    }
}
/**
*/
export class Rect {

    static __wrap(ptr) {
        const obj = Object.create(Rect.prototype);
        obj.ptr = ptr;

        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.ptr;
        this.ptr = 0;

        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_rect_free(ptr);
    }
    /**
    * @returns {number}
    */
    get top() {
        const ret = wasm.__wbg_get_rect_top(this.ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set top(arg0) {
        wasm.__wbg_set_rect_top(this.ptr, arg0);
    }
    /**
    * @returns {number}
    */
    get left() {
        const ret = wasm.__wbg_get_rect_left(this.ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set left(arg0) {
        wasm.__wbg_set_rect_left(this.ptr, arg0);
    }
    /**
    * @returns {number}
    */
    get right() {
        const ret = wasm.__wbg_get_rect_right(this.ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set right(arg0) {
        wasm.__wbg_set_rect_right(this.ptr, arg0);
    }
    /**
    * @returns {number}
    */
    get bottom() {
        const ret = wasm.__wbg_get_rect_bottom(this.ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set bottom(arg0) {
        wasm.__wbg_set_rect_bottom(this.ptr, arg0);
    }
    /**
    * @returns {Uint8Array}
    */
    clone_buffer() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.rect_clone_buffer(retptr, this.ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            var v0 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_free(r0, r1 * 1);
            return v0;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
}

export function __wbg_alert_ff5897297c909650(arg0, arg1) {
    alert(getStringFromWasm0(arg0, arg1));
};

export function __wbindgen_string_new(arg0, arg1) {
    const ret = getStringFromWasm0(arg0, arg1);
    return addHeapObject(ret);
};

export function __wbg_new_abda76e883ba8a5f() {
    const ret = new Error();
    return addHeapObject(ret);
};

export function __wbg_stack_658279fe44541cf6(arg0, arg1) {
    const ret = getObject(arg1).stack;
    const ptr0 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    getInt32Memory0()[arg0 / 4 + 1] = len0;
    getInt32Memory0()[arg0 / 4 + 0] = ptr0;
};

export function __wbg_error_f851667af71bcfc6(arg0, arg1) {
    try {
        console.error(getStringFromWasm0(arg0, arg1));
    } finally {
        wasm.__wbindgen_free(arg0, arg1);
    }
};

export function __wbindgen_object_drop_ref(arg0) {
    takeObject(arg0);
};

export function __wbindgen_throw(arg0, arg1) {
    throw new Error(getStringFromWasm0(arg0, arg1));
};

