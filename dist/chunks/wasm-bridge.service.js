import { Subject, of } from "rxjs";
let WasmBridgeService;
let __tla = (async () => {
  const __vite__wasmUrl = "/_app/immutable/assets/ironrdp_bg-a36fbe54.wasm";
  const __vite__initWasm = async (opts = {}, url) => {
    let result;
    if (url.startsWith("data:")) {
      const urlContent = url.replace(/^data:.*?base64,/, "");
      let bytes;
      if (typeof Buffer === "function" && typeof Buffer.from === "function") {
        bytes = Buffer.from(urlContent, "base64");
      } else if (typeof atob === "function") {
        const binaryString = atob(urlContent);
        bytes = new Uint8Array(binaryString.length);
        for (let i = 0; i < binaryString.length; i++) {
          bytes[i] = binaryString.charCodeAt(i);
        }
      } else {
        throw new Error("Cannot decode base64-encoded data URL");
      }
      result = await WebAssembly.instantiate(bytes, opts);
    } else {
      const response = await fetch(url);
      const contentType = response.headers.get("Content-Type") || "";
      if ("instantiateStreaming" in WebAssembly && contentType.startsWith("application/wasm")) {
        result = await WebAssembly.instantiateStreaming(response, opts);
      } else {
        const buffer = await response.arrayBuffer();
        result = await WebAssembly.instantiate(buffer, opts);
      }
    }
    return result.instance.exports;
  };
  const heap = new Array(32).fill(void 0);
  heap.push(void 0, null, true, false);
  function getObject(idx) {
    return heap[idx];
  }
  let heap_next = heap.length;
  function dropObject(idx) {
    if (idx < 36)
      return;
    heap[idx] = heap_next;
    heap_next = idx;
  }
  function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
  }
  const lTextDecoder = typeof TextDecoder === "undefined" ? (0, module.require)("util").TextDecoder : TextDecoder;
  let cachedTextDecoder = new lTextDecoder("utf-8", {
    ignoreBOM: true,
    fatal: true
  });
  cachedTextDecoder.decode();
  let cachedUint8Memory0 = new Uint8Array();
  function getUint8Memory0() {
    if (cachedUint8Memory0.byteLength === 0) {
      cachedUint8Memory0 = new Uint8Array(memory.buffer);
    }
    return cachedUint8Memory0;
  }
  function getStringFromWasm0(ptr, len) {
    return cachedTextDecoder.decode(getUint8Memory0().subarray(ptr, ptr + len));
  }
  function addHeapObject(obj) {
    if (heap_next === heap.length)
      heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];
    heap[idx] = obj;
    return idx;
  }
  function makeMutClosure(arg0, arg1, dtor, f) {
    const state = {
      a: arg0,
      b: arg1,
      cnt: 1,
      dtor
    };
    const real = (...args) => {
      state.cnt++;
      const a = state.a;
      state.a = 0;
      try {
        return f(a, state.b, ...args);
      } finally {
        if (--state.cnt === 0) {
          __wbindgen_export_0.get(state.dtor)(a, state.b);
        } else {
          state.a = a;
        }
      }
    };
    real.original = state;
    return real;
  }
  function __wbg_adapter_10(arg0, arg1, arg2) {
    _dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h805bf563cd253bec(arg0, arg1, addHeapObject(arg2));
  }
  function update_mouse$1(session_id, mouse_x, mouse_y, left_click) {
    const ret = update_mouse(session_id, mouse_x, mouse_y, left_click);
    return takeObject(ret);
  }
  let WASM_VECTOR_LEN = 0;
  const lTextEncoder = typeof TextEncoder === "undefined" ? (0, module.require)("util").TextEncoder : TextEncoder;
  let cachedTextEncoder = new lTextEncoder("utf-8");
  const encodeString = typeof cachedTextEncoder.encodeInto === "function" ? function(arg, view) {
    return cachedTextEncoder.encodeInto(arg, view);
  } : function(arg, view) {
    const buf = cachedTextEncoder.encode(arg);
    view.set(buf);
    return {
      read: arg.length,
      written: buf.length
    };
  };
  function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === void 0) {
      const buf = cachedTextEncoder.encode(arg);
      const ptr2 = malloc(buf.length);
      getUint8Memory0().subarray(ptr2, ptr2 + buf.length).set(buf);
      WASM_VECTOR_LEN = buf.length;
      return ptr2;
    }
    let len = arg.length;
    let ptr = malloc(len);
    const mem = getUint8Memory0();
    let offset = 0;
    for (; offset < len; offset++) {
      const code = arg.charCodeAt(offset);
      if (code > 127)
        break;
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
  let cachedInt32Memory0 = new Int32Array();
  function getInt32Memory0() {
    if (cachedInt32Memory0.byteLength === 0) {
      cachedInt32Memory0 = new Int32Array(memory.buffer);
    }
    return cachedInt32Memory0;
  }
  function connect$1(username, password, address) {
    try {
      const retptr = __wbindgen_add_to_stack_pointer(-16);
      const ptr0 = passStringToWasm0(username, __wbindgen_malloc, __wbindgen_realloc);
      const len0 = WASM_VECTOR_LEN;
      const ptr1 = passStringToWasm0(password, __wbindgen_malloc, __wbindgen_realloc);
      const len1 = WASM_VECTOR_LEN;
      const ptr2 = passStringToWasm0(address, __wbindgen_malloc, __wbindgen_realloc);
      const len2 = WASM_VECTOR_LEN;
      connect(retptr, ptr0, len0, ptr1, len1, ptr2, len2);
      var r0 = getInt32Memory0()[retptr / 4 + 0];
      var r1 = getInt32Memory0()[retptr / 4 + 1];
      if (r1) {
        throw takeObject(r0);
      }
    } finally {
      __wbindgen_add_to_stack_pointer(16);
    }
  }
  function init$1() {
    init();
  }
  function handleError(f, args) {
    try {
      return f.apply(this, args);
    } catch (e) {
      __wbindgen_exn_store(addHeapObject(e));
    }
  }
  function __wbg_adapter_16(arg0, arg1, arg2, arg3) {
    wasm_bindgen__convert__closures__invoke2_mut__h1e4db00dee5344e0(arg0, arg1, addHeapObject(arg2), addHeapObject(arg3));
  }
  function __wbindgen_object_drop_ref(arg0) {
    takeObject(arg0);
  }
  function __wbindgen_string_new(arg0, arg1) {
    const ret = getStringFromWasm0(arg0, arg1);
    return addHeapObject(ret);
  }
  function __wbindgen_cb_drop(arg0) {
    const obj = takeObject(arg0).original;
    if (obj.cnt-- == 1) {
      obj.a = 0;
      return true;
    }
    const ret = false;
    return ret;
  }
  function __wbg_call_168da88779e35f61() {
    return handleError(function(arg0, arg1, arg2) {
      const ret = getObject(arg0).call(getObject(arg1), getObject(arg2));
      return addHeapObject(ret);
    }, arguments);
  }
  function __wbg_new_9962f939219f1820(arg0, arg1) {
    try {
      var state0 = {
        a: arg0,
        b: arg1
      };
      var cb0 = (arg02, arg12) => {
        const a = state0.a;
        state0.a = 0;
        try {
          return __wbg_adapter_16(a, state0.b, arg02, arg12);
        } finally {
          state0.a = a;
        }
      };
      const ret = new Promise(cb0);
      return addHeapObject(ret);
    } finally {
      state0.a = state0.b = 0;
    }
  }
  function __wbg_resolve_99fe17964f31ffc0(arg0) {
    const ret = Promise.resolve(getObject(arg0));
    return addHeapObject(ret);
  }
  function __wbg_then_11f7a54d67b4bfad(arg0, arg1) {
    const ret = getObject(arg0).then(getObject(arg1));
    return addHeapObject(ret);
  }
  function __wbg_new_abda76e883ba8a5f() {
    const ret = new Error();
    return addHeapObject(ret);
  }
  function __wbg_stack_658279fe44541cf6(arg0, arg1) {
    const ret = getObject(arg1).stack;
    const ptr0 = passStringToWasm0(ret, __wbindgen_malloc, __wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    getInt32Memory0()[arg0 / 4 + 1] = len0;
    getInt32Memory0()[arg0 / 4 + 0] = ptr0;
  }
  function __wbg_error_f851667af71bcfc6(arg0, arg1) {
    try {
      console.error(getStringFromWasm0(arg0, arg1));
    } finally {
      __wbindgen_free(arg0, arg1);
    }
  }
  function __wbindgen_throw(arg0, arg1) {
    throw new Error(getStringFromWasm0(arg0, arg1));
  }
  function __wbindgen_closure_wrapper67(arg0, arg1, arg2) {
    const ret = makeMutClosure(arg0, arg1, 23, __wbg_adapter_10);
    return addHeapObject(ret);
  }
  const __vite__wasmModule = await __vite__initWasm({
    "./ironrdp_bg.js": {
      __wbindgen_object_drop_ref,
      __wbindgen_string_new,
      __wbindgen_cb_drop,
      __wbg_call_168da88779e35f61,
      __wbg_new_9962f939219f1820,
      __wbg_resolve_99fe17964f31ffc0,
      __wbg_then_11f7a54d67b4bfad,
      __wbg_new_abda76e883ba8a5f,
      __wbg_stack_658279fe44541cf6,
      __wbg_error_f851667af71bcfc6,
      __wbindgen_throw,
      __wbindgen_closure_wrapper67
    }
  }, __vite__wasmUrl);
  const memory = __vite__wasmModule.memory;
  const update_mouse = __vite__wasmModule.update_mouse;
  const connect = __vite__wasmModule.connect;
  const init = __vite__wasmModule.init;
  const __wbindgen_export_0 = __vite__wasmModule.__wbindgen_export_0;
  const _dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h805bf563cd253bec = __vite__wasmModule._dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h805bf563cd253bec;
  const __wbindgen_add_to_stack_pointer = __vite__wasmModule.__wbindgen_add_to_stack_pointer;
  const __wbindgen_malloc = __vite__wasmModule.__wbindgen_malloc;
  const __wbindgen_realloc = __vite__wasmModule.__wbindgen_realloc;
  const __wbindgen_exn_store = __vite__wasmModule.__wbindgen_exn_store;
  const wasm_bindgen__convert__closures__invoke2_mut__h1e4db00dee5344e0 = __vite__wasmModule.wasm_bindgen__convert__closures__invoke2_mut__h1e4db00dee5344e0;
  const __wbindgen_free = __vite__wasmModule.__wbindgen_free;
  const IronWasm = Object.freeze(Object.defineProperty({
    __proto__: null,
    update_mouse: update_mouse$1,
    connect: connect$1,
    init: init$1,
    __wbindgen_object_drop_ref,
    __wbindgen_string_new,
    __wbindgen_cb_drop,
    __wbg_call_168da88779e35f61,
    __wbg_new_9962f939219f1820,
    __wbg_resolve_99fe17964f31ffc0,
    __wbg_then_11f7a54d67b4bfad,
    __wbg_new_abda76e883ba8a5f,
    __wbg_stack_658279fe44541cf6,
    __wbg_error_f851667af71bcfc6,
    __wbindgen_throw,
    __wbindgen_closure_wrapper67
  }, Symbol.toStringTag, {
    value: "Module"
  }));
  WasmBridgeService = class WasmBridgeService {
    wasmBridge = IronWasm;
    _resize = new Subject();
    _updateImage = new Subject();
    resize;
    updateImage;
    constructor() {
      this.resize = this._resize.asObservable();
      this.updateImage = this._updateImage.asObservable();
    }
    init() {
      this.wasmBridge.init();
    }
    updateMouse(mouse_x, mouse_y, click_state) {
    }
    connect(username, password, address) {
      this.wasmBridge.connect(username, password, address);
      return of({
        session_id: 0,
        initial_desktop_size: {
          height: 0,
          width: 0
        },
        websocket_port: 0
      });
    }
  };
})();
export {
  WasmBridgeService,
  __tla
};
