# IronRDP

A collection of Rust crates providing an implementation of the Microsoft Remote Desktop Protocol, with a focus on security.

## Demonstration

https://user-images.githubusercontent.com/3809077/202049929-76f42471-aeb0-41da-9118-0dc6ea491bd2.mp4

## Video Codec Support

Supported codecs:

- Uncompressed raw bitmap
- Interleaved Run-Length Encoding (RLE) Bitmap Codec
- RDP 6.0 Bitmap Compression
- Microsoft RemoteFX (RFX)

## Examples

### [`ironrdp-client`](https://github.com/Devolutions/IronRDP/tree/master/crates/ironrdp-client)

A full-fledged RDP client based on IronRDP crates suite, and implemented using non-blocking, asynchronous I/O.

```shell
cargo run --bin ironrdp-client -- <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

### [`screenshot`](https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp/examples/screenshot.rs)

Example of utilizing IronRDP in a blocking, synchronous fashion.

This example showcases the use of IronRDP in a blocking manner. It
demonstrates how to create a basic RDP client with just a few hundred lines
of code by leveraging the IronRDP crates suite.

In this basic client implementation, the client establishes a connection
with the destination server, decodes incoming graphics updates, and saves the
resulting output as a BMP image file on the disk.

```shell
cargo run --example=screenshot -- --host <HOSTNAME> --username <USERNAME> --password <PASSWORD> --output out.bmp
```

### How to enable RemoteFX on server

Run the following PowerShell commands, and reboot.

```pwsh
Set-ItemProperty -Path 'HKLM:\Software\Policies\Microsoft\Windows NT\Terminal Services' -Name 'ColorDepth' -Type DWORD -Value 5
Set-ItemProperty -Path 'HKLM:\Software\Policies\Microsoft\Windows NT\Terminal Services' -Name 'fEnableVirtualizedGraphics' -Type DWORD -Value 1
```

Alternatively, you may change a few group policies using `gpedit.msc`:

1. Run `gpedit.msc`.

2. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/RemoteFX for Windows Server 2008 R2/Configure RemoteFX`

3. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Enable RemoteFX encoding for RemoteFX clients designed for Windows Server 2008 R2 SP1`

4. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Limit maximum color depth`

5. Reboot.

## Architecture

See the [ARCHITECTURE.md](https://github.com/Devolutions/IronRDP/blob/master/ARCHITECTURE.md) document.

## Getting help

- Report bugs in the [issue tracker](https://github.com/Devolutions/IronRDP/issues)
- Discuss the project on the [matrix room](https://matrix.to/#/#IronRDP:matrix.org)
