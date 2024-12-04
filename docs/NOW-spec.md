<!--
This file was created using Markdown Monster (https://markdownmonster.west-wind.com), which offers
extended formatting capabilities for markdown files, such as TOC and embedded HTML.
-->
# NOW-PROTO 1.0

<style>
    .byte-layout {
        width: 100%;
        table-layout: fixed;
    }
    .byte-layout th {
        colspan: 1;
        text-align: center;
        vertical-align: bottom;
    }
    .byte-layout td {
        text-align: center;
    }
</style>


[[_TOC_]]

# Messages

## Transport

The NOW virtual channel protocol use an RDP dynamic virtual channel ("Devolutions::Now::Agent") as a transport type.

## Message Syntax

The following sections specify the NOW protocol message syntax. Unless otherwise specified, all fields defined in this document use the little-endian format.

### Common Structures

#### NOW_INTEGER

Signed and unsigned integer encoding structures of various sizes.

##### NOW_VARU32

The NOW_VARU32 structure is used to encode signed integer values in the range [0, 0x3FFFFFFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="2">c</td>
            <td colspan="6">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="8">val3 (optional)</td>
            <td colspan="8">val4 (optional)</td>
        </tr>
    </tbody>
</table>

**c (2 bits)**: A 2-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |
| 2 | The val1, val2, val3 fields are present (3 bytes) |
| 3 | The val1, val2, val3, val4 fields are present (4 bytes) |

**val1 (6 bits)**: A 6-bit integer containing the 6 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: An 8-bit integer containing the second most significant bits of the integer value represented by this structure.

**val3 (1 byte)**: An 8-bit integer containing the third most significant bits of the integer value represented by this structure.

**val4 (1 byte)**: An 8-bit integer containing the least significant bits of the integer value represented by this structure.

#### NOW_STRING

##### NOW_VARSTR

The NOW_VARSTR structure is used to represent variable-length strings that could be large, while remaining compact in size for small strings.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">len (variable)</td>
        </tr>
        <tr>
            <td colspan="32">str (variable)</td>
        </tr>
    </tbody>
</table>

**len (variable)**: A NOW_VARU32 structure containing the string length, excluding the null terminator.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

#### NOW_HEADER

The NOW_HEADER structure is the header common to all NOW protocol messages.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class.

| Flag                            | Meaning              |
|---------------------------------|----------------------|
| NOW_NEGOTIATION_MSG_CLASS_ID<br>0x10 | Protocol negotiation class |
| NOW_SYSTEM_MSG_CLASS_ID<br>0x11 | System message class |
| NOW_SESSION_MSG_CLASS_ID<br>0x12 | Session message class |
| NOW_EXEC_MSG_CLASS_ID<br>0x13 | Exec message class |

**msgType (1 byte)**: The message type, specific to the message class.

**msgFlags (2 bytes)**: The message flags, specific to the message type and class.

#### NOW_STATUS
Operation execution status code.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="16">flags</td>
            <td colspan="8">kind</td>
            <td colspan="8">reserved</td>
        </tr>
        <tr>
            <td colspan="32">code</td>
        </tr>
    </tbody>
</table>

**flags (2 bytes)**: Status flags.

| Value | Meaning |
|-------|---------|
| NOW_EXEC_RESULT_ERROR<br>0x0001 | This flag set for all error statuses. |
| NOW_EXEC_RESULT_ERROR_MESSAGE<br>0x0002 | `errorMessage` field is present and contains optional error message. |

**kind (1 byte)**: The status kind.
When `NOW_EXEC_RESULT_ERROR` is set, this field represents error kind.

| Value | Meaning |
|-------|---------|
| NOW_EXEC_RESULT_ERROR_KIND_GENERIC<br>0x0000 | `code` value is undefined and could be ignored. |
| NOW_EXEC_RESULT_ERROR_KIND_NOW<br>0x0001 | `code` contains NowProto-defined error code. |
| NOW_EXEC_RESULT_ERROR_KIND_WINAPI<br>0x0002 | `code` field contains Windows error code. |
| NOW_EXEC_RESULT_ERROR_KIND_UNIX<br>0x0003 | `code` field contains Unix error code. |

For successful operation this field value is operation specific.

**reserved (1 byte)**: Reserved value. Should be set to 0 and ignored during parsing.

**code (4 bytes)**: The status code.

- If `NOW_EXEC_RESULT_ERROR` flag is NOT set, this value represents operation-specific code (e.g. process exit code).
- If `NOW_EXEC_RESULT_ERROR` is set, this value represents error code according to
  `NOW_EXEC_RESULT_ERROR_KIND_*` value. If no error kind flags set, value of this
  field is undefined and should be ignored.

    - `NOW_EXEC_RESULT_ERROR_KIND_NOW` codes:

        | Value | Meaning |
        |-------|---------|
        | NOW_CODE_IN_USE<br>0x0001 | Resource (e.g. exec session id is already in use). |
        | NOW_CODE_INVALID_REQUEST<br>0x0002 | Sent request is invalid (e.g. invalid exec request params). |
        | NOW_CODE_ABORTED<br>0x0003 | Operation has been aborted on the server side. |
        | NOW_CODE_NOT_FOUND<br>0x0004 | Resource not found. |
        | NOW_CODE_ACCESS_DENIED<br>0x0005 | Resource can't be accessed. |
        | NOW_CODE_INTERNAL<br>0x0006 | Internal error. |
        | NOW_CODE_NOT_IMPLEMENTED<br>0x0007 | Operation is not implemented on current platform. |

    - `NOW_EXEC_RESULT_ERROR_KIND_WINAPI`: code contains standard WinAPI error.
    - `NOW_EXEC_RESULT_ERROR_KIND_UNIX`: code contains standard UNIX error code.

**errorMessage(variable, optional)**: this value contains optional error message if
`NOW_EXEC_RESULT_ERROR_MESSAGE` flag is set

### Negotiation Messages

#### NOW_NEGOTIATION_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_NEGOTIATION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value                           | Meaning              |
|---------------------------------|----------------------|
| NOW_NEGOTIATION_CAPSET_MSG_ID<br>0x01 | NOW_NEGOTIATION_CAPSET_MSG |

#### NOW_NEGOTIATION_CAPSET_MSG

This message is first set by the server side, to advertise capabilities.

Received server message should be downgraded by the client (remove non-intersecting capabilities) and sent back to the server at the start of DVC channel communications. DVC channel should be closed if protocol
versions are not compatible.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="16">versionMajor</td>
            <td colspan="16">versionMinor</td>
        </tr>
        <tr>
            <td colspan="32">systemCapset</td>
        </tr>
        <tr>
            <td colspan="32">sessionCapset</td>
        </tr>
        <tr>
            <td colspan="32">execCapset</td>
        </tr>
        <tr>
            <td colspan="32">heartbeatInterval(optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_NEGOTIATION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_NEGOTIATION_CAPSET_MSG_ID).

**msgFlags (2 bytes)**: Message flags.

| Flag | Meaning |
|-------|---------|
| NOW_NEGOTIATION_SET_HEATBEAT<br>0x0001 | `heartbeat` field is present. |

**versionMajor (1 byte)**: Major protocol version. Breaking changes in protocol should
increment major version; Protocol implementations with different major version are not compatible.

**versionMinor (1 byte)**: Minor protocol version. Incremented when new non-breaking feature is added.

**systemCapset (4 bytes)**: System commands capabilities set.

| Flag | Meaning |
|-------|---------|
| NOW_CAP_SYSTEM_SHUTDOWN<br>0x0001 | System shutdown command support. |

**sessionCapset (4 bytes)**: Session commands capabilities set.

| Flag | Meaning |
|-------|---------|
| NOW_CAP_SESSION_LOCK<br>0x00000001 | Lock command support. |
| NOW_CAP_SESSION_LOGOFF<br>0x00000002 | Logoff command support. |
| NOW_CAP_SESSION_MSGBOX<br>0x00000004 | Message box command support. |

**execCapset (4 bytes)**: Remote execution capabilities set.

| Flag | Meaning |
|-------|---------|
| NOW_CAP_EXEC_STYLE_RUN<br>0x00000001 | Generic "Run" execution style. |
| NOW_CAP_EXEC_STYLE_PROCESS<br>0x00000002 | CreateProcess() execution style. |
| NOW_CAP_EXEC_STYLE_SHELL<br>0x00000004 | System shell (.sh) execution style. |
| NOW_CAP_EXEC_STYLE_BATCH<br>0x00000008 | Windows batch file (.bat) execution style. |
| NOW_CAP_EXEC_STYLE_WINPS<br>0x00000010 | Windows PowerShell (.ps1) execution style. |
| NOW_CAP_EXEC_STYLE_PWSH<br>0x00000020 | PowerShell 7 (.ps1) execution style. |

<!-- TODO: add AppleScript command -->

**heartbeatInterval (4 bytes, optional)**: A 32-bit unsigned integer, which represents
periodic heartbeat interval *hint* for a server (60 seconds by default).
Disables periodic heartbeat if set to `0`.

### System Messages

#### NOW_SYSTEM_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SYSTEM_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value                           | Meaning              |
|---------------------------------|----------------------|
| NOW_SYSTEM_INFO_REQ_ID<br>0x01 | NOW_SYSTEM_INFO_REQ_MSG |
| NOW_SYSTEM_INFO_RSP_ID<br>0x02 | NOW_SYSTEM_INFO_RSP_MSG |
| NOW_SYSTEM_SHUTDOWN_ID<br>0x03 | NOW_SYSTEM_SHUTDOWN_MSG |

<!-- TODO: Define NOW_SYSTEM_INFO_REQ_MSG, NOW_SYSTEM_INFO_RSP_MSG   -->

#### NOW_SYSTEM_SHUTDOWN_MSG

The NOW_SYSTEM_SHUTDOWN_MSG structure is used to request a system shutdown.NOW_SESSION_LOGOFF_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">timeout</td>
        </tr>
        <tr>
            <td colspan="32">message</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SYSTEM_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SYSTEM_SHUTDOWN_MSG_ID)

**msgFlags (2 bytes)**: The message flags.

| Flag | Meaning |
|------|---------|
| NOW_SHUTDOWN_FLAG_FORCE<br>0x0001 | Force shutdown |
| NOW_SHUTDOWN_FLAG_REBOOT<br>0x0002 | Reboot after shutdown |

**timeout (4 bytes)**: This system shutdown timeout, in seconds.

**message (variable)**: A NOW_STRING structure containing an optional shutdown message.

### Session Messages

#### NOW_SESSION_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value                           | Meaning              |
|---------------------------------|----------------------|
| NOW_SESSION_LOCK_MSG_ID<br>0x01 | NOW_SESSION_LOCK_MSG |
| NOW_SESSION_LOGOFF_MSG_ID<br>0x02 | NOW_SESSION_LOGOFF_MSG |
| NOW_SESSION_MESSAGE_BOX_MSG_REQ_ID<br>0x03 | NOW_SESSION_MESSAGE_BOX_MSG |
| NOW_SESSION_MESSAGE_BOX_RSP_MSG_ID<br>0x04 | NOW_SESSION_MESSAGE_RSP_MSG |

**msgFlags (2 bytes)**: The message flags.

#### NOW_SESSION_LOCK_MSG

The NOW_SESSION_LOCK_MSG is used to request locking the user session.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_LOCK_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

#### NOW_SESSION_LOGOFF_MSG

The NOW_SESSION_LOGOFF_MSG is used to request a user session logoff.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_LOGOFF_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

#### NOW_SESSION_MSGBOX_REQ_MSG

The NOW_SESSION_MSGBOX_REQ_MSG is used to show a message box in the user session, similar to what the [WTSSendMessage function](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtssendmessagew) does.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">requestId</td>
        </tr>
        <tr>
            <td colspan="32">style</td>
        </tr>
        <tr>
            <td colspan="32">timeout</td>
        </tr>
        <tr>
            <td colspan="32">text (variable)</td>
        </tr>
                <tr>
            <td colspan="32">title (variable, optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_MESSAGE_BOX_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                | Meaning                                 |
|-------------------------------------|-----------------------------------------|
| NOW_MSGBOX_FLAG_TITLE<br>0x00000001 | Set if `title` field is present |
| NOW_MSGBOX_FLAG_STYLE<br>0x00000002 | The style field contains a non-default value |
| NOW_MSGBOX_FLAG_TIMEOUT<br>0x00000004 | The timeout field contains a non-default value |
| NOW_MSGBOX_FLAG_RESPONSE<br>0x00000008 | A response message is expected (don't fire and forget) |

**requestId (4 bytes)**: the message request id, sent back in the response.

**style (4 bytes)**: The message box style, ignored if NOW_MSGBOX_FLAG_STYLE is not set. MBOK is the default, refer to the [MessageBox function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-messagebox) for all possible styles. This field may be ignored on platforms other than Windows.

**timeout (4 bytes)**: The timeout, in seconds, that the message box dialog should wait for the user response. This value is ignored if NOW_MSGBOX_FLAG_TIMEOUT is not set.

**text (variable)**: The message box text.

**title (variable, optional)**: The message box title.

#### NOW_SESSION_MSGBOX_RSP_MSG

The NOW_SESSION_MSGBOX_RSP_MSG is a message sent in response to NOW_SESSION_MSGBOX_REQ_MSG if the NOW_MSGBOX_FLAG_RESPONSE has been set, and contains the result from the message box dialog.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">requestId</td>
        </tr>
        <tr>
            <td colspan="32">status</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_MESSAGE_RSP_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**requestId (4 bytes)**: The corresponding message box request id.

**status (4 bytes)**: `NOW_STATUS` structure containing message box request status. `status.code`
contains message box response code, defined as following:

| Value        | Meaning |
|--------------|---------|
| IDABORT<br>3 | Abort   |
| IDCANCEL<br>2 | Cancel   |
| IDCONTINUE<br>11 | Continue   |
| IDIGNORE<br>5 | Ignore   |
| IDNO<br>7 | No   |
| IDOK<br>1 | OK   |
| IDRETRY<br>4 | Retry   |
| IDTRYAGAIN<br>10 | Try Again   |
| IDYES<br>6 | Yes   |
| IDTIMEOUT<br>32000 | Timeout   |

### Execution Messages

#### NOW_EXEC_MSG

The NOW_EXEC_MSG message is used to execute remote commands or scripts.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value | Meaning |
|-------|---------|
| NOW_EXEC_CAPSET_MSG_ID<br>0x00 | NOW_EXEC_CAPSET_MSG |
| NOW_EXEC_ABORT_MSG_ID<br>0x01 | NOW_EXEC_ABORT_MSG |
| NOW_EXEC_CANCEL_REQ_MSG_ID<br>0x02 | NOW_EXEC_CANCEL_REQ_MSG |
| NOW_EXEC_CANCEL_RSP_MSG_ID<br>0x03 | NOW_EXEC_CANCEL_RSP_MSG |
| NOW_EXEC_RESULT_MSG_ID<br>0x04 | NOW_EXEC_RESULT_MSG |
| NOW_EXEC_DATA_MSG_ID<br>0x05 | NOW_EXEC_DATA_MSG |
| NOW_EXEC_RUN_MSG_ID<br>0x10 | NOW_EXEC_RUN_MSG |
| NOW_EXEC_PROCESS_MSG_ID<br>0x11 | NOW_EXEC_PROCESS_MSG |
| NOW_EXEC_SHELL_MSG_ID<br>0x12 | NOW_EXEC_SHELL_MSG |
| NOW_EXEC_BATCH_MSG_ID<br>0x13 | NOW_EXEC_BATCH_MSG |
| NOW_EXEC_WINPS_MSG_ID<br>0x14 | NOW_EXEC_WINPS_MSG |
| NOW_EXEC_PWSH_MSG_ID<br>0x15 | NOW_EXEC_PWSH_MSG |
| NOW_EXEC_HEARTBEAT_MSG_ID<br>0x20 | NOW_EXEC_HEARTBEAT_MSG |

**msgFlags (2 bytes)**: The message flags.

#### NOW_EXEC_ABORT_MSG

The NOW_EXEC_ABORT_MSG message is used to abort a remote execution immediately due to an unrecoverable error. This message can be sent at any time without an explicit response message. The session is considered aborted as soon as this message is sent.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">exitCode</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_ABORT_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**exitCode (4 bytes)**: Exit code for application abort (Ignored if not supported by OS).

#### NOW_EXEC_CANCEL_REQ_MSG

The NOW_EXEC_CANCEL_REQ_MSG message is used to cancel a remote execution session.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_CANCEL_REQ_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

#### NOW_EXEC_CANCEL_RSP_MSG

The NOW_EXEC_CANCEL_RSP_MSG message is used to respond to a remote execution cancel request.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">status</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_CANCEL_RSP_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**status (4 bytes)**: `NOW_STATUS` structure containing message box request status. `status.code`
should be set to 0.

#### NOW_EXEC_RESULT_MSG

The NOW_EXEC_RESULT_MSG message is used to return the result of an execution request. The session is considered terminated as soon as this message is sent.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">status (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_RESULT_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**status (variable)**: `NOW_STATUS` structure containing session execution result. `status.code` contains process
exit code on success.

#### NOW_EXEC_DATA_MSG

The NOW_EXEC_DATA_MSG message is used to send input/output data as part of a remote execution.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">data (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_DATA_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                   | Meaning                         |
|----------------------------------------|---------------------------------|
| NOW_EXEC_FLAG_DATA_FIRST<br>0x00000001 | This is the first data message. |
| NOW_EXEC_FLAG_DATA_LAST<br>0x00000002 | This is the last data message, the command completed execution. |
| NOW_EXEC_FLAG_DATA_STDIN<br>0x00000004 | The data is from the standard input. |
| NOW_EXEC_FLAG_DATA_STDOUT<br>0x00000008 | The data is from the standard output. |
| NOW_EXEC_FLAG_DATA_STDERR<br>0x00000010 | The data is from the standard error. |

Message should contain exactly one of `NOW_EXEC_FLAG_DATA_STDIN`, `NOW_EXEC_FLAG_DATA_STDOUT` or `NOW_EXEC_FLAG_DATA_STDERR` flags set.

First stream message should always contain `NOW_EXEC_FLAG_DATA_FIRST`
flag.

`NOW_EXEC_FLAG_DATA_LAST` should indicate EOF for a stream, all consecutive messages for the given stream will be ignored by either party (client or sever).


**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**data (variable)**: The input/output data represented as `NOW_VARBUF`

#### NOW_EXEC_RUN_MSG

The NOW_EXEC_RUN_MSG message is used to send a run request. This request type maps to starting a program by using the “Run” menu on operating systems (the Start Menu on Windows, the Dock on macOS etc.). The execution of programs started with NOW_EXEC_RUN_MSG is not followed and does not send back the output.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_RUN_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

#### NOW_EXEC_PROCESS_MSG

The NOW_EXEC_PROCESS_MSG message is used to send a Windows [CreateProcess()](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw) request.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">filename (variable)</td>
        </tr>
        <tr>
            <td colspan="32">parameters (variable, optional)</td>
        </tr>
        <tr>
            <td colspan="32">directory (variable, optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_PROCESS_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                   | Meaning                   |
|----------------------------------------|---------------------------|
| NOW_EXEC_FLAG_PROCESS_PARAMETERS_SET<br>0x00000001 | Message contains `parameters` field if set |
| NOW_EXEC_FLAG_PROCESS_DIRECTORY_SET<br>0x00000002 | Message contains `directory` field if set |


**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**filename (variable)**: A NOW_VARSTR structure containing the file name. Corresponds to the lpApplicationName parameter.

**parameters (variable, optional)**: A NOW_VARSTR structure containing the command parameters. Corresponds to the lpCommandLine parameter.

**directory (variable, optional)**: A NOW_VARSTR structure containing the command working directory. Corresponds to the lpCurrentDirectory parameter.

#### NOW_EXEC_SHELL_MSG

The NOW_EXEC_SHELL_MSG message is used to execute a remote shell script.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">shell (variable, optional)</td>
        </tr>
        <tr>
            <td colspan="32">directory (variable, optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_SHELL_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                   | Meaning                   |
|----------------------------------------|---------------------------|
| NOW_EXEC_FLAG_SHELL_SHELL_SET<br>0x00000001 | Message contains `shell` field if set |
| NOW_EXEC_FLAG_SHELL_DIRECTORY_SET<br>0x00000002 | Message contains `directory` field if set |

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the script file contents to execute.

**shell (variable, optional)**: A NOW_VARSTR structure containing the shell to use for execution. If no shell is specified, the default system shell (/bin/sh) will be used.

**directory (variable, optional)**: A NOW_VARSTR structure containing the command working directory.

#### NOW_EXEC_BATCH_MSG

The NOW_EXEC_BATCH_MSG message is used to execute a remote batch script.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">directory (variable, optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_BATCH_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                   | Meaning                   |
|----------------------------------------|---------------------------|
| NOW_EXEC_FLAG_BATCH_DIRECTORY_SET<br>0x00000002 | Message contains `directory` field if set |

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the script file contents to execute.

**directory (variable, optional)**: A NOW_VARSTR structure containing the command working directory.

#### NOW_EXEC_WINPS_MSG

The NOW_EXEC_WINPS_MSG message is used to execute a remote Windows PowerShell (powershell.exe) command.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">executionPolicy (variable, optional)</td>
        </tr>
        <tr>
            <td colspan="32">configurationName (variable, optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_WINPS_MSG_ID).

**msgFlags (2 bytes)**: The message flags, specifying the PowerShell command-line arguments.

| Flag                                   | Meaning                   |
|----------------------------------------|---------------------------|
| NOW_EXEC_FLAG_PS_NO_LOGO<br>0x00000001 | PowerShell -NoLogo option |
| NOW_EXEC_FLAG_PS_NO_EXIT<br>0x00000002 | PowerShell -NoExit option |
| NOW_EXEC_FLAG_PS_STA<br>0x00000004 | PowerShell -Sta option |
| NOW_EXEC_FLAG_PS_MTA<br>0x00000008 | PowerShell -Mta option |
| NOW_EXEC_FLAG_PS_NO_PROFILE<br>0x00000010 | PowerShell -NoProfile option |
| NOW_EXEC_FLAG_PS_NON_INTERACTIVE<br>0x00000020 | PowerShell -NonInteractive option |
| NOW_EXEC_FLAG_PS_EXECUTION_POLICY<br>0x00000040 | `executionPolicy` field is set and specifies the PowerShell -ExecutionPolicy parameter |
| NOW_EXEC_FLAG_PS_CONFIGURATION_NAME<br>0x00000080 | `configurationName` field is set and specifies the PowerShell -ConfigurationName parameter |

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

**executionPolicy (variable, optional)**: A NOW_VARSTR structure containing the execution policy (-ExecutionPolicy) parameter value.

**configurationName (variable, optional)**: A NOW_VARSTR structure containing the configuration name (-ConfigurationName) parameter value.

#### NOW_EXEC_PWSH_MSG

The NOW_EXEC_PWSH_MSG message is used to execute a remote PowerShell 7 (pwsh) command.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">executionPolicy (variable, optional)</td>
        </tr>
        <tr>
            <td colspan="32">configurationName (variable, optional)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_PWSH_MSG_ID).

**msgFlags (2 bytes)**: The message flags, specifying the PowerShell command-line arguments, same as with NOW_EXEC_WINPS_MSG.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

**executionPolicy (variable, optional)**: A NOW_VARSTR structure, same as with NOW_EXEC_WINPS_MSG.

**configurationName (variable, optional)**: A NOW_VARSTR structure, same as with NOW_EXEC_WINPS_MSG.

#### NOW_EXEC_HEARTBEAT_MSG

The NOW_EXEC_HEARTBEAT_MSG message is sent immediately after execution session has started and then is sent
periodically on intervals specified during negotiation phase.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_HEARTBEAT_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

### Version History
- 1.0
    - Initial protocol version
