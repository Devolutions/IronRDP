# IronRDP RDPDR

Implements the RDPDR static virtual channel as described in
[\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension][spec]

[spec]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

This crate is part of the [IronRDP] project.

## Virtual Printers

`Rdpdr::with_printer` announces a PostScript virtual printer using
`MS Publisher Imagesetter` as the default server-side driver. This matches the
driver used by Guacamole-like RDP printer redirection flows and keeps the client
format-agnostic: printer IRPs deliver the raw job bytes to the backend.
Printer devices are advertised after the server sends `RDPDR_USER_LOGGEDON_PDU`;
pre-logon announces remain reserved for special devices such as smart cards.

Use `Rdpdr::with_printer_driver` when the target host needs a different
installed printer driver. The selected driver controls the document format the
server writes to the redirected printer, so consumers are responsible for any
PostScript-to-PDF or other conversion step before presenting the job to a user.

[IronRDP]: https://github.com/Devolutions/IronRDP
