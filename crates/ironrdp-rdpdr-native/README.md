# IronRDP RDPDR native backends

Native backend building blocks for the IronRDP RDPDR static channel.

- On macOS and Linux, the crate exports the existing `nix::backend` filesystem
  backend.
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

