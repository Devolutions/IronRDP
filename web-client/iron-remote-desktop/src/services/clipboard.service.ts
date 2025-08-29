import type { RemoteDesktopService } from './remote-desktop.service';
import { isComponentDestroyed } from '../lib/stores/componentLifecycleStore';
import { get } from 'svelte/store';
import type { ClipboardData } from '../interfaces/ClipboardData';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import { runWhenFocusedQueue } from '../lib/stores/runWhenFocusedStore';
import { SessionEventType } from '../enums/SessionEventType';

const CLIPBOARD_MONITORING_INTERVAL_MS = 100;

export class ClipboardService {
    private remoteDesktopService: RemoteDesktopService;
    private module: RemoteDesktopModule;

    private isClipboardApiSupported: boolean = false;

    private lastClientClipboardItems: Record<string, string | Uint8Array> = {};
    private lastReceivedClipboardData: Record<string, string | Uint8Array> = {};
    private lastSentClipboardData: ClipboardData | null = null;
    private clipboardDataToSave: ClipboardData | null = null;
    private lastClipboardMonitorLoopError: Error | null = null;

    constructor(remoteDesktopService: RemoteDesktopService, module: RemoteDesktopModule) {
        this.remoteDesktopService = remoteDesktopService;
        this.module = module;
    }

    initClipboard() {
        // Detect if browser supports async Clipboard API
        if (navigator.clipboard != undefined) {
            if (navigator.clipboard.read != undefined && navigator.clipboard.write != undefined) {
                this.isClipboardApiSupported = true;
            }
        }

        if (!this.isClipboardApiSupported) return;

        this.remoteDesktopService.setOnForceClipboardUpdate(this.onForceClipboardUpdate.bind(this));

        if (this.remoteDesktopService.autoClipboard) {
            this.remoteDesktopService.setOnRemoteClipboardChanged(this.onRemoteClipboardChangedAutoMode.bind(this));
            // Start the clipboard monitoring loop
            setTimeout(this.onMonitorClipboard.bind(this), CLIPBOARD_MONITORING_INTERVAL_MS);
        } else {
            this.remoteDesktopService.setOnRemoteClipboardChanged(this.onRemoteClipboardChangedManualMode.bind(this));
        }
    }

    // Copies clipboard content received from the server to the local clipboard.
    // Returns the result of the operation. On failure, it additionally raises an error session event.
    async saveRemoteClipboardData(): Promise<boolean> {
        if (this.clipboardDataToSave == null) {
            this.remoteDesktopService.raiseSessionEvent({
                type: SessionEventType.ERROR,
                data: 'The server did not send the clipboard data.',
            });
            return false;
        }

        try {
            const mime_formats = this.clipboardDataToRecord(this.clipboardDataToSave);
            const clipboard_item = new ClipboardItem(mime_formats);
            await navigator.clipboard.write([clipboard_item]);

            this.clipboardDataToSave = null;
            return true;
        } catch (err) {
            this.remoteDesktopService.raiseSessionEvent({
                type: SessionEventType.ERROR,
                data: 'Failed to write to the clipboard: ' + err,
            });
            return false;
        }
    }

    // Sends local clipboard's content to the server.
    // Returns the result of the operation. On failure, it additionally raises an error session event.
    async sendClipboardData(): Promise<boolean> {
        try {
            const value = await navigator.clipboard.read();

            // Clipboard is empty
            if (value.length == 0) {
                this.remoteDesktopService.raiseSessionEvent({
                    type: SessionEventType.ERROR,
                    data: 'The clipboard has no data.',
                });
                return false;
            }

            // We only support one item at a time
            const item = value[0];

            if (!item.types.some((type) => type.startsWith('text/') || type.startsWith('image/png'))) {
                // Unsupported types
                this.remoteDesktopService.raiseSessionEvent({
                    type: SessionEventType.ERROR,
                    data: 'The clipboard has no data of supported type (text or image).',
                });
                return false;
            }

            const clipboardData = new this.module.ClipboardData();

            for (const kind of item.types) {
                // Get blob
                const blobIsString = kind.startsWith('text/');
                const blob = await item.getType(kind);

                if (blobIsString) {
                    clipboardData.addText(kind, await blob.text());
                } else {
                    clipboardData.addBinary(kind, new Uint8Array(await blob.arrayBuffer()));
                }
            }

            if (!clipboardData.isEmpty()) {
                this.lastSentClipboardData = clipboardData;
                // TODO(Fix): onClipboardChanged takes an ownership over clipboardData, so lastSentClipboardData will be nullptr.
                await this.remoteDesktopService.onClipboardChanged(clipboardData);
            }

            return true;
        } catch (err) {
            this.remoteDesktopService.raiseSessionEvent({
                type: SessionEventType.ERROR,
                data: 'Failed to read from the clipboard: ' + err,
            });
            return false;
        }
    }

    private runWhenWindowFocused(fn: () => void) {
        if (document.hasFocus()) {
            fn();
        } else {
            runWhenFocusedQueue.enqueue(fn);
        }
    }

    // This function is required to convert `ClipboardData` to an object that can be used
    // with `ClipboardItem` API.
    private clipboardDataToRecord(data: ClipboardData): Record<string, Blob> {
        const result = {} as Record<string, Blob>;

        for (const item of data.items()) {
            const mime = item.mimeType();
            result[mime] = new Blob([item.value()], { type: mime });
        }

        return result;
    }

    private clipboardDataToClipboardItemsRecord(data: ClipboardData): Record<string, string | Uint8Array> {
        const result = {} as Record<string, string | Uint8Array>;

        for (const item of data.items()) {
            const mime = item.mimeType();
            result[mime] = item.value();
        }

        return result;
    }

    // This callback is required to send initial clipboard state if available.
    private onForceClipboardUpdate() {
        // TODO(Fix): lastSentClipboardData is nullptr.
        try {
            if (this.lastSentClipboardData) {
                this.remoteDesktopService.onClipboardChanged(this.lastSentClipboardData);
            } else {
                this.remoteDesktopService.onClipboardChangedEmpty();
            }
        } catch (err) {
            console.error('Failed to send initial clipboard state: ' + err);
        }
    }

    // This callback is required to update client clipboard state when remote side has changed.
    private onRemoteClipboardChangedManualMode(data: ClipboardData) {
        this.clipboardDataToSave = data;
        this.remoteDesktopService.raiseSessionEvent({
            type: SessionEventType.CLIPBOARD_REMOTE_UPDATE,
            data: '',
        });
    }

    // This callback is required to update client clipboard state when remote side has changed.
    private onRemoteClipboardChangedAutoMode(data: ClipboardData) {
        try {
            const mime_formats = this.clipboardDataToRecord(data);
            const clipboard_item = new ClipboardItem(mime_formats);
            this.runWhenWindowFocused(() => {
                this.lastReceivedClipboardData = this.clipboardDataToClipboardItemsRecord(data);
                navigator.clipboard.write([clipboard_item]);
            });
        } catch (err) {
            console.error('Failed to set client clipboard: ' + err);
        }
    }

    // Called periodically to monitor clipboard changes
    private async onMonitorClipboard() {
        try {
            if (!document.hasFocus()) {
                return;
            }

            const value = await navigator.clipboard.read();

            // Clipboard is empty
            if (value.length == 0) {
                return;
            }

            // We only support one item at a time
            const item = value[0];

            if (!item.types.some((type) => type.startsWith('text/') || type.startsWith('image/png'))) {
                // Unsupported types
                return;
            }

            const values: Record<string, string | Uint8Array> = {};
            let sameValue = true;

            // Sadly, browsers build new `ClipboardItem` object for each `read` call,
            // so we can't do reference comparison here :(
            //
            // For monitoring loop approach we also can't drop this logic, as it will result in
            // very frequent network activity.
            for (const kind of item.types) {
                // Get blob
                const blobIsString = kind.startsWith('text/');

                const blob = await item.getType(kind);
                const value = blobIsString ? await blob.text() : new Uint8Array(await blob.arrayBuffer());

                const is_equal = blobIsString
                    ? function (a: string | Uint8Array | undefined, b: string | Uint8Array | undefined) {
                          return a === b;
                      }
                    : function (a: string | Uint8Array | undefined, b: string | Uint8Array | undefined) {
                          if (!(a instanceof Uint8Array) || !(b instanceof Uint8Array)) {
                              return false;
                          }

                          return a.length === b.length && a.every((v, i) => v === b[i]);
                      };

                const previousValue = this.lastClientClipboardItems[kind];

                if (!is_equal(previousValue, value)) {
                    // When the local clipboard updates, we need to compare it with the last data received from the server.
                    // If it's identical, the clipboard was updated with the server's data, so we shouldn't send this data
                    // to the server.
                    if (is_equal(this.lastReceivedClipboardData[kind], value)) {
                        this.lastClientClipboardItems[kind] = this.lastReceivedClipboardData[kind];
                    }
                    // One of mime types has changed, we need to update the clipboard cache
                    else {
                        sameValue = false;
                    }
                }

                values[kind] = value;
            }

            // Clipboard has changed, we need to acknowledge remote side about it.
            if (!sameValue) {
                this.lastClientClipboardItems = values;

                const clipboardData = new this.module.ClipboardData();

                // Iterate over `Record` type
                Object.entries(values).forEach(([key, value]: [string, string | Uint8Array]) => {
                    // skip null/undefined values
                    if (value == null) {
                        return;
                    }

                    if (key.startsWith('text/') && typeof value === 'string') {
                        clipboardData.addText(key, value);
                    } else if (key.startsWith('image/') && value instanceof Uint8Array) {
                        clipboardData.addBinary(key, value);
                    }
                });

                if (!clipboardData.isEmpty()) {
                    this.lastSentClipboardData = clipboardData;
                    // TODO(Fix): onClipboardChanged takes an ownership over clipboardData, so lastSentClipboardData will be nullptr.
                    await this.remoteDesktopService.onClipboardChanged(clipboardData);
                }
            }
        } catch (err) {
            if (err instanceof Error) {
                const printError =
                    this.lastClipboardMonitorLoopError === null ||
                    this.lastClipboardMonitorLoopError.toString() !== err.toString();
                // Prevent spamming the console with the same error
                if (printError) {
                    console.error('Clipboard monitoring error: ' + err);
                }
                this.lastClipboardMonitorLoopError = err;
            }
        } finally {
            if (!get(isComponentDestroyed)) {
                setTimeout(this.onMonitorClipboard.bind(this), CLIPBOARD_MONITORING_INTERVAL_MS);
            }
        }
    }
}
