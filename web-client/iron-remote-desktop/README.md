# Iron Remote Desktop

Reusable web component and NPM package for remote desktop sessions, built with Svelte.

## Design Philosophy

`iron-remote-desktop` is **protocol-agnostic**. It knows nothing about RDP, VNC, or any other
specific remote protocol. It defines only features that are universal across all remote backends:
keyboard and mouse input, canvas rendering and resize, clipboard text/binary, connection
lifecycle, and cursor style.

### Backends

A **backend** implements the `RemoteDesktopModule` interface and plugs in via the `module`
component property/prop (for example, by assigning `element.module = Backend` or the
framework-specific equivalent). The RDP backend is `iron-remote-desktop-rdp`; other backends
can be written against the same interfaces.

### Extension mechanism

Protocol-specific features that have no equivalent in other protocols must never be added to
`UserInteraction`, `Session`, or `SessionBuilder`. They belong in the backend package and are
delivered via the extension mechanism:

```typescript
// Backend-defined factory (in iron-remote-desktop-rdp):
import { preConnectionBlob, displayControl } from '@devolutions/iron-remote-desktop-rdp';

// Consumer configures protocol-specific behaviour through extensions on the UserInteraction
// instance received from the `ready` event:
ironRemoteDesktop.addEventListener('ready', (event) => {
  const ui = event.detail;

  const config = ui.configBuilder().withExtension(preConnectionBlob('...')).withExtension(displayControl(true)).build();

  ui.connect(config);
});
```

The `Extension` type is `unknown` in `iron-remote-desktop`, opaque by design. The component
passes extension values to the backend without inspection; the backend interprets them.

At runtime, `invokeExtension(ext)` follows the same pattern for dynamic, post-connect control.

**The guiding question for any new `UserInteraction` / `Session` / `SessionBuilder` method:**

A method belongs in the base API if **either** of the following is true:

1. **The web component itself needs to call it** to implement transparent, protocol-independent
   behaviour (e.g., `supportsUnicodeKeyboardShortcuts()` is called by the component to adapt
   keyboard handling, without consumer involvement).
2. **The feature is universal**: every reasonable remote protocol backend would implement it
   in a meaningful way (e.g., resize, clipboard text, cursor style).

If neither applies, it is protocol-specific and must go through extensions.

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

## Supported Input

Mouse: movement, click (4 buttons), scroll. Keyboard: standard layout, Windows key,
Ctrl+Alt+Del. Lock keys (Caps Lock, Num Lock, Scroll Lock, Kana): partial support.

## Component parameters

You can add some parameters for default initialization on the component `<iron-remote-desktop />`.

> Note that due to a limitation of the framework all parameters need to be lower-cased.

- `scale`: The scaling behavior of the distant screen. Can be `fit`, `real` or `full`. Default is `real`;
- `verbose`: Show logs from `iron-remote-desktop`. `true` or `false`. Default is `false`.
- `debugwasm`: Show debug info from web assembly. Can be `"OFF"`, `"ERROR"`, `"WARN"`, `"INFO"`, `"DEBUG"`, `"TRACE"`. Default is `"OFF"`.
- `flexcentre`: Helper to force `iron-remote-desktop` a flex and centering the content automatically. Otherwise, you need to manage manually. Default is `true`.
- `module`: An implementation of the [RemoteDesktopModule](./src/interfaces/RemoteDesktopModule.ts)

## `UserInteraction` methods

Build a `Config` using `configBuilder()`, then call `connect(config)`. Protocol-specific
configuration (e.g., pre-connection blob, KDC proxy URL) is passed via extensions on the
`ConfigBuilder` — see the backend package for available extension factories.

```ts
configBuilder(): ConfigBuilder;
connect(config: Config): Promise<NewSessionInfo>;
```

> `ctrlAltDel()` — Sends Ctrl+Alt+Del to the remote host.

> `metaKey()` — Sends the Windows/Meta key to the remote host.

> `setVisibility(value: boolean)` — Shows or hides the rendering canvas.

> `setScale(scale: ScreenScale)` — Sets canvas scaling behaviour (`fit`, `real`, or `full`).
> See [`ScreenScale`](./src/enums/ScreenScale.ts).

> `shutdown()` — Terminates the active session.

> `setKeyboardUnicodeMode(useUnicode: boolean)` — Toggles Unicode keyboard mode.

> `setCursorStyleOverride(style: string | null)` — Overrides cursor style; `null` restores default.

> `resize(width: number, height: number, scale?: number)` — Resizes the remote screen.

> `setEnableClipboard(enable: boolean)` — Enables or disables clipboard synchronization.

> `setEnableAutoClipboard(enable: boolean)` — Enables or disables automatic clipboard polling.

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

> `invokeExtension(ext: Extension)` — Sends a protocol-specific extension command at runtime.
> The extension value is passed to the backend without inspection.

## File Transfer

File transfer is protocol-specific. The `iron-remote-desktop` package defines only the
protocol-agnostic `FileTransferProvider` interface; the implementation lives in the backend
package (e.g., `RdpFileTransferProvider` in `@devolutions/iron-remote-desktop-rdp`).

### Enabling File Transfer

```typescript
import { RdpFileTransferProvider } from '@devolutions/iron-remote-desktop-rdp';

// Create a provider and pass it to the web component
const provider = new RdpFileTransferProvider({ chunkSize: 64 * 1024 });
component.enableFileTransfer(provider);

// Connect as usual - the provider receives builder extensions and session automatically
await component.connect(config);

// Listen for files available for download
provider.on('files-available', async (files) => {
  for (let i = 0; i < files.length; i++) {
    const { completion } = provider.downloadFile(files[i], i);
    const blob = await completion;
    saveAs(blob, files[i].name);
  }
});

// Track progress
provider.on('download-progress', (progress) => {
  console.log(`${progress.fileName}: ${progress.percentage}%`);
});

// Upload files via drag-and-drop or file picker
const dropped = await provider.handleDrop(event);
provider.uploadFiles(dropped);
```

See the `@devolutions/iron-remote-desktop-rdp` package for the full API, events, and
extension factories.
