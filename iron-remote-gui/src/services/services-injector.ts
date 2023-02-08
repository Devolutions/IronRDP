import type {ServerBridgeService} from './server-bridge.service';
import {loggingService} from "./logging.service";

export let serverBridge: ServerBridgeService;

export async function initServerBridge(mode: 'native' | 'web' = 'web', debug: "OFF" | "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE") {
    if (serverBridge === undefined || serverBridge === null) {
        if (mode === 'native') {
            loggingService.info('Initialize native bridge...');
            const module = await import("./tauri-bridge.service");
            serverBridge = new module.TauriBridgeService();
        } else {
            loggingService.info('Initialize web bridge');
            const module = await import("./wasm-bridge.service");
            serverBridge = new module.WasmBridgeService();
            await serverBridge.init(debug || 'INFO');
        }
    }
}
