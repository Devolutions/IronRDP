export declare const ardQualityMode: (mode: string) => Extension;

export declare const Backend: {
    DesktopSize: typeof DesktopSize;
    InputTransaction: typeof InputTransaction;
    SessionBuilder: typeof SessionBuilder;
    ClipboardData: typeof ClipboardData;
    DeviceEvent: typeof DeviceEvent;
};

declare class ClipboardData {
    free(): void;
    [Symbol.dispose](): void;
    addBinary(mime_type: string, binary: Uint8Array): void;
    addText(mime_type: string, text: string): void;
    constructor();
    isEmpty(): boolean;
    items(): ClipboardItem_2[];
}

declare class ClipboardItem_2 {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    mimeType(): string;
    value(): any;
}

declare class DesktopSize {
    free(): void;
    [Symbol.dispose](): void;
    constructor(width: number, height: number);
    height: number;
    width: number;
}

declare class DeviceEvent {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    static keyPressed(scancode: number): DeviceEvent;
    static keyReleased(scancode: number): DeviceEvent;
    static mouseButtonPressed(button: number): DeviceEvent;
    static mouseButtonReleased(button: number): DeviceEvent;
    static mouseMove(x: number, y: number): DeviceEvent;
    static unicodePressed(unicode: string): DeviceEvent;
    static unicodeReleased(unicode: string): DeviceEvent;
    static wheelRotations(vertical: boolean, rotation_amount: number, rotation_unit: RotationUnit): DeviceEvent;
}

declare interface DynamicResizingSupportedCallback {
    (): void;
}

export declare const dynamicResizingSupportedCallback: (callback: DynamicResizingSupportedCallback) => Extension;

export declare const enableCursor: (enable: boolean) => Extension;

export declare const enabledEncodings: (encodings: string) => Extension;

export declare const enableExtendedClipboard: (enable: boolean) => Extension;

declare class Extension {
    free(): void;
    [Symbol.dispose](): void;
    constructor(ident: string, value: any);
}

export declare const forceFirmwareV7: (enable: boolean) => Extension;

export declare const forceWsPort: (enable: boolean) => Extension;

export declare const init: (log_level: string) => Promise<void>;

declare class InputTransaction {
    free(): void;
    [Symbol.dispose](): void;
    addEvent(event: DeviceEvent): void;
    constructor();
}

export declare const jpegQualityLevel: (level: number) => Extension;

export declare const pixelFormat: (format: string) => Extension;

export declare const requestSharedSession: (value: boolean) => Extension;

export declare const resolutionQuality: (quality: string) => Extension;

declare enum RotationUnit {
    Pixel = 0,
    Line = 1,
    Page = 2,
}

declare class Session {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    applyInputs(transaction: InputTransaction): void;
    desktopSize(): DesktopSize;
    invokeExtension(ext: Extension): any;
    onClipboardPaste(content: ClipboardData): Promise<void>;
    releaseAllInputs(): void;
    resize(width: number, height: number, scale_factor?: number | null, physical_width?: number | null, physical_height?: number | null): void;
    run(): Promise<SessionTerminationInfo>;
    shutdown(): void;
    supportsUnicodeKeyboardShortcuts(): boolean;
    synchronizeLockKeys(scroll_lock: boolean, num_lock: boolean, caps_lock: boolean, kana_lock: boolean): void;
}

declare class SessionBuilder {
    free(): void;
    [Symbol.dispose](): void;
    authToken(token: string): SessionBuilder;
    canvasResizedCallback(callback: Function): SessionBuilder;
    connect(): Promise<Session>;
    constructor();
    desktopSize(desktop_size: DesktopSize): SessionBuilder;
    destination(destination: string): SessionBuilder;
    extension(ext: Extension): SessionBuilder;
    forceClipboardUpdateCallback(callback: Function): SessionBuilder;
    password(password: string): SessionBuilder;
    proxyAddress(address: string): SessionBuilder;
    remoteClipboardChangedCallback(callback: Function): SessionBuilder;
    renderCanvas(canvas: HTMLCanvasElement): SessionBuilder;
    serverDomain(server_domain: string): SessionBuilder;
    setCursorStyleCallback(callback: Function): SessionBuilder;
    setCursorStyleCallbackContext(context: any): SessionBuilder;
    username(username: string): SessionBuilder;
}

declare class SessionTerminationInfo {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    reason(): string;
}

export declare const sharingApprovalMode: (mode: string) => Extension;

export declare const ultraVirtualDisplay: (enable: boolean) => Extension;

export declare const vmId: (id: string) => Extension;

export declare const wheelSpeedFactor: (factor: number) => Extension;

export { }
