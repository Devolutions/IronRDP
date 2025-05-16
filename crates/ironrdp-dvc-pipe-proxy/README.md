# IronRDP DVC pipe proxy

Generic DVC handler which makes IronRDP connect to specific DVC channel and create a named pipe
server, which will be used for proxying DVC messages to/from user-defined DVC logic
implemented as named pipe clients (either in the same process or in a different process).