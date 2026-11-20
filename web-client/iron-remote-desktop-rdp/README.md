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

## Camera

Register `camera(callback)` before connecting to redirect one camera to the remote desktop, shown there as `Browser camera` or the name given with `cameraName(name)`.
The device offers MJPEG at 640x480 (30 and 15 fps), 320x240 (15 fps) and 1280x720 (15 fps), in that preference order.
Its current media type is the first offered until the server selects one, and stays the last selected across stops.
The embedding application owns the capture and the JPEG encoding.

The callback receives these message shapes.

| Message                                 | Fields                                                                           |
| --------------------------------------- | -------------------------------------------------------------------------------- |
| `{ type: 'start', width, height, fps }` | The format the server started the stream in; capture in it until a stop message. |
| `{ type: 'stop' }`                      | The server stopped the stream, deactivated the device or closed its channel.     |

A stream restarted in another format gets a stop message before its start message.
No stop message follows the end of the session itself; the session's end is that signal.

After a start message, push each frame as one JPEG with `session.invokeExtension(cameraSample(frame))`.
The server asks for frames one Sample Request at a time ([MS-RDPECAM §2.2.3.13 Sample Request]).
A frame answers the oldest pending request at once, and `invokeExtension` returns `true`; with no request pending the frame is held, in place of any older held frame, until the next request, and `invokeExtension` returns `false`.
It also returns `false` when no stream is running, in which case the frame is dropped, and when the server refused the frame and got a Sample Error Response instead.
A held frame is discarded when the stream stops or restarts, so a new stream never gets a frame of the old size.

[MS-RDPECAM §2.2.3.13 Sample Request]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

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

To announce several printers, such as every printer of the local computer, pass
`printers([{ name, driverName?, deviceId?, isDefault?, network? }])` instead of
`printerName`, `printerDeviceId` and `printerDriverName`. Printers without a
`deviceId` get the lowest free ID from 2 up; when none is marked `isDefault`, the
first one is the default. `onJobStart(fileId, deviceId)` tells which printer a job
was sent to, and the optional `onPrinterAnnounced(deviceId, accepted, resultCode)`
reports the host's answer to each announcement. A host that lacks the named
driver may still accept the announcement and only log that it could not create
the queue.

## Drive Redirection

A page cannot open local files, so redirected drives are served by the page
itself. Announce them with `drives([{ deviceId, name }])` (device IDs unique among
drives and printers) and register `driveRequestCallback(request => ...)`. Each
filesystem request of the host arrives there as a `DriveRequest`; answer every one
with a nonzero `id` through `session.invokeExtension(driveReply({ id, status, ... }))`,
in any order. `status` is an NTSTATUS (0 is success).

The requests are `create` (path relative to the drive root, with the raw MS-SMB2
access, disposition and options; reply with `fileId` and `information`), `close`,
`read`, `write`, `stat`, `list` (a `\dir\pattern` query; reply with every matching
entry at once, which WASM then enumerates), `volume` (`total` and `free` bytes),
`setBasic`, `setSize`, `setDelete` and `rename`. `announced` reports the host's
answer to each drive's announcement and `reset` means every handle is gone; neither
takes a reply. Times are Unix milliseconds. Flushes, byte-range locks and device
controls are answered in WASM; change notifications are never completed.

The page decides what a drive exposes: it must confine paths to the shared folder
itself (the host can send `..` or names of links).
