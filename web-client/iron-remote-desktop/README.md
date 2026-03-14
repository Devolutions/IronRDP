# Iron Remote Desktop

This is the core of the web client written on top of Svelte and built as a reusable Web Component.
Also, it contains the TypeScript interfaces exposed by WebAssembly bindings from `ironrdp-web` and used by `iron-svelte-client`.

## Development

Make you modification in the source code then use [iron-svelte-client](../iron-svelte-client) to test.

## Build

Run `npm run build`

## Usage

As member of the Devolutions organization, you can import the Web Component from JFrog Artifactory by running the following npm command:

```shell
$ npm install @devolutions/iron-remote-desktop
```

Otherwise, you can run `npm install` targeting the `dist/` folder directly.

Import the `iron-remote-desktop.umd.cjs` from `node_modules/` folder.

Then use the HTML tag `<iron-remote-desktop/>` in your page.

In your code add a listener for the `ready` event on the `iron-remote-desktop` HTML element.
Get `evt.detail.irgUserInteraction` from the `Promise`, a property whose type is `UserInteraction`.
Call the `connect` method on this object.

## Limitations

Mouse input supports movement and click (4 buttons) but not scroll.
Keyboard support covers the standard layout.
Windows key and CTRL+ALT+DEL can be sent via methods on `UserInteraction`.
Lock keys (like Caps Lock) have partial support.

## Component parameters

You can add some parameters for default initialization on the component `<iron-remote-desktop />`.

> Note that due to a limitation of the framework all parameters need to be lower-cased.

- `scale`: The scaling behavior of the distant screen. Can be `fit`, `real` or `full`. Default is `real`;
- `verbose`: Show logs from `iron-remote-desktop`. `true` or `false`. Default is `false`.
- `debugwasm`: Show debug info from web assembly. Can be `"OFF"`, `"ERROR"`, `"WARN"`, `"INFO"`, `"DEBUG"`, `"TRACE"`. Default is `"OFF"`.
- `flexcentre`: Helper to force `iron-remote-desktop` a flex and centering the content automatically. Otherwise, you need to manage manually. Default is `true`.
- `module`: An implementation of the [RemoteDesktopModule](./src/interfaces/RemoteDesktopModule.ts)

## `UserInteraction` methods

```ts
connect(
  username: string,
  password: string,
  destination: string,
  proxyAddress: string,
  serverDomain: string,
  authToken: string,
  desktopSize?: DesktopSize,
  preConnectionBlob?: string,
  kdc_proxy_url?: string,
  use_display_control: boolean,
): Observable<NewSessionInfo>;
```

> `username` and `password` are the credentials to use on the remote host.

> `destination` refers to the Devolutions Gateway hostname and port.

> `proxyAddress` is the address of the Devolutions Gateway proxy

> `serverDomain` is the Windows domain name (if the target computer has one)

> `authtoken` is the authentication token to send to the Devolutions Gateway.

> `desktopSize` is the initial size of the desktop

> `preConnectionBlob` is the pre connection blob data

> `kdc_proxy_url` is the URL to a KDC Proxy, as specified in [MS-KKDCP documentation](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-kkdcp/5bcebb8d-b747-4ee5-9453-428aec1c5c38)

> `use_display_control` is the value that defined if the Display Control Virtual Channel will be used.

> `ctrlAltDel()`
>
> Sends the ctrl+alt+del key to server.

> `metaKey()`
>
> Sends the meta key event to remote host (i.e.: Windows key).

> `setVisibility(value: boolean)`
>
> Shows or hides rendering canvas.

> `setScale(scale: ScreenScale)`
>
> Sets the scale behavior of the canvas.
> See the [ScreenScale](./src/enums/ScreenScale.ts) enum for possible values.

> `shutdown()`
>
> Shutdowns the active session.

> `setKeyboardUnicodeMode(use_unicode: boolean)`
>
> Sets the keyboard Unicode mode.

> `setCursorStyleOverride(style?: string)`
>
> Overrides the default cursor style. If `style` is `null`, the default cursor style will be used.

> `resize(width: number, height: number, scale?: number)`
>
> Resizes the screen.

> `setEnableClipboard(enable: boolean)`
>
> Enables or disable the clipboard based on the `enable` value.

## File Transfer

The `FileTransferManager` provides a high-level API for bidirectional file transfer operations in RDP sessions.

### Features

- **Download files** from remote to browser with automatic chunking and reassembly
- **Upload files** from browser to remote with progress tracking
- **Browser integration helpers** for file picking and drag-and-drop
- **Progress events** for building responsive UIs
- **Cancellation support** via AbortController
- **Configurable chunk size** (default: 64KB)

### Basic Usage

```typescript
import { FileTransferManager } from '@devolutions/iron-remote-desktop';

// Create and configure manager with SessionBuilder
const builder = new IronRemoteDesktop.SessionBuilder();
const manager = FileTransferManager.setup(builder, { chunkSize: 64 * 1024 });

// Configure session as usual
builder
  .username('user')
  .password('pass')
  .destination('rdp.example.com')
  .proxyAddress('gateway.example.com')
  .authToken('token');

// Connect - manager is automatically ready after this
const session = await builder.connect();

// Listen for files available for download
manager.on('files-available', async (files) => {
  for (let i = 0; i < files.length; i++) {
    const blob = await manager.downloadFile(files[i], i);
    // Trigger browser download (using your preferred method)
    saveAs(blob, files[i].name);
  }
});

// Track download progress
manager.on('download-progress', (progress) => {
  console.log(`${progress.fileName}: ${progress.percentage}%`);
});
```

### Uploading Files

```typescript
// Using file picker (must be triggered by user gesture)
button.onclick = async () => {
  const files = await manager.showFilePicker({ multiple: true });
  await manager.uploadFiles(files);
};

// Using drag-and-drop
dropZone.addEventListener('dragover', (e) => manager.handleDragOver(e));
dropZone.addEventListener('drop', async (e) => {
  const files = manager.handleDrop(e);
  await manager.uploadFiles(files);
});

// Or use your own file input
const fileInput = document.querySelector('input[type=file]');
const files = Array.from(fileInput.files);
await manager.uploadFiles(files);
```

### Cancellation

```typescript
const controller = new AbortController();

// Start download with cancellation support
const blob = await manager.downloadFile(fileInfo, 0, controller.signal);

// Cancel from another handler
cancelButton.onclick = () => controller.abort();
```

### Events

- `files-available` - Emitted when remote copies files (provides array of FileInfo)
- `download-progress` - Emitted during downloads (provides TransferProgress)
- `upload-progress` - Emitted during uploads (provides TransferProgress)
- `download-complete` - Emitted when a download finishes (provides FileInfo and Blob)
- `upload-complete` - Emitted when an upload finishes (provides File)
- `transfer-cancelled` - Emitted when a transfer is cancelled (provides file index)
- `error` - Emitted on transfer errors (provides FileTransferError)

### Low-Level API

For advanced use cases, you can use the low-level session methods directly:

```typescript
import { FileContentsFlags } from '@devolutions/iron-remote-desktop';

// Lock clipboard for file transfer
const clipDataId = await session.lockClipboard();

// Request file size
session.requestFileContents(streamId, fileIndex, FileContentsFlags.SIZE, 0, 8, clipDataId);

// Request file data
session.requestFileContents(streamId, fileIndex, FileContentsFlags.RANGE, offset, chunkSize, clipDataId);

// Unlock clipboard when done
session.unlockClipboard(clipDataId);

// Submit file contents (for uploads)
session.submitFileContents(streamId, isError, data);

// Initiate file copy (broadcast file list)
session.initiateFileCopy(files);
```

**Important:** When using `requestFileContents()`, the `fileIndex` parameter must be a non-negative integer (>= 0). The implementation validates this requirement per MS-RDPECLIP 2.2.5.3 and will reject negative indices. Ensure you validate file indices before calling this method.
