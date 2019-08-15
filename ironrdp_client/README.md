# IronRDP client

A command-line RDP client, which performs connection to an RDP server up to the Active state.
The client collects the minimum information about the system (the screen width, height, DPI, etc.),
 and takes other info from the command-line arguments (all the arguments have a default value).
If the RDP Connection Sequence completes, the client returns a `SUCCESS` exit code, 
or an `error` exit code in case of failure.

## Command-line Interface

```
USAGE:
    ironrdp_client [OPTIONS] <ADDR> --password <PASSWORD> --security_protocol <SECURITY_PROTOCOL>... --username <USERNAME>

FLAGS:
    -h, --help       Prints help information
    -v, --version    Prints version information

OPTIONS:
        --dig_product_id <DIG_PRODUCT_ID>
            Contains a value that uniquely identifies the client [default: ]

    -d, --domain <DOMAIN>                                                    An optional target RDP server domain name
        --ime_file-name <IME_FILENAME>
            The input method editor (IME) file name associated with the active input locale [default: ]

        --keyboard_functional_keys_count <KEYBOARD_FUNCTIONAL_KEYS_COUNT>
            The number of function keys on the keyboard [default: 12]

        --keyboard_subtype <KEYBOARD_SUBTYPE>
            The keyboard subtype (an original equipment manufacturer-dependent value) [default: 0]

        --keyboard_type <KEYBOARD_TYPE>
            The keyboard type [default: ibm_enhanced]  [possible values: ibm_pc_xt, olivetti_ico, ibm_pc_at,
            ibm_enhanced, nokia1050, nokia9140, japanese]
    -l, --log_file <LOG_FILE>
            A file with IronRDP client logs [default: ironrdp_client.log]

    -p, --password <PASSWORD>                                                A target RDP server user password
    -s, --security_protocol <SECURITY_PROTOCOL>...
            Specify the security protocols to use [default: hybrid_ex]  [possible values: ssl, hybrid, hybrid_ex]

        --static_channel <STATIC_CHANNEL>...                                 Unique static channel name
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
   cargo run -- 192.168.1.100:3389 -u SimpleUsername -p SimplePassword!
    ```
3. After the RDP Connection Sequence finishes, 
the client will print `RDP connection sequence finished`
in case of success, and (for example) `RDP failed because of negotiation error: ...`
in case of error. Additional logs are available in `<LOG_FILE>`.
