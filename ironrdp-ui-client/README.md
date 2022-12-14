### GUI client
1. An experimental GUI is part of the cli tool and can be enabled by using the gui feature flag.
2. Sample command to run the ui client:
    ```
    cargo run --bin ironrdp-cli --features=gui -- -u SimpleUsername -p SimplePassword! --avc444 --thin-client --small-cache --capabilities 0xf 192.168.1.100:3389
    ```
3. If the GUI has artifacts it can be dumped to a file using the gfx_dump_file parameter. Later the replay-client binary can be used to debug and fix any issues 
 in the renderer.