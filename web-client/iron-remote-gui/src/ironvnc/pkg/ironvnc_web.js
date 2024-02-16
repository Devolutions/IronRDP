let wasm;

const heap = new Array(128).fill(undefined);

heap.push(undefined, null, true, false);

function getObject(idx) { return heap[idx]; }

let heap_next = heap.length;

function addHeapObject(obj) {
    if (heap_next === heap.length) heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];

    heap[idx] = obj;
    return idx;
}

function dropObject(idx) {
    if (idx < 132) return;
    heap[idx] = heap_next;
    heap_next = idx;
}

function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
}

const cachedTextDecoder = (typeof TextDecoder !== 'undefined' ? new TextDecoder('utf-8', { ignoreBOM: true, fatal: true }) : { decode: () => { throw Error('TextDecoder not available') } } );

if (typeof TextDecoder !== 'undefined') { cachedTextDecoder.decode(); };

let cachedUint8Memory0 = null;

function getUint8Memory0() {
    if (cachedUint8Memory0 === null || cachedUint8Memory0.byteLength === 0) {
        cachedUint8Memory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8Memory0;
}

function getStringFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return cachedTextDecoder.decode(getUint8Memory0().subarray(ptr, ptr + len));
}

let WASM_VECTOR_LEN = 0;

const cachedTextEncoder = (typeof TextEncoder !== 'undefined' ? new TextEncoder('utf-8') : { encode: () => { throw Error('TextEncoder not available') } } );

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
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8Memory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

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
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8Memory0().subarray(ptr + offset, ptr + len);
        const ret = encodeString(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

let cachedInt32Memory0 = null;

function getInt32Memory0() {
    if (cachedInt32Memory0 === null || cachedInt32Memory0.byteLength === 0) {
        cachedInt32Memory0 = new Int32Array(wasm.memory.buffer);
    }
    return cachedInt32Memory0;
}

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

const CLOSURE_DTORS = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(state => {
    wasm.__wbindgen_export_2.get(state.dtor)(state.a, state.b)
});

function makeMutClosure(arg0, arg1, dtor, f) {
    const state = { a: arg0, b: arg1, cnt: 1, dtor };
    const real = (...args) => {
        // First up with a closure we increment the internal reference
        // count. This ensures that the Rust closure environment won't
        // be deallocated while we're invoking it.
        state.cnt++;
        const a = state.a;
        state.a = 0;
        try {
            return f(a, state.b, ...args);
        } finally {
            if (--state.cnt === 0) {
                wasm.__wbindgen_export_2.get(state.dtor)(a, state.b);
                CLOSURE_DTORS.unregister(state);
            } else {
                state.a = a;
            }
        }
    };
    real.original = state;
    CLOSURE_DTORS.register(real, state, state);
    return real;
}
function __wbg_adapter_28(arg0, arg1) {
    wasm.wasm_bindgen__convert__closures__invoke0_mut__h472a2eec1eb63f16(arg0, arg1);
}

function __wbg_adapter_31(arg0, arg1, arg2) {
    wasm._dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h52ac3e07abf19181(arg0, arg1, addHeapObject(arg2));
}

function __wbg_adapter_34(arg0, arg1) {
    wasm._dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h567eb7933fc125a1(arg0, arg1);
}

function __wbg_adapter_41(arg0, arg1, arg2) {
    wasm._dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h8cf2451f34e79e5d(arg0, arg1, addHeapObject(arg2));
}

function _assertClass(instance, klass) {
    if (!(instance instanceof klass)) {
        throw new Error(`expected instance of ${klass.name}`);
    }
    return instance.ptr;
}
/**
* @param {string} log_level
*/
export function ironrdp_init(log_level) {
    const ptr0 = passStringToWasm0(log_level, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.ironrdp_init(ptr0, len0);
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8Memory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        wasm.__wbindgen_exn_store(addHeapObject(e));
    }
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8Memory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedUint8ClampedMemory0 = null;

function getUint8ClampedMemory0() {
    if (cachedUint8ClampedMemory0 === null || cachedUint8ClampedMemory0.byteLength === 0) {
        cachedUint8ClampedMemory0 = new Uint8ClampedArray(wasm.memory.buffer);
    }
    return cachedUint8ClampedMemory0;
}

function getClampedArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ClampedMemory0().subarray(ptr / 1, ptr / 1 + len);
}
function __wbg_adapter_204(arg0, arg1, arg2, arg3) {
    wasm.wasm_bindgen__convert__closures__invoke2_mut__h794127c7cda2640f(arg0, arg1, addHeapObject(arg2), addHeapObject(arg3));
}

/**
*/
export const IronRdpErrorKind = Object.freeze({
/**
* Catch-all error kind
*/
General:0,"0":"General",
/**
* Incorrect password used
*/
WrongPassword:1,"1":"WrongPassword",
/**
* Unable to login to machine
*/
LogonFailure:2,"2":"LogonFailure",
/**
* Insufficient permission, server denied access
*/
AccessDenied:3,"3":"AccessDenied",
/**
* Something wrong happened when sending or receiving the RDCleanPath message
*/
RDCleanPath:4,"4":"RDCleanPath",
/**
* Couldn’t connect to proxy
*/
ProxyConnect:5,"5":"ProxyConnect", });

const ClipboardContentFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_clipboardcontent_free(ptr >>> 0));
/**
* Object which represents single clipboard format represented standard MIME type.
*/
export class ClipboardContent {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(ClipboardContent.prototype);
        obj.__wbg_ptr = ptr;
        ClipboardContentFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ClipboardContentFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_clipboardcontent_free(ptr);
    }
    /**
    * @param {string} mime_type
    * @param {string} text
    * @returns {ClipboardContent}
    */
    static new_text(mime_type, text) {
        const ptr0 = passStringToWasm0(mime_type, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.clipboardcontent_new_binary(ptr0, len0, ptr1, len1);
        return ClipboardContent.__wrap(ret);
    }
    /**
    * @param {string} mime_type
    * @param {Uint8Array} binary
    * @returns {ClipboardContent}
    */
    static new_binary(mime_type, binary) {
        const ptr0 = passStringToWasm0(mime_type, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArray8ToWasm0(binary, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.clipboardcontent_new_binary(ptr0, len0, ptr1, len1);
        return ClipboardContent.__wrap(ret);
    }
    /**
    * @returns {string}
    */
    mime_type() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.clipboardcontent_mime_type(retptr, this.__wbg_ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
    * @returns {any}
    */
    value() {
        const ret = wasm.clipboardcontent_value(this.__wbg_ptr);
        return takeObject(ret);
    }
}

const ClipboardTransactionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_clipboardtransaction_free(ptr >>> 0));
/**
* Object which represents complete clipboard transaction with multiple MIME types.
*/
export class ClipboardTransaction {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(ClipboardTransaction.prototype);
        obj.__wbg_ptr = ptr;
        ClipboardTransactionFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ClipboardTransactionFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_clipboardtransaction_free(ptr);
    }
    /**
    * @returns {ClipboardTransaction}
    */
    static new() {
        const ret = wasm.clipboardtransaction_new();
        return ClipboardTransaction.__wrap(ret);
    }
    /**
    * @param {ClipboardContent} content
    */
    add_content(content) {
        _assertClass(content, ClipboardContent);
        var ptr0 = content.__destroy_into_raw();
        wasm.clipboardtransaction_add_content(this.__wbg_ptr, ptr0);
    }
    /**
    * @returns {boolean}
    */
    is_empty() {
        const ret = wasm.clipboardtransaction_is_empty(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
    * @returns {Array<any>}
    */
    content() {
        const ret = wasm.clipboardtransaction_content(this.__wbg_ptr);
        return takeObject(ret);
    }
}

const DesktopSizeFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_desktopsize_free(ptr >>> 0));
/**
*/
export class DesktopSize {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(DesktopSize.prototype);
        obj.__wbg_ptr = ptr;
        DesktopSizeFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        DesktopSizeFinalization.unregister(this);
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
        const ret = wasm.__wbg_get_desktopsize_width(this.__wbg_ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set width(arg0) {
        wasm.__wbg_set_desktopsize_width(this.__wbg_ptr, arg0);
    }
    /**
    * @returns {number}
    */
    get height() {
        const ret = wasm.__wbg_get_desktopsize_height(this.__wbg_ptr);
        return ret;
    }
    /**
    * @param {number} arg0
    */
    set height(arg0) {
        wasm.__wbg_set_desktopsize_height(this.__wbg_ptr, arg0);
    }
    /**
    * @param {number} width
    * @param {number} height
    * @returns {DesktopSize}
    */
    static new(width, height) {
        const ret = wasm.desktopsize_new(width, height);
        return DesktopSize.__wrap(ret);
    }
}

const DeviceEventFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_deviceevent_free(ptr >>> 0));
/**
*/
export class DeviceEvent {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(DeviceEvent.prototype);
        obj.__wbg_ptr = ptr;
        DeviceEventFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        DeviceEventFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_deviceevent_free(ptr);
    }
    /**
    * @param {number} button
    * @returns {DeviceEvent}
    */
    static new_mouse_button_pressed(button) {
        const ret = wasm.deviceevent_new_mouse_button_pressed(button);
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {number} button
    * @returns {DeviceEvent}
    */
    static new_mouse_button_released(button) {
        const ret = wasm.deviceevent_new_mouse_button_released(button);
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {number} x
    * @param {number} y
    * @returns {DeviceEvent}
    */
    static new_mouse_move(x, y) {
        const ret = wasm.deviceevent_new_mouse_move(x, y);
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {boolean} vertical
    * @param {number} rotation_units
    * @returns {DeviceEvent}
    */
    static new_wheel_rotations(vertical, rotation_units) {
        const ret = wasm.deviceevent_new_wheel_rotations(vertical, rotation_units);
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {number} scancode
    * @returns {DeviceEvent}
    */
    static new_key_pressed(scancode) {
        const ret = wasm.deviceevent_new_key_pressed(scancode);
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {number} scancode
    * @returns {DeviceEvent}
    */
    static new_key_released(scancode) {
        const ret = wasm.deviceevent_new_key_released(scancode);
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {string} character
    * @returns {DeviceEvent}
    */
    static new_unicode_pressed(character) {
        const ret = wasm.deviceevent_new_unicode_pressed(character.codePointAt(0));
        return DeviceEvent.__wrap(ret);
    }
    /**
    * @param {string} character
    * @returns {DeviceEvent}
    */
    static new_unicode_released(character) {
        const ret = wasm.deviceevent_new_unicode_released(character.codePointAt(0));
        return DeviceEvent.__wrap(ret);
    }
}

const InputTransactionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_inputtransaction_free(ptr >>> 0));
/**
*/
export class InputTransaction {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(InputTransaction.prototype);
        obj.__wbg_ptr = ptr;
        InputTransactionFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        InputTransactionFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_inputtransaction_free(ptr);
    }
    /**
    * @returns {InputTransaction}
    */
    static new() {
        const ret = wasm.inputtransaction_new();
        return InputTransaction.__wrap(ret);
    }
    /**
    * @param {DeviceEvent} event
    */
    add_event(event) {
        _assertClass(event, DeviceEvent);
        var ptr0 = event.__destroy_into_raw();
        wasm.inputtransaction_add_event(this.__wbg_ptr, ptr0);
    }
}

const IronRdpErrorFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_ironrdperror_free(ptr >>> 0));
/**
*/
export class IronRdpError {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(IronRdpError.prototype);
        obj.__wbg_ptr = ptr;
        IronRdpErrorFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        IronRdpErrorFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_ironrdperror_free(ptr);
    }
    /**
    * @returns {string}
    */
    backtrace() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.ironrdperror_backtrace(retptr, this.__wbg_ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
    * @returns {IronRdpErrorKind}
    */
    kind() {
        const ret = wasm.ironrdperror_kind(this.__wbg_ptr);
        return ret;
    }
}

const SessionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_session_free(ptr >>> 0));
/**
*/
export class Session {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(Session.prototype);
        obj.__wbg_ptr = ptr;
        SessionFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        SessionFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_session_free(ptr);
    }
    /**
    * @returns {Promise<SessionTerminationInfo>}
    */
    run() {
        const ret = wasm.session_run(this.__wbg_ptr);
        return takeObject(ret);
    }
    /**
    * @returns {DesktopSize}
    */
    desktop_size() {
        const ret = wasm.session_desktop_size(this.__wbg_ptr);
        return DesktopSize.__wrap(ret);
    }
    /**
    * @param {InputTransaction} transaction
    */
    apply_inputs(transaction) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            _assertClass(transaction, InputTransaction);
            var ptr0 = transaction.__destroy_into_raw();
            wasm.session_apply_inputs(retptr, this.__wbg_ptr, ptr0);
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
    release_all_inputs() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.session_release_all_inputs(retptr, this.__wbg_ptr);
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
    * @param {boolean} _scroll_lock
    * @param {boolean} _num_lock
    * @param {boolean} _caps_lock
    * @param {boolean} _kana_lock
    */
    synchronize_lock_keys(_scroll_lock, _num_lock, _caps_lock, _kana_lock) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.session_synchronize_lock_keys(retptr, this.__wbg_ptr, _scroll_lock, _num_lock, _caps_lock, _kana_lock);
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
    shutdown() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.session_shutdown(retptr, this.__wbg_ptr);
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
    * @param {ClipboardTransaction} _content
    * @returns {Promise<void>}
    */
    on_clipboard_paste(_content) {
        _assertClass(_content, ClipboardTransaction);
        var ptr0 = _content.__destroy_into_raw();
        const ret = wasm.session_on_clipboard_paste(this.__wbg_ptr, ptr0);
        return takeObject(ret);
    }
    /**
    * @returns {boolean}
    */
    supports_unicode_keyboard_shortcuts() {
        const ret = wasm.session_supports_unicode_keyboard_shortcuts(this.__wbg_ptr);
        return ret !== 0;
    }
}

const SessionBuilderFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_sessionbuilder_free(ptr >>> 0));
/**
*/
export class SessionBuilder {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(SessionBuilder.prototype);
        obj.__wbg_ptr = ptr;
        SessionBuilderFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        SessionBuilderFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_sessionbuilder_free(ptr);
    }
    /**
    * @returns {SessionBuilder}
    */
    static new() {
        const ret = wasm.sessionbuilder_new();
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Required
    * @param {string} username
    * @returns {SessionBuilder}
    */
    username(username) {
        const ptr0 = passStringToWasm0(username, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_username(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Required
    * @param {string} destination
    * @returns {SessionBuilder}
    */
    destination(destination) {
        const ptr0 = passStringToWasm0(destination, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_destination(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {string} server_domain
    * @returns {SessionBuilder}
    */
    server_domain(server_domain) {
        const ptr0 = passStringToWasm0(server_domain, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_server_domain(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Required
    * @param {string} password
    * @returns {SessionBuilder}
    */
    password(password) {
        const ptr0 = passStringToWasm0(password, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_password(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Required
    * @param {string} address
    * @returns {SessionBuilder}
    */
    proxy_address(address) {
        const ptr0 = passStringToWasm0(address, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_proxy_address(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Required
    * @param {string} token
    * @returns {SessionBuilder}
    */
    auth_token(token) {
        const ptr0 = passStringToWasm0(token, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_auth_token(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {string} arg0
    * @returns {SessionBuilder}
    */
    pcb(arg0) {
        const ptr0 = passStringToWasm0(arg0, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_pcb(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {string | undefined} [kdc_proxy_url]
    * @returns {SessionBuilder}
    */
    kdc_proxy_url(kdc_proxy_url) {
        var ptr0 = isLikeNone(kdc_proxy_url) ? 0 : passStringToWasm0(kdc_proxy_url, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len0 = WASM_VECTOR_LEN;
        const ret = wasm.sessionbuilder_kdc_proxy_url(this.__wbg_ptr, ptr0, len0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {DesktopSize} desktop_size
    * @returns {SessionBuilder}
    */
    desktop_size(desktop_size) {
        _assertClass(desktop_size, DesktopSize);
        var ptr0 = desktop_size.__destroy_into_raw();
        const ret = wasm.sessionbuilder_desktop_size(this.__wbg_ptr, ptr0);
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Required
    * @param {HTMLCanvasElement} canvas
    * @returns {SessionBuilder}
    */
    render_canvas(canvas) {
        const ret = wasm.sessionbuilder_render_canvas(this.__wbg_ptr, addHeapObject(canvas));
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    *
    * # Callback signature:
    * ```typescript
    * function callback(
    *     cursor_kind: string,
    *     cursor_data: string | undefined,
    *     hotspot_x: number | undefined,
    *     hotspot_y: number | undefined
    * ): void
    * ```
    *
    * # Cursor kinds:
    * - `default` (default system cursor); other arguments are `UNDEFINED`
    * - `none` (hide cursor); other arguments are `UNDEFINED`
    * - `url` (custom cursor data URL); `cursor_data` contains the data URL with Base64-encoded
    *   cursor bitmap; `hotspot_x` and `hotspot_y` are set to the cursor hotspot coordinates.
    * @param {Function} callback
    * @returns {SessionBuilder}
    */
    set_cursor_style_callback(callback) {
        const ret = wasm.sessionbuilder_set_cursor_style_callback(this.__wbg_ptr, addHeapObject(callback));
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {any} context
    * @returns {SessionBuilder}
    */
    set_cursor_style_callback_context(context) {
        const ret = wasm.sessionbuilder_set_cursor_style_callback_context(this.__wbg_ptr, addHeapObject(context));
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {Function} callback
    * @returns {SessionBuilder}
    */
    remote_clipboard_changed_callback(callback) {
        const ret = wasm.sessionbuilder_remote_clipboard_changed_callback(this.__wbg_ptr, addHeapObject(callback));
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {Function} callback
    * @returns {SessionBuilder}
    */
    remote_received_format_list_callback(callback) {
        const ret = wasm.sessionbuilder_remote_received_format_list_callback(this.__wbg_ptr, addHeapObject(callback));
        return SessionBuilder.__wrap(ret);
    }
    /**
    * Optional
    * @param {Function} callback
    * @returns {SessionBuilder}
    */
    force_clipboard_update_callback(callback) {
        const ret = wasm.sessionbuilder_force_clipboard_update_callback(this.__wbg_ptr, addHeapObject(callback));
        return SessionBuilder.__wrap(ret);
    }
    /**
    * @returns {Promise<Session>}
    */
    connect() {
        const ret = wasm.sessionbuilder_connect(this.__wbg_ptr);
        return takeObject(ret);
    }
}

const SessionTerminationInfoFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_sessionterminationinfo_free(ptr >>> 0));
/**
*/
export class SessionTerminationInfo {

    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(SessionTerminationInfo.prototype);
        obj.__wbg_ptr = ptr;
        SessionTerminationInfoFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }

    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        SessionTerminationInfoFinalization.unregister(this);
        return ptr;
    }

    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_sessionterminationinfo_free(ptr);
    }
    /**
    * @returns {string}
    */
    reason() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.sessionterminationinfo_reason(retptr, this.__wbg_ptr);
            var r0 = getInt32Memory0()[retptr / 4 + 0];
            var r1 = getInt32Memory0()[retptr / 4 + 1];
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);

            } catch (e) {
                if (module.headers.get('Content-Type') != 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else {
                    throw e;
                }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);

    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };

        } else {
            return instance;
        }
    }
}

function __wbg_get_imports() {
    const imports = {};
    imports.wbg = {};
    imports.wbg.__wbindgen_object_clone_ref = function(arg0) {
        const ret = getObject(arg0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_object_drop_ref = function(arg0) {
        takeObject(arg0);
    };
    imports.wbg.__wbindgen_cb_drop = function(arg0) {
        const obj = takeObject(arg0).original;
        if (obj.cnt-- == 1) {
            obj.a = 0;
            return true;
        }
        const ret = false;
        return ret;
    };
    imports.wbg.__wbg_sessionterminationinfo_new = function(arg0) {
        const ret = SessionTerminationInfo.__wrap(arg0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_number_new = function(arg0) {
        const ret = arg0;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_ironrdperror_new = function(arg0) {
        const ret = IronRdpError.__wrap(arg0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_session_new = function(arg0) {
        const ret = Session.__wrap(arg0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_string_new = function(arg0, arg1) {
        const ret = getStringFromWasm0(arg0, arg1);
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_clearTimeout_541ac0980ffcef74 = function(arg0) {
        const ret = clearTimeout(takeObject(arg0));
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_setTimeout_7d81d052875b0f4f = function() { return handleError(function (arg0, arg1) {
        const ret = setTimeout(getObject(arg0), arg1);
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbindgen_is_string = function(arg0) {
        const ret = typeof(getObject(arg0)) === 'string';
        return ret;
    };
    imports.wbg.__wbindgen_string_get = function(arg0, arg1) {
        const obj = getObject(arg1);
        const ret = typeof(obj) === 'string' ? obj : undefined;
        var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        var len1 = WASM_VECTOR_LEN;
        getInt32Memory0()[arg0 / 4 + 1] = len1;
        getInt32Memory0()[arg0 / 4 + 0] = ptr1;
    };
    imports.wbg.__wbg_queueMicrotask_f61ee94ee663068b = function(arg0) {
        queueMicrotask(getObject(arg0));
    };
    imports.wbg.__wbg_queueMicrotask_f82fc5d1e8f816ae = function(arg0) {
        const ret = getObject(arg0).queueMicrotask;
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_is_function = function(arg0) {
        const ret = typeof(getObject(arg0)) === 'function';
        return ret;
    };
    imports.wbg.__wbg_instanceof_CanvasRenderingContext2d_b43c8f92c4744b7b = function(arg0) {
        let result;
        try {
            result = getObject(arg0) instanceof CanvasRenderingContext2D;
        } catch (_) {
            result = false;
        }
        const ret = result;
        return ret;
    };
    imports.wbg.__wbg_putImageData_b7bdf7d494cca4db = function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
        getObject(arg0).putImageData(getObject(arg1), arg2, arg3, arg4, arg5, arg6, arg7);
    }, arguments) };
    imports.wbg.__wbg_putImageData_41ff54f30e0b1de7 = function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7) {
        getObject(arg0).putImageData(getObject(arg1), arg2, arg3, arg4, arg5, arg6, arg7);
    }, arguments) };
    imports.wbg.__wbg_wasClean_1efd9561c5671b45 = function(arg0) {
        const ret = getObject(arg0).wasClean;
        return ret;
    };
    imports.wbg.__wbg_code_72a380a2ce61a242 = function(arg0) {
        const ret = getObject(arg0).code;
        return ret;
    };
    imports.wbg.__wbg_reason_ad453a16ee68a1b9 = function(arg0, arg1) {
        const ret = getObject(arg1).reason;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getInt32Memory0()[arg0 / 4 + 1] = len1;
        getInt32Memory0()[arg0 / 4 + 0] = ptr1;
    };
    imports.wbg.__wbg_newwitheventinitdict_744eb6eb61245b7c = function() { return handleError(function (arg0, arg1, arg2) {
        const ret = new CloseEvent(getStringFromWasm0(arg0, arg1), getObject(arg2));
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_addEventListener_9bf60ea8a362e5e4 = function() { return handleError(function (arg0, arg1, arg2, arg3) {
        getObject(arg0).addEventListener(getStringFromWasm0(arg1, arg2), getObject(arg3));
    }, arguments) };
    imports.wbg.__wbg_addEventListener_374cbfd2bbc19ccf = function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
        getObject(arg0).addEventListener(getStringFromWasm0(arg1, arg2), getObject(arg3), getObject(arg4));
    }, arguments) };
    imports.wbg.__wbg_dispatchEvent_40c3472e9e4dcf5e = function() { return handleError(function (arg0, arg1) {
        const ret = getObject(arg0).dispatchEvent(getObject(arg1));
        return ret;
    }, arguments) };
    imports.wbg.__wbg_removeEventListener_66ee1536a0b32c11 = function() { return handleError(function (arg0, arg1, arg2, arg3) {
        getObject(arg0).removeEventListener(getStringFromWasm0(arg1, arg2), getObject(arg3));
    }, arguments) };
    imports.wbg.__wbg_readyState_c8f9a5deaec3bb41 = function(arg0) {
        const ret = getObject(arg0).readyState;
        return ret;
    };
    imports.wbg.__wbg_setbinaryType_68fc3c6feda7310c = function(arg0, arg1) {
        getObject(arg0).binaryType = takeObject(arg1);
    };
    imports.wbg.__wbg_new_2575c598b4006174 = function() { return handleError(function (arg0, arg1) {
        const ret = new WebSocket(getStringFromWasm0(arg0, arg1));
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_close_328b8b803521cbdd = function() { return handleError(function (arg0) {
        getObject(arg0).close();
    }, arguments) };
    imports.wbg.__wbg_send_5bf3f962e9ffe0f6 = function() { return handleError(function (arg0, arg1, arg2) {
        getObject(arg0).send(getStringFromWasm0(arg1, arg2));
    }, arguments) };
    imports.wbg.__wbg_send_2ba7d32fcb03b9a4 = function() { return handleError(function (arg0, arg1, arg2) {
        getObject(arg0).send(getArrayU8FromWasm0(arg1, arg2));
    }, arguments) };
    imports.wbg.__wbg_newwithu8clampedarray_37ad5b92c7f416b0 = function() { return handleError(function (arg0, arg1, arg2) {
        const ret = new ImageData(getClampedArrayU8FromWasm0(arg0, arg1), arg2 >>> 0);
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_debug_34c9290896ec9856 = function(arg0) {
        console.debug(getObject(arg0));
    };
    imports.wbg.__wbg_error_e60eff06f24ab7a4 = function(arg0) {
        console.error(getObject(arg0));
    };
    imports.wbg.__wbg_info_d7d58472d0bab115 = function(arg0) {
        console.info(getObject(arg0));
    };
    imports.wbg.__wbg_warn_f260f49434e45e62 = function(arg0) {
        console.warn(getObject(arg0));
    };
    imports.wbg.__wbg_setwidth_7591ce24118fd14a = function(arg0, arg1) {
        getObject(arg0).width = arg1 >>> 0;
    };
    imports.wbg.__wbg_setheight_f7ae862183d88bd5 = function(arg0, arg1) {
        getObject(arg0).height = arg1 >>> 0;
    };
    imports.wbg.__wbg_getContext_164dc98953ddbc68 = function() { return handleError(function (arg0, arg1, arg2) {
        const ret = getObject(arg0).getContext(getStringFromWasm0(arg1, arg2));
        return isLikeNone(ret) ? 0 : addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_data_ba3ea616b5392abf = function(arg0) {
        const ret = getObject(arg0).data;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_setwidth_a04b04d18cb81715 = function(arg0, arg1) {
        getObject(arg0).width = arg1 >>> 0;
    };
    imports.wbg.__wbg_setheight_ae3c51b7555bd27d = function(arg0, arg1) {
        getObject(arg0).height = arg1 >>> 0;
    };
    imports.wbg.__wbg_crypto_58f13aa23ffcb166 = function(arg0) {
        const ret = getObject(arg0).crypto;
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_is_object = function(arg0) {
        const val = getObject(arg0);
        const ret = typeof(val) === 'object' && val !== null;
        return ret;
    };
    imports.wbg.__wbg_process_5b786e71d465a513 = function(arg0) {
        const ret = getObject(arg0).process;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_versions_c2ab80650590b6a2 = function(arg0) {
        const ret = getObject(arg0).versions;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_node_523d7bd03ef69fba = function(arg0) {
        const ret = getObject(arg0).node;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_msCrypto_abcb1295e768d1f2 = function(arg0) {
        const ret = getObject(arg0).msCrypto;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_require_2784e593a4674877 = function() { return handleError(function () {
        const ret = module.require;
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_randomFillSync_a0d98aa11c81fe89 = function() { return handleError(function (arg0, arg1) {
        getObject(arg0).randomFillSync(takeObject(arg1));
    }, arguments) };
    imports.wbg.__wbg_getRandomValues_504510b5564925af = function() { return handleError(function (arg0, arg1) {
        getObject(arg0).getRandomValues(getObject(arg1));
    }, arguments) };
    imports.wbg.__wbg_new_75208e29bddfd88c = function() {
        const ret = new Array();
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_newnoargs_cfecb3965268594c = function(arg0, arg1) {
        const ret = new Function(getStringFromWasm0(arg0, arg1));
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_call_3f093dd26d5569f8 = function() { return handleError(function (arg0, arg1) {
        const ret = getObject(arg0).call(getObject(arg1));
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_new_632630b5cec17f21 = function() {
        const ret = new Object();
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_self_05040bd9523805b9 = function() { return handleError(function () {
        const ret = self.self;
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_window_adc720039f2cb14f = function() { return handleError(function () {
        const ret = window.window;
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_globalThis_622105db80c1457d = function() { return handleError(function () {
        const ret = globalThis.globalThis;
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_global_f56b013ed9bcf359 = function() { return handleError(function () {
        const ret = global.global;
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbindgen_is_undefined = function(arg0) {
        const ret = getObject(arg0) === undefined;
        return ret;
    };
    imports.wbg.__wbg_instanceof_ArrayBuffer_9221fa854ffb71b5 = function(arg0) {
        let result;
        try {
            result = getObject(arg0) instanceof ArrayBuffer;
        } catch (_) {
            result = false;
        }
        const ret = result;
        return ret;
    };
    imports.wbg.__wbg_instanceof_Error_5869c4f17aac9eb2 = function(arg0) {
        let result;
        try {
            result = getObject(arg0) instanceof Error;
        } catch (_) {
            result = false;
        }
        const ret = result;
        return ret;
    };
    imports.wbg.__wbg_message_2a19bb5b62cf8e22 = function(arg0) {
        const ret = getObject(arg0).message;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_name_405bb0aa047a1bf5 = function(arg0) {
        const ret = getObject(arg0).name;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_toString_07f01913ec9af122 = function(arg0) {
        const ret = getObject(arg0).toString();
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_call_67f2111acd2dfdb6 = function() { return handleError(function (arg0, arg1, arg2) {
        const ret = getObject(arg0).call(getObject(arg1), getObject(arg2));
        return addHeapObject(ret);
    }, arguments) };
    imports.wbg.__wbg_getTime_0e03c3f524be31ef = function(arg0) {
        const ret = getObject(arg0).getTime();
        return ret;
    };
    imports.wbg.__wbg_new0_7a6141101f2206da = function() {
        const ret = new Date();
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_new_70828a4353259d4b = function(arg0, arg1) {
        try {
            var state0 = {a: arg0, b: arg1};
            var cb0 = (arg0, arg1) => {
                const a = state0.a;
                state0.a = 0;
                try {
                    return __wbg_adapter_204(a, state0.b, arg0, arg1);
                } finally {
                    state0.a = a;
                }
            };
            const ret = new Promise(cb0);
            return addHeapObject(ret);
        } finally {
            state0.a = state0.b = 0;
        }
    };
    imports.wbg.__wbg_resolve_5da6faf2c96fd1d5 = function(arg0) {
        const ret = Promise.resolve(getObject(arg0));
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_then_f9e58f5a50f43eae = function(arg0, arg1) {
        const ret = getObject(arg0).then(getObject(arg1));
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_buffer_b914fb8b50ebbc3e = function(arg0) {
        const ret = getObject(arg0).buffer;
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_newwithbyteoffsetandlength_0de9ee56e9f6ee6e = function(arg0, arg1, arg2) {
        const ret = new Uint8Array(getObject(arg0), arg1 >>> 0, arg2 >>> 0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_new_b1f2d6842d615181 = function(arg0) {
        const ret = new Uint8Array(getObject(arg0));
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_set_7d988c98e6ced92d = function(arg0, arg1, arg2) {
        getObject(arg0).set(getObject(arg1), arg2 >>> 0);
    };
    imports.wbg.__wbg_length_21c4b0ae73cba59d = function(arg0) {
        const ret = getObject(arg0).length;
        return ret;
    };
    imports.wbg.__wbg_newwithlength_0d03cef43b68a530 = function(arg0) {
        const ret = new Uint8Array(arg0 >>> 0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_subarray_adc418253d76e2f1 = function(arg0, arg1, arg2) {
        const ret = getObject(arg0).subarray(arg1 >>> 0, arg2 >>> 0);
        return addHeapObject(ret);
    };
    imports.wbg.__wbg_set_961700853a212a39 = function() { return handleError(function (arg0, arg1, arg2) {
        const ret = Reflect.set(getObject(arg0), getObject(arg1), getObject(arg2));
        return ret;
    }, arguments) };
    imports.wbg.__wbindgen_debug_string = function(arg0, arg1) {
        const ret = debugString(getObject(arg1));
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getInt32Memory0()[arg0 / 4 + 1] = len1;
        getInt32Memory0()[arg0 / 4 + 0] = ptr1;
    };
    imports.wbg.__wbindgen_throw = function(arg0, arg1) {
        throw new Error(getStringFromWasm0(arg0, arg1));
    };
    imports.wbg.__wbindgen_memory = function() {
        const ret = wasm.memory;
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_closure_wrapper906 = function(arg0, arg1, arg2) {
        const ret = makeMutClosure(arg0, arg1, 544, __wbg_adapter_28);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_closure_wrapper1186 = function(arg0, arg1, arg2) {
        const ret = makeMutClosure(arg0, arg1, 696, __wbg_adapter_31);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_closure_wrapper1188 = function(arg0, arg1, arg2) {
        const ret = makeMutClosure(arg0, arg1, 696, __wbg_adapter_34);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_closure_wrapper1190 = function(arg0, arg1, arg2) {
        const ret = makeMutClosure(arg0, arg1, 696, __wbg_adapter_31);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_closure_wrapper1192 = function(arg0, arg1, arg2) {
        const ret = makeMutClosure(arg0, arg1, 696, __wbg_adapter_31);
        return addHeapObject(ret);
    };
    imports.wbg.__wbindgen_closure_wrapper1208 = function(arg0, arg1, arg2) {
        const ret = makeMutClosure(arg0, arg1, 708, __wbg_adapter_41);
        return addHeapObject(ret);
    };

    return imports;
}

function __wbg_init_memory(imports, maybe_memory) {

}

function __wbg_finalize_init(instance, module) {
    wasm = instance.exports;
    __wbg_init.__wbindgen_wasm_module = module;
    cachedInt32Memory0 = null;
    cachedUint8Memory0 = null;
    cachedUint8ClampedMemory0 = null;


    return wasm;
}

function initSync(module) {
    if (wasm !== undefined) return wasm;

    const imports = __wbg_get_imports();

    __wbg_init_memory(imports);

    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }

    const instance = new WebAssembly.Instance(module, imports);

    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(input) {
    if (wasm !== undefined) return wasm;

    if (typeof input === 'undefined') {
        input = new URL('ironvnc_web_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof input === 'string' || (typeof Request === 'function' && input instanceof Request) || (typeof URL === 'function' && input instanceof URL)) {
        input = fetch(input);
    }

    __wbg_init_memory(imports);

    const { instance, module } = await __wbg_load(await input, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync }
export default __wbg_init;
