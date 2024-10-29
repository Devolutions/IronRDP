# GUI client

1. An experimental GUI based of glutin and glow library.
2. Sample command to run the ui client:
    ```
    cargo run --bin ironrdp-gui-client -- -u SimpleUsername -p SimplePassword! --avc444 --thin-client --small-cache --capabilities 0xf 192.168.1.100:3389
    ```
3. If the GUI has artifacts it can be dumped to a file using the gfx_dump_file parameter. Later the ironrdp-replay-client binary can be used to debug and fix any issues 
 in the renderer.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
