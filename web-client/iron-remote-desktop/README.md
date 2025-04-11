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

For now, we didn't make the enums used by some method directly available (I didn't find the good way to export them directly with the component.).
You need to recreate them on your application for now (it will be improved in future version);

Also, even if the connection to RDP work there is still a lot of improvement to do.
As of now, you can expect, mouse movement and click (4 buttons) - no scroll, Keyboard for at least the standard.
Windows and CTRL+ALT+DEL can be called by method on `UserInteraction`.
Lock keys (like caps lock), have a partial support.
Other advanced functionalities (sharing / copy past...) are not implemented yet.

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
