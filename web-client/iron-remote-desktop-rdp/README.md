# Iron Remote Desktop RDP

This is implementation of `RemoteDesktopModule` interface from [iron-remote-desktop](../iron-remote-desktop) for RDP connection.

## Development

Make your modification in the source code then use [iron-svelte-client](../iron-svelte-client) to test.

## Build

Run `npm run build`

## Usage

As member of the Devolutions organization, you can import the Web Component from JFrog Artifactory by running the following npm command:

```shell
$ npm install @devolutions/iron-remote-desktop-rdp
```

Otherwise, you can run `npm install` targeting the `dist/` folder directly.

Import the `iron-remote-desktop-rdp.umd.cjs` from `node_modules/` folder.

## Virtual Printer

Register `printJobStreamCallbacks` before connecting to enable the browser-side
RDPDR virtual printer. The RDPDR backend forwards write chunks as they arrive
instead of buffering the completed job in Rust.

By default, the web connector follows FreeRDP's macOS heuristic where possible:
browser-reported macOS 14+ uses `Microsoft Print to PDF`, and other clients use
`MS Publisher Imagesetter` for PostScript data. Pass
`printerDriverName(PrinterDriverName.PostScript)` or another explicit driver if
your target host requires a different installed driver. Jobs larger than 128 MiB
are rejected, and queued write chunks are bounded to protect browser memory.
