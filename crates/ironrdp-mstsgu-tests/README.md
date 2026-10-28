# ironrdp-mstsgu-tests

Process-isolated Mstsgu authentication and proxy checks, launched by `ironrdp-testsuite-extra`.
The suite builds this unpublished binary with explicit features, sets proxy variables on each child, and checks its exit status.
The `proxy-connect <gateway-endpoint>` scenario opens and closes both HTTP transport legs against the suite's loopback gateway.
The `smartcard-auth` scenario checks rejected credentials and malformed Negotiate challenges without contacting a KDC.
The `smartcard-disabled` scenario checks the explicit error when smartcard support is absent.
The `rpch-gateway` and `rpch-malformed` scenarios exercise the public gateway stream against a loopback TLS proxy, including RPC integrity, messaging, channel recycling, and malformed authenticated replies.
The ignored `public_gateway_stream_preserves_data_across_in_channel_rollover` test transfers more than 1 GiB through `rpch-in-recycle` to check signed data across the IN channel limit.
