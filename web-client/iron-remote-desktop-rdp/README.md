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

## Standard RDP Security

`enableStandardRdpSecurity(true)` opts into `PROTOCOL_RDP` with
`ENCRYPTION_LEVEL_NONE` and disables TLS and CredSSP negotiation. This is
intended only for trusted RDCleanPath deployments where the WebSocket and the
proxy-to-server transport already provide the required security.

```ts
import { ConfigBuilder } from '@devolutions/iron-remote-desktop';
import { enableStandardRdpSecurity } from '@devolutions/iron-remote-desktop-rdp';

const config = new ConfigBuilder()
  .withDestination('localhost:3389')
  .withProxyAddress('wss://example.test/rdp')
  .withAuthToken('token')
  .withExtension(enableStandardRdpSecurity(true))
  .build();
```

## RDCleanPath errors

When the proxy rejects the connection, the error's `rdcleanpathDetails()` carries the RDCleanPath `errorCode`.
`RDCleanPathErrorCode` names the standard codes; codes 16384 to 32767 are specific to the proxy in use.

```ts
import { RDCleanPathErrorCode } from '@devolutions/iron-remote-desktop-rdp';

if (error.rdcleanpathDetails()?.errorCode === RDCleanPathErrorCode.EXPIRED) {
  // Ask the user to sign in again
}
```

## Audio playback

Register `audioPlayback(callback)` before connecting to receive audio from the remote desktop.
Playback is disabled unless a callback is supplied.
The embedding application owns playback, scheduling, and audio resources.

The callback receives these message shapes.

| Message                                        | Fields                                                                                                                           |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `{ type: 'wave', sampleRate, channels, data }` | `sampleRate` is in Hz; `channels` is the channel count; `data` is a `Uint8Array` of interleaved signed 16-bit little-endian PCM. |
| `{ type: 'volume', left, right }`              | Each number is the corresponding raw SNDVOL channel value divided by 65535, ranging from 0 (silence) to 1 (full volume).         |
| `{ type: 'close' }`                            | Playback closed or format negotiation started, including the initial negotiation before any wave.                                |

The volume numbers preserve the protocol's logarithmic level scale, as specified by [MS-RDPEA §2.2.4.1 Volume PDU] and [§2.2.2.2 Client Audio Formats and Version PDU].
They are not linear amplitude gains; mapping them to a playback API is the embedding application's responsibility.
The extension advertises stereo 16-bit PCM at 48,000, 44,100, and 22,050 Hz in that preference order.

A `close` message can arrive while the application has no active playback and again during renegotiation.
It does not mean the RDP session ended, and subsequent wave messages can resume playback.

[MS-RDPEA §2.2.4.1 Volume PDU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/d13a7930-00cc-4cdb-a17d-7bda00f6edb7
[§2.2.2.2 Client Audio Formats and Version PDU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/d58d4d57-cb36-453a-866f-86beacc6d79b

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
