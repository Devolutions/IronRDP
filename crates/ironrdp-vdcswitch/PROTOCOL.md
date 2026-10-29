# Virtual desktop control wire format

No Microsoft Open Specification documents `Microsoft::Windows::RDS::VirtualDesktopControl::Switch`.
This note describes the layouts and rules this crate implements, following the Windows App 2.0.1485.0 client, so the `oracle: reference` markers in `crates/ironrdp-testsuite-core/tests/vdcswitch` have one place to point to.

## Header

Every PDU starts with a 6-byte header, all fields little-endian.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 2 | `pduType` |
| 2 | 4 | `pduLength`: the whole PDU, header included |

The client rejects a buffer shorter than 6 bytes and a `pduLength` greater than the buffer.
It checks each type only against a minimum `pduLength` and ignores bytes past the fields it reads.

## PDUs

| Type | Name | Direction | Minimum length | Body after the header |
| --- | --- | --- | --- | --- |
| 1 | Version request | client to server | 20 + URI | `minVersion` u32, `maxVersion` u32, `cbLaunchUri` u32, launch URI |
| 2 | Version response | server to client | 10 | `version` u32 |
| 3 | Switch-to-local request | server to client | 6 | none |
| 4 | Switch-to-local response | client to server | 10 | `hresult` u32 |
| 5 | Switch-to-cloud request | client to server | 6 | none |
| 6 | Switch-to-cloud response | server to client | 10 | `hresult` u32 |
| 7 | Perform local action | server to client | 7 | `action` u8 |

### Version request (type 1)

The PDU is `2 * (launch URI length) + 20` bytes long:

- `minVersion` `0x00010000` at offset 6 and `maxVersion` `0x00020000` at offset 10.
- `cbLaunchUri` at offset 14: the launch URI's UTF-16 size, including its terminating NUL.
- The launch URI at offset 18, as NUL-terminated UTF-16.

The client sends it as soon as the channel opens.

### Version response (type 2)

The client reads `version` at offset 6 and accepts versions 1 (`0x00010000`) through 2 (`0x00020000`).
Windows App closes the channel on any other version.
This crate cannot close the channel, so it ignores every later PDU instead (see the README).

### Switch-to-local request and response (types 3 and 4)

When Windows App hands the switch to its host application, it sends no response.
Otherwise it switches and sends a 10-byte response: type 4, length 10, then the HRESULT of the switch.
`VdcSwitchHandler::switch_to_local_requested` chooses between the two by returning `None` or `Some(hresult)`.

### Switch-to-cloud request and response (types 5 and 6)

The request is the 6-byte header alone: type 5, length 6.
Sending it needs an open channel but not a negotiated version; without a channel it fails with `E_POINTER`.
The client reads the response's HRESULT at offset 6 and takes no further action on it.

### Perform local action (type 7)

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
- What the server does with the launch URI in the version request was not determined.
