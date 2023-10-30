# IronRDP Error

Generic error type for IronRDP crates.

The provided API is `no_std`-compatible, but the quality of the error reporting will decrease
when `std` feature is disabled, and even more so when `alloc` feature is disabled too:

- When `std` is enabled, [`ErrorReport`] is able to walk down all the source errors.
- When `alloc` is enabled, [`ErrorReport`] will only show the first source without going all the way down.
- When no feature is enabled, source errors are discarded because we can’t store them on the heap.
