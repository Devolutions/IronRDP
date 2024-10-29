# IronRDP Blocking

Blocking I/O abstraction wrapping the IronRDP state machines conveniently.

This crate is a higher level abstraction for IronRDP state machines using blocking I/O instead of
asynchronous I/O. This results in a simpler API with fewer dependencies that may be used
instead of `ironrdp-async` when concurrency is not a requirement.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
