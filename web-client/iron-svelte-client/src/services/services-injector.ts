import type { ServerBridgeService } from './server-bridge.service';

export let serverBridge: ServerBridgeService;

if (import.meta.env.MODE === 'tauri') {
    import("./tauri-bridge.service").then(module => serverBridge = new module.TauriBridgeService());
} else {
    import("./wasm-bridge.service").then(module => {
        serverBridge = new module.WasmBridgeService();
        serverBridge.init();
    });
}