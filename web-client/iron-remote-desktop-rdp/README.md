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

Register `printJobCompleteCallback` before connecting to enable the browser-side
RDPDR virtual printer. The default server-side driver is
`MS Publisher Imagesetter`, which produces PostScript data; pass
`printerDriverName(...)` if your target host requires a different installed
driver. The callback receives the completed job as a single `Uint8Array`, so the
application should convert PostScript to PDF before opening a browser print
dialog. Jobs larger than 128 MiB are rejected to protect browser memory.
