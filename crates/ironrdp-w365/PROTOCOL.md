# Windows 365 redirection wire formats

No Microsoft Open Specification documents the battery status, power request, screen capture redirection, or virtual desktop control channels.
This note describes the layouts and rules this crate implements, following the Windows App 2.0.1485.0 client, so the `oracle: reference` markers in `crates/ironrdp-testsuite-core/tests/w365` have one place to point to.

All fields are little-endian.

## Battery status redirection

`Microsoft::Windows::RDS::BatteryStatusRedirection`.

### Header

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 2 | `pduType` |
| 2 | 4 | `pduLength`: the whole PDU, header included |

The client rejects a buffer shorter than 6 bytes and a `pduLength` greater than the buffer, then checks each type against a minimum `pduLength`.

### PDUs

| Type | Name | Direction | Length | Body after the header |
| --- | --- | --- | --- | --- |
| 1 | Server ready | server to client | at least 14 | `protocolVersion` u32, `protocolFlags` u32 |
| 2 | Client ready | client to server | 16 | `protocolVersion` `0x00010000`, `protocolFlags` 0, reserved u16 0 |
| 3 | Battery info request | server to client | at least 16 | `batteryState` u32, `batteryLifePercent` u16, `batteryLifeMinutes` u32 |
| 4 | Battery info changed | client to server | 16 | same fields as type 3 |

The client answers a server ready PDU with version 1 whatever the server announced.

### Battery state

`BatteryState::ON_BATTERY`, `NO_BATTERY`, and `AC_POWER` are 0, 1, and 2.
In terms of the Win32 `SYSTEM_POWER_STATUS` fields:

- When `BatteryFlag` is `0x80` (no battery) or `0xFF` (unknown), the state is 1 and the percentage and minutes are 0.
- Otherwise the state is 2 when `ACLineStatus` is 1 and 0 when it is not; the percentage is `BatteryLifePercent`; the minutes are `BatteryLifeTime / 60`, or 0 when `BatteryLifeTime` is `0xFFFFFFFF`.

This crate leaves reading the battery to the embedder.

### Pacing

A battery info request carries the state, percentage, and minutes the server currently shows, and allows the client one update.
The client sends a type 4 PDU, on that request or later when the local battery changes, only when an update is allowed and either:

- no update was sent on this channel yet, or
- the state or the percentage differs from what the server shows.

The minutes alone never trigger an update.
Sending uses up the permission until the next type 3 request.

### RemoteApp

Windows App does not open the channel in RemoteApp sessions.

## Power request redirection

`Microsoft::Windows::RDS::PowerRequestRedirection`.

The header is the same 6-byte type and length as the battery channel.
The client rejects a buffer shorter than 6 bytes and a `pduLength` greater than the buffer.

| Type | Name | Direction | Length | Body after the header |
| --- | --- | --- | --- | --- |
| 1 | Server ready | server to client | at least 14 | `protocolVersion` u32, `protocolFlags` u32 |
| 2 | Client ready | client to server | 14 | `protocolVersion` `0x00010000`, `protocolFlags` 0 |
| 3 | Power request info changed | server to client | at least 10 | `powerRequestFlags` u32 |

The client always answers with version 1, whatever the server announced.

`powerRequestFlags` is at offset 6:

- Bit `0x1` means the Cloud PC holds power requests, and bit `0x2` means it holds none.
- A PDU with neither bit or with both bits is rejected.
- Otherwise Windows App keeps the local system and display awake while bit `0x1` is set.

What Windows App does when the channel closes while requests are held was not determined; this crate reports the requests as released.

## Screen capture redirection

`Microsoft::Windows::RDS::ScreenCaptureRedirection`.

### Header

This channel uses an 8-byte header.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 2 | `operationType` |
| 2 | 2 | `flags` |
| 4 | 4 | `payloadLength`: the bytes after the header |

The client rejects a PDU unless it is at least 8 bytes and `payloadLength + 8` equals its size exactly.

### PDUs

| Type | Name | Direction | `flags` | Payload |
| --- | --- | --- | --- | --- |
| 1 | Server ready | server to client | 0 | `protocolVersion` u32, `protocolFlags` u32 |
| 2 | Client ready | client to server | 0 | `protocolVersion` `0x00010000`, `protocolFlags` 0 |
| 3 | Screen capture | client to server | image type | `captureId` u16, image bytes |
| 4 | Save result | server to client | 0 | `captureId` u16, `hresult` u32, `cbFileName` u32, `cbFilePath` u32, file name, file path |

A server ready PDU is accepted only when `flags` is 0, `payloadLength` is 8, and `protocolVersion` is `0x00010000`; only then does the client answer with the client ready PDU and allow uploads.

### Screen capture (type 3)

The header `flags` hold the image type, 1 for PNG and 2 for JPEG, and the payload is the 2-byte capture ID followed by the image, so the PDU is the image plus 10 bytes.
The client rejects an empty image and any upload while the channel is unavailable, and takes capture IDs from a counter that wraps to 0 after `0x7FFF`.

### Save result (type 4)

The client reads:

- `captureId` at offset 8, which must not exceed `0x7FFF`.
- `hresult` at offset 10, `cbFileName` at offset 14, and `cbFilePath` at offset 18.
- The file name at offset 22 and the file path right after it, as UTF-16.

It requires `cbFileName + cbFilePath` to equal `payloadLength - 14`.

## Virtual desktop control

`Microsoft::Windows::RDS::VirtualDesktopControl::Switch`, which Cloud PCs open in Boot and Switch mode.

### Header

Every PDU starts with a 6-byte header, all fields little-endian.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 2 | `pduType` |
| 2 | 4 | `pduLength`: the whole PDU, header included |

The client rejects a buffer shorter than 6 bytes and a `pduLength` greater than the buffer.
It checks each type only against a minimum `pduLength` and ignores bytes past the fields it reads.

### PDUs

| Type | Name | Direction | Minimum length | Body after the header |
| --- | --- | --- | --- | --- |
| 1 | Version request | client to server | 20 + URI | `minVersion` u32, `maxVersion` u32, `cbLaunchUri` u32, launch URI |
| 2 | Version response | server to client | 10 | `version` u32 |
| 3 | Switch-to-local request | server to client | 6 | none |
| 4 | Switch-to-local response | client to server | 10 | `hresult` u32 |
| 5 | Switch-to-cloud request | client to server | 6 | none |
| 6 | Switch-to-cloud response | server to client | 10 | `hresult` u32 |
| 7 | Perform local action | server to client | 7 | `action` u8 |

#### Version request (type 1)

The PDU is `2 * (launch URI length) + 20` bytes long:

- `minVersion` `0x00010000` at offset 6 and `maxVersion` `0x00020000` at offset 10.
- `cbLaunchUri` at offset 14: the launch URI's UTF-16 size, including its terminating NUL.
- The launch URI at offset 18, as NUL-terminated UTF-16.

The client sends it as soon as the channel opens.

#### Version response (type 2)

The client reads `version` at offset 6 and accepts versions 1 (`0x00010000`) through 2 (`0x00020000`).
Windows App closes the channel on any other version.
This crate cannot close the channel, so it ignores every later PDU instead (see the README).

#### Switch-to-local request and response (types 3 and 4)

When Windows App hands the switch to its host application, it sends no response.
Otherwise it switches and sends a 10-byte response: type 4, length 10, then the HRESULT of the switch.
`VdcSwitchHandler::switch_to_local_requested` chooses between the two by returning `None` or `Some(hresult)`.

#### Switch-to-cloud request and response (types 5 and 6)

The request is the 6-byte header alone: type 5, length 6.
Sending it needs an open channel but not a negotiated version; without a channel it fails with `E_POINTER`.
The client reads the response's HRESULT at offset 6 and takes no further action on it.

#### Perform local action (type 7)

The client handles type 7 only when `pduLength` is at least 7, a version was negotiated, and the negotiated version is at least `0x00020000`.
It reads the action as the byte at offset 6.

Each action opens a Windows Settings page:

| Value | Action | Settings URI |
| --- | --- | --- |
| 0 | Bluetooth settings | `ms-settings:bluetooth?launchedFromRemoteSession=1` |
| 1 | Sound settings | `ms-settings:sound?launchedFromRemoteSession=1` |
| 2 | Display settings | `ms-settings:display?launchedFromRemoteSession=1` |
| 3 | Account settings | `ms-settings:signinoptions?launchedFromRemoteSession=1` |
| 4 | System settings | `ms-settings:system?launchedFromRemoteSession=1` |

When its host application does not handle local actions, Windows App opens `ms-settings:devices?launchedFromRemoteSession=1` for action 0 alone and fails other actions with `E_UNEXPECTED`.

## Not verified

- No layout has been checked against a live Cloud PC session.
- Whether the server NUL-terminates the save result's file name and path was not determined, so the decoder drops trailing NULs.
- Windows App shares its screen capture counter across the whole process and its starting value was not determined; each `ScreenCaptureClient` starts at 0.
- What the server does with the launch URI in the virtual desktop control version request was not determined.
