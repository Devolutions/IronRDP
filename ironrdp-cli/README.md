# IronRDP client

A command-line RDP client, which performs connection to an RDP server and decodes RFX graphical updates.
If IronRDP client encounters an error, then will return `error` exit code and print what caused
an error.

## Prerequisites

Before connection to a Windows 10 RDP server please enable RFX feature:

1. Run  `gpedit.msc`.

2. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/RemoteFX for Windows Server 2008 R2/Configure RemoteFX`

3. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Enable RemoteFX encoding for RemoteFX clients designed for Windows Server 2008 R2 SP1`

4. Enable `Computer Configuration/Administrative Templates/Windows Components/Remote Desktop Services/Remote Desktop Session Host/Remote Session Environment/Limit maximum color depth`

5. Reboot.

## Command-line Interface

```
USAGE:
    ironrdp_client [OPTIONS] <ADDR> --password <PASSWORD> --security-protocol <SECURITY_PROTOCOL>... --username <USERNAME>

FLAGS:
    -h, --help       Prints help information
    -v, --version    Prints version information

OPTIONS:
        --dig-product-id <DIG_PRODUCT_ID>
            Contains a value that uniquely identifies the client [default: ]

    -d, --domain <DOMAIN>                                                    An optional target RDP server domain name
        --ime-file-name <IME_FILENAME>
            The input method editor (IME) file name associated with the active input locale [default: ]

        --keyboard-functional-keys-count <KEYBOARD_FUNCTIONAL_KEYS_COUNT>
            The number of function keys on the keyboard [default: 12]

        --keyboard-subtype <KEYBOARD_SUBTYPE>
            The keyboard subtype (an original equipment manufacturer-dependent value) [default: 0]

        --keyboard-type <KEYBOARD_TYPE>
            The keyboard type [default: ibm_enhanced]  [possible values: ibm_pc_xt, olivetti_ico, ibm_pc_at,
            ibm_enhanced, nokia1050, nokia9140, japanese]
        --log-file <LOG_FILE>
            A file with IronRDP client logs [default: ironrdp_client.log]

    -p, --password <PASSWORD>                                                A target RDP server user password
        --security-protocol <SECURITY_PROTOCOL>...
            Specify the security protocols to use [default: hybrid_ex]  [possible values: ssl, hybrid, hybrid_ex]

    -u, --username <USERNAME>                                                A target RDP server user name

ARGS:
    <ADDR>    An address on which the client will connect. Format: <ip>:<port>
```

It worth to notice that the client takes mandatory arguments as
 - `<ADDR>` as first argument;
 - `--username` or `-u`;
 - `--password` or `-p`.

## Sample Usage

1. Run the RDP server (Windows RDP server, FreeRDP server, etc.);
2. Run the IronRDP client and specify the RDP server address, username and password:
    ```
   cargo run 192.168.1.100:3389 -u SimpleUsername -p SimplePassword!
    ```
3. After the RDP Connection Sequence the client will start receive RFX updates 
and save to the internal buffer.
In case of error, the client will print (for example) `RDP failed because of negotiation error: ...`.
Additional logs are available in `<LOG_FILE>` (`ironrdp_client.log` by default).

### GUI client
1. An experimental GUI is part of the cli tool and can be enabled by using the gui feature flag.
2. Sample command to run the ui client:
    ```
    cargo run --bin ironrdp-cli --features=gui -- -u SimpleUsername -p SimplePassword! --avc444 --thin-client --small-cache --capabilities 0xf 192.168.1.100:3389
    ```
3. If the GUI has artifacts it can be dumped to a file using the gfx_dump_file parameter. Later the replay-client binary can be used to debug and fix any issues 
 in the renderer.