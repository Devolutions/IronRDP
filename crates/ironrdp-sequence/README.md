# IronRDP Sequence

Sans-I/O state-machine contract shared by RDP connect and accept sequences.

## Purpose

`ironrdp-connector` and `ironrdp-acceptor` each drive an RDP connection through a chain of phases (negotiation, channel connection, license exchange, activation, finalization).
Both crates implement the same contract for that chain; this crate is where it now lives, so it does not need to be defined a second time.
`ironrdp-acceptor` still depends on `ironrdp-connector` directly today rather than on this crate; having it implement the contract from here instead, and drop that dependency, is a follow-up.

The contract is entirely sans-I/O: a [`Sequence`] consumes at most one input PDU per step and produces at most one output PDU, leaving all reading, writing, and timing to the caller.
A PDU always comes with the [`MonotonicInstant`] at which the driver's read of it completed, so a sequence never has to guess when its input arrived.

## What this crate provides

- [`Sequence`], [`State`], and [`Written`]: the state-machine contract itself.
- [`StepInput`]: either the one PDU a step consumes, with its arrival time, or no PDU at all.
- [`SequenceError`] and [`SequenceResult`]: the sspi-free error type a `Sequence` step can fail with.
- [`ServerName`] and [`DesktopSize`]: small value types shared by every implementor.
- [`MonotonicInstant`]: the millisecond clock reading a driver stamps its reads with, also used directly by `ironrdp-rdpeudp` for its own timers.

`ironrdp-connector` re-exports every public item from this crate so downstream code keeps using `ironrdp_connector::{Sequence, MonotonicInstant, ...}` unchanged.
`ironrdp-rdpeudp` re-exports [`MonotonicInstant`] the same way, as `ironrdp_rdpeudp::MonotonicInstant`.

## Feature Flags

- `std` (depends on `alloc`): enables `std` integration in `ironrdp-error`, `ironrdp-core`, and `ironrdp-pdu`.
- `alloc`: enables [`ServerName`], which stores a heap-allocated string.
- `state-machine` (depends on `alloc`): enables [`Sequence`], [`StepInput`], [`State`], [`Written`], [`SequenceError`], and the `general_err!`/`reason_err!`/`custom_err!` macros.
  These need `ironrdp-pdu` (for the PDU framing hint returned by [`Sequence::next_pdu_hint`] and for negotiation failure codes), pulled in as an optional dependency only when this feature is on.
  [`MonotonicInstant`], [`DesktopSize`], and [`ServerName`] have no such dependency and stay available without this feature, so a consumer that only needs the clock type — e.g. `ironrdp-rdpeudp` — is not forced to pull in `ironrdp-pdu`'s dependency tree.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
