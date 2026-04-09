# IronRDP CLIPRDR

Implementation of clipboard static virtual channel (`CLIPRDR`) described in [MS-RDPECLIP].

This library includes:
- Clipboard SVC PDUs parsing
- Clipboard SVC processing
- Clipboard backend API types for implementing OS-specific clipboard logic
- File transfer support via clipboard redirection

For concrete native clipboard backend implementations, see `ironrdp-cliprdr-native` crate.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPECLIP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip

## Features

- **Text clipboard transfer**: Copy/paste text between local and remote clipboards
- **File transfer**: Copy/paste files using delayed rendering per MS-RDPECLIP spec
- **Format negotiation**: Automatic capability negotiation with the server
- **Delayed rendering**: Efficient clipboard synchronization with minimal bandwidth usage

## Usage

### Basic Clipboard Operations

```rust
use ironrdp_cliprdr::{CliprdrClient, backend::CliprdrBackend};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId};

// Initialize clipboard client with your backend implementation
let mut cliprdr = CliprdrClient::new(Box::new(my_backend));

// Initiate a text copy operation
let formats = vec![
    ClipboardFormat::new(ClipboardFormatId::new(13)) // CF_UNICODETEXT
];
let messages = cliprdr.initiate_copy(&formats)?;
// Send messages on the CLIPRDR virtual channel

// Initiate a paste operation (after receiving FormatList from remote)
let format_id = ClipboardFormatId::new(13); // CF_UNICODETEXT
let messages = cliprdr.initiate_paste(format_id)?;
// Send messages on the CLIPRDR virtual channel
```

### File Transfer

File transfer follows the delayed rendering pattern specified in MS-RDPECLIP section 1.3.1.4:

1. **Copying files locally** - Advertise file formats without sending data
2. **Pasting files** - Request file list when user initiates paste
3. **Downloading files** - Request individual file contents

#### Copying Files to Remote (Upload)

```rust
use ironrdp_cliprdr::pdu::{FileDescriptor, ClipboardFileAttributes};

// Create file descriptors for files to copy (FileDescriptor is #[non_exhaustive])
let files = vec![
    FileDescriptor::new("document.pdf")
        .with_attributes(ClipboardFileAttributes::ARCHIVE)
        .with_last_write_time(132489216000000000) // FILETIME format
        .with_file_size(1024),
    FileDescriptor::new("spreadsheet.xlsx")
        .with_attributes(ClipboardFileAttributes::ARCHIVE)
        .with_last_write_time(132489216000000000)
        .with_file_size(2048),
];

// Initiate file copy - sends FormatList with FileGroupDescriptorW
let messages = cliprdr.initiate_file_copy(files)?;
// Send messages on the CLIPRDR virtual channel

// When remote requests the file list (via FormatDataRequest),
// the state machine automatically responds with the stored file list
```

#### Pasting Files from Remote (Download)

```rust
// When FormatList arrives containing FileGroupDescriptorW format,
// the backend's on_remote_copy() is called with available formats.
// The file list format ID is stored automatically.

// User initiates paste - request the file list
let file_list_format_id = /* format ID from FormatList */;
let messages = cliprdr.initiate_paste(file_list_format_id)?;
// Send messages on the CLIPRDR virtual channel

// When FormatDataResponse arrives with file list,
// backend's on_remote_file_list() is called automatically:
fn on_remote_file_list(
    &mut self,
    files: &[FileDescriptor],
    clip_data_id: Option<u32>,
) {
    // Receive file metadata and the automatic lock ID
    self.current_lock_id = clip_data_id;

    for (index, file) in files.iter().enumerate() {
        println!("File {}: {} ({} bytes)",
            index,
            file.name,
            file.file_size.unwrap_or(0));
    }

    // Example: Request first file's size (using the lock)
    let size_request = FileContentsRequest {
        stream_id: 1,
        index: 0, // First file in the list
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: clip_data_id,
    };
    // Call cliprdr.request_file_contents(size_request)
    // Response arrives via on_file_contents_response()
}
```

#### Requesting File Contents

```rust
use ironrdp_cliprdr::pdu::{FileContentsRequest, FileContentsFlags};

// Request file size first
let size_request = FileContentsRequest {
    stream_id: 1, // Unique ID for this transfer
    index: 0,     // File index (0-based i32, must be non-negative per MS-RDPECLIP 2.2.5.3)
    flags: FileContentsFlags::SIZE,
    position: 0,
    requested_size: 8, // SIZE requests must request 8 bytes per MS-RDPECLIP 2.2.5.3
    data_id: None,
};
let messages = cliprdr.request_file_contents(size_request)?;

// Then request file data in chunks
let data_request = FileContentsRequest {
    stream_id: 1,
    index: 0,
    flags: FileContentsFlags::RANGE,
    position: 0,      // Byte offset
    requested_size: 4096, // Chunk size
    data_id: None,
};
let messages = cliprdr.request_file_contents(data_request)?;
```

### Clipboard Locking

Per [MS-RDPECLIP] section 2.2.4, clipboard locking prevents clipboard data from being overwritten during file transfer operations. IronRDP acquires locks automatically when `FileGroupDescriptorW` is detected in a FormatList, similar to FreeRDP's behavior.

#### Why Locking is Needed

File transfers can take significant time. Without locking, if the remote clipboard changes during transfer:
- File list metadata becomes stale
- File contents requests may fail with mismatched data
- Download operations fail mid-transfer

Locking creates a snapshot of the clipboard state that persists even if the remote clipboard content changes.

#### Automatic Lock Behavior

When a FormatList containing `FileGroupDescriptorW` is received and `CAN_LOCK_CLIPDATA` was negotiated, the cliprdr processor automatically:

1. Sends a Lock Clipboard Data PDU with a new `clipDataId`
2. Passes the `clipDataId` to the backend via `on_remote_file_list(files, clip_data_id)`
3. Manages the lock lifecycle (expiry, cleanup, Unlock PDUs) internally

Backends receive the `clipDataId` and can pass it to `request_file_contents()` via the `data_id` field. No explicit lock/unlock calls are needed.

#### Driving Timeouts

Callers must drive [`Cliprdr::drive_timeouts()`] from a periodic timer in their event loop (e.g., every 5 seconds). This method processes:
- **Expired locks**: Sends Unlock PDUs for locks past their inactivity timeout or max lifetime
- **Stale requests**: Cleans up file contents requests older than the transfer timeout
- **Abandoned uploads**: Prunes locked file list snapshots with no recent activity

```rust
// In your event loop's timer arm (e.g., tokio::time::interval or gloo_timers::IntervalStream)
let messages = cliprdr.drive_timeouts()?;
for msg in messages {
    send_on_channel(msg)?;
}
```

Locks are cleaned up based on:
- **Inactivity timeout** (60s default): No FileContentsRequest activity after lock expires
- **Maximum lifetime** (2h default): Force cleanup regardless of activity

#### Customizing Lock Timeouts

For specialized use cases (slow networks, large files), customize timeout policy:

```rust
use std::time::Duration;

let cliprdr = Cliprdr::with_lock_timeouts(
    Box::new(my_backend),
    Duration::from_secs(120),  // Inactivity: 2 minutes
    Duration::from_secs(3600), // Max: 1 hour
);
```

#### Concurrent Downloads

Multiple file download operations can run simultaneously using the same lock. The `clipDataId` from `on_remote_file_list()` should be passed to each `request_file_contents()` call. The lock remains active as long as file content requests keep arriving within the inactivity timeout.

#### Lock Expiration on Clipboard Change

When a new `FormatList` arrives (indicating clipboard content changed), all active locks transition to **Expired state** and enter a grace period. Expired locks:
- Remain functional and can continue servicing ongoing file transfers
- Are cleaned up after the inactivity timeout (60s default) if no FileContentsRequest activity occurs
- Are force-cleaned after the maximum lifetime (2h default) regardless of activity

This grace period approach ensures:
- Long-running downloads can complete even if the remote clipboard changes
- Resources are eventually released without blocking active transfers
- Network stalls and reconnections don't cause transfer failures

##### Grace Period Timeout Values

The lock cleanup mechanism uses two timeout values to balance transfer reliability with resource management:

**Inactivity Timeout (default: 60 seconds)**
- Locks are cleaned up after 60 seconds without `FileContentsRequest` activity
- Each `FileContentsRequest` for the lock resets the inactivity timer
- This handles abandoned downloads while allowing active transfers to continue
- Configurable via `Cliprdr::with_lock_timeouts(backend, inactivity_duration, max_lifetime_duration)`

**Maximum Lifetime (default: 2 hours)**
- Locks are force-cleaned after 2 hours regardless of activity
- This prevents indefinite resource accumulation from slow or stalled transfers
- Measured from when the lock was created (not from when it transitioned to Expired)
- Configurable via `Cliprdr::with_lock_timeouts(backend, inactivity_duration, max_lifetime_duration)`

**Activity Tracking:**
- Only `FileContentsRequest` calls update the `last_used_at` timestamp
- Lock/Unlock PDUs do not reset the timer
- A lock survives clipboard changes as long as requests keep arriving within 60s

**Example: Customizing Timeouts for Slow Networks**

```rust
use std::time::Duration;

let cliprdr = Cliprdr::with_lock_timeouts(
    Box::new(MyBackend),
    Duration::from_secs(300),   // 5 minute inactivity timeout
    Duration::from_secs(14400), // 4 hour maximum lifetime
);
```

**When to Adjust Timeouts:**
- **Slow networks**: Increase inactivity timeout to prevent cleanup during normal transfer delays
- **Large files**: Increase both timeouts to accommodate extended download times
- **Resource-constrained systems**: Decrease timeouts to free resources more aggressively
- **Default values work well** for typical network conditions and file sizes

#### Backend Responsibilities

Backends receive the lock ID and use it for file content requests:

```rust
impl CliprdrBackend for MyBackend {
    fn on_remote_file_list(
        &mut self,
        files: &[FileDescriptor],
        clip_data_id: Option<u32>,
    ) {
        // Store the lock ID for use in file content requests
        self.current_lock_id = clip_data_id;
        // Display files to user or start downloads
        self.start_file_downloads(files);
    }
}
```

No explicit lock/unlock calls are needed - the cliprdr processor handles the full lifecycle.

#### clipDataId in File Contents Requests

When making `FileContentsRequest` calls, the `data_id` field is automatically populated with the most recent lock ID if not already set:

```rust
let request = FileContentsRequest {
    stream_id: 1,
    index: 0,
    flags: FileContentsFlags::RANGE,
    position: 0,
    requested_size: 4096,
    data_id: None, // Automatically uses current_lock_id
};
cliprdr.request_file_contents(request)?;
```

Backends can override by setting `data_id` explicitly if managing multiple concurrent locks.

### Implementing a Clipboard Backend

```rust
use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::*;

struct MyClipboardBackend {
    // Your OS-specific clipboard state
}

impl CliprdrBackend for MyClipboardBackend {
    fn temporary_directory(&self) -> &str {
        "/tmp/clipboard"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
    }

    fn on_ready(&mut self) {
        // Clipboard channel is ready
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // Remote has new clipboard content with these formats
    }

    fn on_remote_file_list(
        &mut self,
        files: &[FileDescriptor],
        clip_data_id: Option<u32>,
    ) {
        // Remote has files available for download.
        // clip_data_id is the lock acquired automatically when
        // FileGroupDescriptorW was detected. Pass it to request_file_contents().
        self.current_lock_id = clip_data_id;
        for (index, file) in files.iter().enumerate() {
            println!("File {}: {} ({} bytes)",
                index, file.name, file.file_size.unwrap_or(0));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        // Remote is requesting clipboard data
        // Call cliprdr.submit_format_data() to respond
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        // Remote is requesting file contents
        // Call cliprdr.submit_file_contents() to respond
    }

    // ... implement other required methods
}
```

## File Name Validation

Per MS-RDPECLIP section 2.2.5.2.3.1, file names are automatically validated:
- Maximum length: 259 characters (Unicode)
- File names must not be empty
- No absolute paths when `CB_FILECLIP_NO_FILE_PATHS` is set (relative paths like `subfolder/file.txt` are allowed)

Invalid file descriptors are logged and skipped during `initiate_file_copy()`.

**Note:** IronRDP always sets `CB_FILECLIP_NO_FILE_PATHS` to use stream-based file transfer. Backends should provide relative filenames (e.g., `document.pdf` or `reports/summary.txt`) rather than absolute paths (e.g., `/home/user/document.pdf` or `C:\Users\user\document.pdf`).

## Delayed Rendering

The implementation follows MS-RDPECLIP section 1.3.1.4 "Delayed Rendering":

1. **Copy phase**: Only format IDs are sent, not actual data
2. **Paste phase**: Data is requested only when user initiates paste
3. **File transfers**: File list is requested on paste, file contents on demand

This minimizes network bandwidth and ensures clipboard synchronization efficiency.

## References

- [MS-RDPECLIP]: Remote Desktop Protocol: Clipboard Virtual Channel Extension
- [MS-RDPBCGR]: Remote Desktop Protocol: Basic Connectivity and Graphics Remoting
