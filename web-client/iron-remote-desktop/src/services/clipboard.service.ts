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
    // When true, the clipboard monitoring loop skips reading/sending clipboard updates.
    // Used to prevent the monitoring loop from clobbering an active file upload's
    // FormatList with a text/image clipboard update.
    private monitoringSuppressed: boolean = false;

    constructor(remoteDesktopService: RemoteDesktopService, module: RemoteDesktopModule) {
        this.remoteDesktopService = remoteDesktopService;
        this.module = module;
    }

    /**
     * Suppress clipboard monitoring. While suppressed, the 100ms monitoring
     * loop will skip reading the local clipboard and sending updates to the
     * remote. This prevents the monitor from clobbering a file upload's
     * FormatList announcement with a text/image clipboard update.
     */
    suppressMonitoring(): void {
        this.monitoringSuppressed = true;
    }

    /**
     * Resume clipboard monitoring after a previous {@link suppressMonitoring} call.
     */
    resumeMonitoring(): void {
        this.monitoringSuppressed = false;
    }

    async initClipboard() {
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

        // Gate the polling-based auto-clipboard loop behind a Permissions API
        // check. Two cases are handled:
        //
        // 1. Chromium: The query succeeds and returns a PermissionStatus.
        //    - 'granted': Keep Full mode; auto-clipboard polling works.
        //    - 'prompt': Keep Full mode; Chromium will show a one-time
        //      permission prompt on the first clipboard.read() call. If
        //      the user denies it, the safety net in onMonitorClipboard
        //      catches the NotAllowedError and stops the loop.
        //    - 'denied': Downgrade to TextOnly; the polling loop would
        //      fail on every iteration.
        //
        // 2. Firefox v127+: Exposes clipboard.read()/write() but does not
        //    include "clipboard-read" in its PermissionName WebIDL enum, so
        //    the query throws. Without persistent permission, Firefox requires
        //    transient user activation for every clipboard.read() call, making
        //    the polling loop unusable. Downgrade to TextOnly so Firefox
        //    routes through the text-only fallback paths.
        //
        //    When the permission query fails, a trial clipboard.read() checks
        //    whether Firefox's `dom.events.testing.asyncClipboard` about:config
        //    pref is active. If so, keep Full mode as clipboard works fully in
        //    this scenario, without any user-activation restrictions.
        if (this.ClipboardApiSupported === ClipboardApiSupported.Full) {
            try {
                const permissionStatus = await navigator.permissions.query({
                    name: 'clipboard-read' as PermissionName,
                });

                if (permissionStatus.state === 'denied') {
                    this.ClipboardApiSupported = ClipboardApiSupported.TextOnly;
                }
            } catch {
                try {
                    // Try to read clipboard to check if the asyncClipboard pref is enabled
                    await navigator.clipboard.read();
                } catch {
                    this.ClipboardApiSupported = ClipboardApiSupported.TextOnly;
                }
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

                // Start the clipboard monitoring loop after session has been started
                this.remoteDesktopService.sessionStartedObservable.subscribe((_) => {
                    this.scheduleOnMonitorClipboardUpdate();
                });
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
            await this.remoteDesktopService.onClipboardChanged(clipboardData);
        }
    }

    private scheduleOnMonitorClipboardUpdate() {
        setTimeout(this.onMonitorClipboard.bind(this), CLIPBOARD_MONITORING_INTERVAL_MS);
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
        let stopped = false;
        try {
            if (this.monitoringSuppressed) {
                return;
            }

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
                    await this.remoteDesktopService.onClipboardChanged(clipboardData);
                }
            }
        } catch (err) {
            if (err instanceof DOMException && err.name === 'NotAllowedError') {
                // The browser requires user activation for clipboard reads (e.g. Firefox v127+).
                // The polling loop cannot work in this environment; fall back to manual mode.
                console.warn('Clipboard monitoring disabled: browser requires user activation for clipboard read.');
                this.remoteDesktopService.setOnRemoteClipboardChanged(
                    this.onRemoteClipboardChangedManualMode.bind(this),
                );
                stopped = true;
                return;
            }

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
            if (!stopped && !get(isComponentDestroyed)) {
                this.scheduleOnMonitorClipboardUpdate();
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
            await this.remoteDesktopService.onClipboardChanged(clipboardData);
        }
    }
}
