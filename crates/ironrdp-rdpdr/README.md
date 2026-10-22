# IronRDP RDPDR

Implements the RDPDR static virtual channel as described in
[\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension][spec]

[spec]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

This crate is part of the [IronRDP] project.

## Virtual Printers

`Rdpdr::with_printer` announces a PostScript virtual printer using
`MS Publisher Imagesetter` as the default server-side driver. This matches
FreeRDP's default CUPS printer driver for PostScript redirection and keeps the
client format-agnostic: printer IRPs deliver the raw job bytes to the backend.
Printer devices are advertised after the server sends `RDPDR_USER_LOGGEDON_PDU`;
pre-logon announces remain reserved for special devices such as smart cards.

Use `Rdpdr::with_printer_driver` when the target host needs a different
installed printer driver. The selected driver controls the document format the
server writes to the redirected printer, so consumers are responsible for any
PostScript-to-PDF or other conversion step before presenting the job to a user.

`Rdpdr::with_redirected_printer` announces an `RdpdrPrinter` with its own flags and cached configuration.
Call it once per printer: each one gets its own `PRN<n>` port name, and only the first printer marked default keeps the default flag, as MS-RDPEPC 2.2.2.1 requires.

## Printer Cache Data

Windows servers send `PAKID_PRN_CACHE_DATA` when a user changes printing preferences, installs a queue on the redirected port, or renames or deletes a redirected queue (MS-RDPEPC 3.2.5.1.3 to 3.2.5.1.6).
`Rdpdr::with_printer_cache_store` saves these through an embedder-supplied `RdpdrPrinterCacheStore` and sends the saved data back as `CachedPrinterConfigData` in the next announcement.
A driver saved from an add event replaces the announced driver unless the printer was created with `with_explicit_driver(true)`.
Cached configuration is driver-specific (MS-RDPEPC 3.1.1.1), so each entry records the driver it was produced for, and it is not sent with another driver.
The channel does no I/O itself: `InMemoryRdpdrPrinterCacheStore` keeps entries for the process lifetime, and `ironrdp-rdpdr-native` provides a file-backed store.
Store errors, oversized blobs (over 256 KiB), and messages for printers this client did not announce are logged and ignored; a malformed message fails the channel, as other malformed RDPDR packets do (MS-RDPEFS 3.1.5.2).
Entries are keyed by the local queue name and record the name the server gave the printer, from a rename event or a queue installed on the redirected port, so the next announcement uses that name (MS-RDPEPC 3.2.5.1.3, 3.2.5.1.6).
The redirected printer is renamed, not the local Windows queue: a server must not rename printers on the client machine.

`PAKID_PRN_USING_XPS` is decoded and ignored, because this crate never sets `RDPDR_PRINTER_ANNOUNCE_FLAG_XPSFORMAT`.

[IronRDP]: https://github.com/Devolutions/IronRDP
