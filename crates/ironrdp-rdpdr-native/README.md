# IronRDP RDPDR native backends

Native backend building blocks for the IronRDP RDPDR static channel.

- On macOS and Linux, the crate exports the existing `nix::backend` filesystem
  backend.
  Each drive serves one directory, its root: the shared directory given to `NixRdpdrBackend::new` or `NixRdpdrBackendFactory::new`, or a directory of its own given to `with_drive_root`.
  With an empty shared directory, which `Default` gives, only drives with their own root are served.
  The backend resolves every protocol path one component at a time below the opened root of the drive the request addresses, rejecting `.` and `..` components and never following symbolic links or crossing mount points.
  Without `openat2`, as on macOS and Linux before 5.6, only mounts of another filesystem are refused, not bind mounts within the same one.
  [Drive roots on macOS and Linux](#drive-roots-on-macos-and-linux) describes how roots are resolved and what holds between drives that reach the same files.
  A file ID is valid only through the drive it was opened through: through another drive it is answered like a file ID that was never opened, and the request changes nothing.
  Files are opened without blocking, so a FIFO in a root cannot stall the channel.
  Reads are limited to 1 MiB, and the open files of all drives together to a quarter of the soft descriptor limit, at most 1024, where each root that holds its directory open counts as one open file.
  Files open with the access the server requests, directory queries match Windows wildcards, and byte-range locks bind the server's handles to each other, across drives, but not to local processes.
  The `nix::printer` backend spools PostScript jobs in an exclusively created private directory (0700), using exclusive 0600 files.
  A worker thread submits each closed job to CUPS through `lp`, which gets 60 seconds to accept it, or saves it in the configured folder without replacing existing files.
  A job is limited to 128 MiB and at most 16 jobs are open at once; dropping the backend deletes unfinished spool files.
- On Windows, the crate contains the native, handle-relative filesystem foundation used for drive redirection.
  It validates every protocol path before resolving it below an opened volume root, and it rejects DOS device aliases and reparse-point traversal.
  Its static filesystem support includes create/open, close, flush, bounded offset I/O, file and volume information, metadata changes, security descriptors, alternate data streams, directory enumeration, locks, notifications, and deny-by-default device controls.
  Smartcard-only products are valid when the channel is configured with `ironrdp_rdpdr::Rdpdr::with_smartcard(0)`.
  The WinSCard path implements core and extended MS-RDPESC calls, preserves ANSI wire variants, and follows Windows buffer-probe rules.
  Printer redirection is described in [Windows printers](#windows-printers).
- `FileRdpdrPrinterCacheStore` persists MS-RDPEPC printer cache data as one file per printer in a directory the embedder picks, on every supported platform.

The Windows implementation is intentionally layered so protocol code remains
platform independent in `ironrdp-rdpdr`.

`WindowsRdpdrBackendFactory` configures zero or more `RedirectedDrive`s and optional printer discovery.
A redirected drive root is a volume root (`C:\`) or a directory on one, opened without following reparse points.
`WindowsRdpdrBackendFactory::with_logical_drives` registers the drive letters (device IDs `1` to `26`) and announces the selected letters mapped when each connection starts, without probing their volumes.
`LogicalDriveWatcher` reports volumes mounted and removed during a session, from `WM_DEVICECHANGE` broadcasts and a `GetLogicalDrives` poll for network drives; hosts forward its changes as dynamic drive announcements and removals.

## Drive roots on macOS and Linux

A root given with `with_drive_root` is resolved once, when the backend is configured, and the backend holds the directory it names open for its lifetime.
The drive serves that directory, not its path: it keeps serving it when the directory is moved or its path comes to name something else, and when the channel restarts.
Once a drive has a root of its own, the shared directory is resolved and held the same way.
A backend whose drives all serve the shared directory opens it by path for each request, and does not check it.
Roots that name the same directory each hold it open through their own path, so a drive is bound by what the mount on the path of its own root allows, such as a read-only bind mount.
Each root that holds its directory open counts as one open file against the limit on open files, and keeps its filesystem busy while the backend lives.
A root that is unusable because of a symbolic link or a `..` step on its path, as described below, still holds its directory open and counts.
The roots hold at most half of that limit, so at least half is left to open files: with a soft descriptor limit of 256, at most 32 roots hold their directory open.
A root given beyond that is still resolved, so the other roots are checked against its directory, but it holds nothing open and is unusable.

A root is unusable when its path:

- is relative or empty, or holds a `.` or `..` component;
- names no directory, or takes more than 40 symbolic links or 1024 components to resolve;
- follows a symbolic link in or below the directory of any root of the same backend, its own included;
- takes a `..` step, from the target of a symbolic link, in or below the directory of any root of the same backend, its own included;
- is given when the roots given before it already hold half the limit on open files.

In a directory it is served, and below it, a server decides what every entry is.
A path that only descends from there ends at or below that directory, whatever the server does.
A link read there, or a step back up from there, makes the end of the path depend on those entries, so it could name any directory the next time it is resolved.
Symbolic links that lie outside every root are followed, and so are the `..` components of their targets that are taken outside every root.
A drive with an unusable root is still configured and announced, but is served nothing: a create through it fails as it does when the shared directory cannot be opened.
The other drives are served, and a warning is logged when a root becomes unusable.
Roots are checked against each other in whichever order they are given.
A root given again for the same device replaces the earlier one, which then no longer counts among the roots held open.

### Drives that reach the same files

Drives may serve the same directory, or directories inside one another.
The limits on open files and byte-range locks apply to all drives together.
A lock or a pending delete on a file holds through every drive that reaches it, and a rename does not replace a file that is open through any drive.
A rename moves the open handles of every drive that serves the same directory to the new name, however the roots of those drives are spelled.
A file or directory that is open through a drive that serves another directory, another hard link of it included, cannot be renamed until those handles are closed: the rename fails with `STATUS_SHARING_VIOLATION`.
A path names a file only beneath the root of its drive, so such a handle could not take the new name.
A handle on the root of its own drive is the exception, because it names that root whatever the directory is called: the drive of an enclosing directory can rename the root of an inner drive while the inner drive holds only that root open.
Once a drive has a root of its own, the directories that led to a file when it was marked for deletion cannot be renamed until the delete is done or the mark is cleared: the rename fails with `STATUS_ACCESS_DENIED`, as for a directory that holds open files.
The file is deleted by a name it was marked through, which those directories are part of.
A file marked for deletion through several drives is deleted by the name marked last that still removes it.
Each drive with a root of its own keeps the name it marked last, and the drives that serve the shared directory keep one name between them, the one marked last through any of them.
When a name fails, as beneath a root on a read-only mount, the name marked before it through another root is tried.
With only a shared directory, the name marked last is the only one, whichever drive marked it.

Between drives that serve different directories, these limitations are known:

- An open file keeps a directory from being renamed only from the root of its own drive down, and a file marked for deletion only from the root of each drive it was marked through down.
  The drive of an enclosing directory can therefore rename a directory above the root of an inner drive while the inner drive has files open, and the inner drive then serves its directory at the new path.
- The drive of an enclosing directory can delete the root of an inner drive once it is empty, and nothing can then be created or opened below that root.
- Roots are resolved again each time a backend is built.
  Between two backends built from one configuration, the drive of an enclosing directory can rename directories so that the path of an inner root names another directory inside the enclosing root, and the later backend serves that directory through the inner drive.
  A symbolic link placed on that path makes the inner root unusable instead, so under the assumptions below the later backend never serves a directory outside the enclosing root through the inner drive.

### Guarantees and assumptions

These hold by construction and are tested:

- A drive with its own root serves the directory the root named when the backend was configured, for the lifetime of the backend.
- A root is unusable when the walk that resolved it, one component at a time from `/`, followed a symbolic link or took a `..` step in the directory of a root of the backend or in a directory it reached through one.
- A drive with an unusable root is served nothing, and the other drives are served.
- The roots hold at most half the limit on open files, so at least half of it is left to open files.

These are assumed, and nothing checks them:

- Local processes are trusted, because they can change what any path names.
- No bind mount, and on macOS no firmlink such as `/Users` for `/System/Volumes/Data/Users`, shows a directory inside a root at another path, where the walk does not reach a link or a `..` step through that root.
- No other backend with another configuration lets a server write to a directory that a root path leads through.
  A root path is trusted as local configuration wherever it lies outside the roots of its own backend.
- No server of another connection changes the directories on a root path while the root is resolved.

## Windows printers

`WindowsRdpdrBackendFactory::with_printer_redirection` takes a `WindowsPrinterRedirection`, which selects one of:

- `WindowsPrinterSelection::Default`: the current user's default queue (`with_default_printer(true)` is shorthand for this).
- `WindowsPrinterSelection::All`: every queue from `EnumPrintersW(PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS)`, as FreeRDP's Windows backend does.
- `WindowsPrinterSelection::Named`: the listed queues, in order; a queue that cannot be opened is skipped with a warning.

Queues are read again for every connection, up to 64 printers.
The Windows default queue is announced as the default printer unless `with_default_queue` names another one.
Each printer announces its local driver name, which is what mstsc and FreeRDP send.
`with_driver(queue, driver)` overrides the driver for one queue; the override also wins over a driver the server cached.
`with_cache_store` keeps server printer settings between connections.

### Missing drivers

Without Easy Print, a Windows host can redirect a printer only when it has a driver with the exact announced name (MS-RDPEPC <1>).
mstsc avoids the problem with Easy Print, and FreeRDP uses `MS Publisher Imagesetter` only for its CUPS backend.
A Windows 11 host that lacks the driver still accepts the device announcement: it logs event 1111, which the client never sees, and creates no queue, so the client cannot detect the problem and retry with another driver.
Pick a driver the host has, such as a vendor universal driver, with `with_driver`.
`MS Publisher Imagesetter` ships with Windows, but it renders PostScript, so it only suits PostScript-capable local printers.

### Datatype

Jobs are spooled with the `RAW` datatype on a worker thread, bounded to 128 MiB per job and 16 open jobs.
The server renders each job with its own copy of the announced driver and sends a PRN file (MS-RDPEPC 3.2.5.1.2), which is already in the printer's language.
`RAW` makes the print processor pass it to the port unchanged.
The queue's default datatype is not used: it can be `NT EMF 1.008` or `XPS_PASS`, and the print processor would then try to replay device-ready bytes as EMF or XPS.

### Easy Print (XPS)

The backend does not set `RDPDR_PRINTER_ANNOUNCE_FLAG_XPSFORMAT`.
Advertising it would require:

1. Two dynamic virtual channels from MS-RDPEXPS: `TSVCTKT` for the print ticket interface and `XPSRD` for the printer driver interface, about 40 messages in all.
   The Easy Print driver on the server calls them synchronously while applications print, to read capabilities and convert settings.
2. A client implementation of those calls on top of the local driver: `PTGetPrintCapabilities`, `PTConvertDevModeToPrintTicket`, `PTConvertPrintTicketToDevMode`, and `PTMergeAndValidatePrintTicket` from `prntvpt.dll`, plus `DeviceCapabilitiesW` and `DocumentPropertiesW`.
   The asynchronous printer and document properties messages would also need a local driver UI owned by the viewer window.
3. Handling `PAKID_PRN_USING_XPS` by switching the printer to XPS mode, so later writes are an XPS document instead of a PRN file.
4. Printing that XPS document locally: `XPS_PASS` jobs for XPSDrv and v4 queues, or `StartXpsPrintJob` / the XPS rasterization service for GDI drivers, which is how mstsc prints on queues without an XPS pipeline.

Item 1 belongs in a new portable crate next to `ironrdp-rdpdr`, item 2 and item 4 in this crate behind the Windows target, and item 3 in `ironrdp-rdpdr` and the printer worker.
The flag can only be advertised once all four are in place, because the server sends XPS data as soon as it accepts it.

