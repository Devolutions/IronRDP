import type { RemoteDesktopService } from './remote-desktop.service';
import { isComponentDestroyed } from '../lib/stores/componentLifecycleStore';
import { get } from 'svelte/store';
import type { ClipboardData } from '../interfaces/ClipboardData';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import { runWhenFocusedQueue } from '../lib/stores/runWhenFocusedStore';
import { ClipboardApiSupported } from '../enums/ClipboardApiSupported';
import { IronErrorKind } from '../interfaces/Error';

const CLIPBOARD_MONITORING_INTERVAL_MS = 100;

// Helper function to conveniently throw an `IronError`.
function throwIronError(message: string): never {
    throw {
        kind: () => IronErrorKind.General,
        backtrace: () => message,
    };
}

export class ClipboardService {
    private remoteDesktopService: RemoteDesktopService;
    private module: RemoteDesktopModule;

    private ClipboardApiSupported: ClipboardApiSupported = ClipboardApiSupported.None;

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
        // Clipboard API is available only in secure contexts (HTTPS).
        if (!window.isSecureContext) {
            this.remoteDesktopService.emitWarningEvent('Clipboard is available only in secure contexts (HTTPS).');
            return;
        }

        // Detect if browser supports async Clipboard API
        if (navigator.clipboard != undefined) {
            if (navigator.clipboard.read != undefined && navigator.clipboard.write != undefined) {
                this.ClipboardApiSupported = ClipboardApiSupported.Full;
            } else if (navigator.clipboard.readText != undefined) {
                this.ClipboardApiSupported = ClipboardApiSupported.TextOnly;
                this.remoteDesktopService.emitWarningEvent(
                    'Clipboard is limited to text-only data types due to an outdated browser version!',
                );
            } else if (navigator.clipboard.writeText != undefined) {
                this.ClipboardApiSupported = ClipboardApiSupported.TextOnlyServerOnly;
                this.remoteDesktopService.emitWarningEvent(
                    'Clipboard reading is not supported and writing is limited to text-only data types due to an outdated browser version!',
                );
            }
        }

        // The basic Clipboard API is widely supported in modern browsers,
        // so this condition should never be true in practice.
        if (this.ClipboardApiSupported === ClipboardApiSupported.None) {
            this.remoteDesktopService.emitWarningEvent(
                'Clipboard is not supported due to an outdated browser version!',
            );
            return;
        }

        this.remoteDesktopService.setOnForceClipboardUpdate(this.onForceClipboardUpdate.bind(this));

        if (this.ClipboardApiSupported === ClipboardApiSupported.Full) {
            if (this.remoteDesktopService.autoClipboard) {
                this.remoteDesktopService.setOnRemoteClipboardChanged(this.onRemoteClipboardChangedAutoMode.bind(this));
                // Start the clipboard monitoring loop
                setTimeout(this.onMonitorClipboard.bind(this), CLIPBOARD_MONITORING_INTERVAL_MS);
            } else {
                this.remoteDesktopService.setOnRemoteClipboardChanged(
                    this.onRemoteClipboardChangedManualMode.bind(this),
                );
            }
        } else {
            this.remoteDesktopService.setOnRemoteClipboardChanged(this.ffOnRemoteClipboardChanged.bind(this));
        }
    }

    // Copies clipboard content received from the server to the local clipboard.
    // Returns the result of the operation. On failure, it additionally raises an error session event.
    async saveRemoteClipboardData(): Promise<void> {
        if (this.ClipboardApiSupported !== ClipboardApiSupported.Full) {
            return await this.ffSaveRemoteClipboardData();
        }

        if (this.clipboardDataToSave == null) {
            throwIronError('The server did not send the clipboard data.');
        }

        try {
            const mime_formats = this.clipboardDataToRecord(this.clipboardDataToSave);
            const clipboard_item = new ClipboardItem(mime_formats);
            await navigator.clipboard.write([clipboard_item]);

            this.clipboardDataToSave = null;
        } catch (err) {
            throwIronError('Failed to write to the clipboard: ' + err);
        }
    }

    // Sends local clipboard's content to the server.
    // Returns the result of the operation. On failure, it additionally raises an error session event.
    async sendClipboardData(): Promise<void> {
        if (this.ClipboardApiSupported !== ClipboardApiSupported.Full) {
            return await this.ffSendClipboardData();
        }

        const value = await navigator.clipboard.read().catch((err) => {
            throwIronError('Failed to read from the clipboard: ' + err);
        });

        // Clipboard is empty
        if (value.length == 0) {
            throwIronError('The clipboard has no data.');
        }

        // We only support one item at a time
        const item = value[0];

        if (!item.types.some((type) => type.startsWith('text/') || type.startsWith('image/png'))) {
            // Unsupported types
            throwIronError('The clipboard has no data of supported type (text or image).');
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
            // Explicitly clone the `clipboardData` object as `onClipboardChanged` consumes the value.
            await this.remoteDesktopService.onClipboardChanged(clipboardData.clone());
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
        try {
            if (this.lastSentClipboardData) {
                this.remoteDesktopService.onClipboardChanged(this.lastSentClipboardData.clone());
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
        this.remoteDesktopService.emitClipboardRemoteUpdateEvent();
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
    private async onMonitorClipboard(): Promise<void> {
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
                    // Explicitly clone the `clipboardData` object as `onClipboardChanged` consumes the value.
                    await this.remoteDesktopService.onClipboardChanged(clipboardData.clone());
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

    // Firefox v126 and below does not support `navigator.clipboard.read` and `navigator.clipboard.write`.
    // So, we need to define specific methods to handle text-only clipboard.
    //
    // Also, Firefox v124 and below does not support `navigator.clipboard.readText`.
    // Because of this, we cannot read the data from the clipboard at all.

    private ffClipboardDataToSave: string | null = null;

    // This function is required to retrieve the text data from the `ClipboardData`.
    private ffRetrieveTextData(data: ClipboardData): string {
        for (const item of data.items()) {
            if (item.mimeType().startsWith('text/')) {
                const value = item.value();
                if (typeof value === 'string') return value;
            }
        }

        return '';
    }

    // Firefox specific function.
    // This callback is required to update client clipboard state when remote side has changed.
    private ffOnRemoteClipboardChanged(data: ClipboardData) {
        const value = this.ffRetrieveTextData(data);
        // Non-text clipboard data is ignored.
        if (value === '') return;

        this.ffClipboardDataToSave = value;
        this.remoteDesktopService.emitClipboardRemoteUpdateEvent();
    }

    // Firefox specific function. We are using text-only clipboard API here.
    //
    // Copies clipboard content received from the server to the local clipboard.
    // Returns the result of the operation. On failure, it additionally raises an error session event.
    private async ffSaveRemoteClipboardData(): Promise<void> {
        if (this.ffClipboardDataToSave == null) {
            throwIronError('The server did not send the clipboard data.');
        }

        try {
            await navigator.clipboard.writeText(this.ffClipboardDataToSave);
            this.ffClipboardDataToSave = null;
        } catch (err) {
            throwIronError('Failed to write to the clipboard: ' + err);
        }
    }

    // Firefox specific function. We are using text-only clipboard API here.
    //
    // Sends local clipboard's content to the server.
    // Returns the result of the operation. On failure, it additionally raises an error session event.
    private async ffSendClipboardData(): Promise<void> {
        if (this.ClipboardApiSupported !== ClipboardApiSupported.TextOnly) {
            throwIronError('The browser does not support clipboard read.');
        }

        const value = await navigator.clipboard.readText().catch((err) => {
            throwIronError('Failed to read from the clipboard: ' + err);
        });

        // Clipboard is empty
        if (value.length == 0) {
            throwIronError('The clipboard has no data.');
        }

        const clipboardData = new this.module.ClipboardData();
        clipboardData.addText('text/plain', value);

        if (!clipboardData.isEmpty()) {
            this.lastSentClipboardData = clipboardData;
            // Explicitly clone the `clipboardData` object as `onClipboardChanged` consumes the value.
            await this.remoteDesktopService.onClipboardChanged(clipboardData.clone());
        }
    }
}
