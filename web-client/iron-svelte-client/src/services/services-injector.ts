import type { ServerBridgeService } from './server-bridge.service';

export let serverBridge: ServerBridgeService;

import("./wasm-bridge.service").then(module => {
    serverBridge = new module.WasmBridgeService();
    serverBridge.init();
});
