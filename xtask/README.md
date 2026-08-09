# IronRDP project automation

Free-form automation following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification.

Validate a pull request's squash-commit message from a GitHub event:

```shell
cargo xtask pr check-message --event-file path/to/event.json
```
