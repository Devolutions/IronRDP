# IronRDP PDU

RDP PDU encoding and decoding library.

## WIP: overview of encoding and decoding traits

It’s important for `PduEncode` to be object-safe in order to enable patterns such as the one
found in `ironrdp-svc`:

```rust
pub trait StaticVirtualChannel {
    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<Box<dyn PduEncode>>>;
}
```

(The actual trait is a bit more complicated, but this gives the rough idea.)

TODO: elaborate this section

## Difference between `WriteBuf` and `WriteCursor`

`WriteBuf` is used in APIs where the required space cannot be known in advance. For instance,
`ironrdp_connector::Sequence::step` is taking `&mut WriteBuf` instead of `&mut
WriteCursor` because it’s unlikely the user knows exactly how much space is required to encode a
specific response before actually processing the input payload, and as such there is no way
to communicate the caller how much space is required before calling the function.

`WriteCursor`, in essence, is a helper for this kind of code:

```rust
    let pdu = /* some PDU structure */;

    // Find the required space
    let pdu_size = pdu.size();

    // Resize the buffer, making sure there is enough space to fit the serialized PDU
    if buf.len() < pdu_size {
        buf.resize(pdu_size, 0);
    }

    // Proceed to actually serialize the PDU into the buffer…
    let mut cursor = WriteCursor::new(dst);
    encode_cursor(pdu, &mut cursor)?;
```

But it’s going a bit further by keeping track of the already filled region (as opposed to the
initialized yet unfilled region of the inner `Vec<u8>`). One purpose is to facilitate buffer re-
use: it’s okay to keep around the same `WriteBuf` and repeatedly write into it. Contrary to
`Vec<u8>`, bytes will be written at the beginning of the unfilled region (a `Vec<u8>` will write
bytes at the beginning of the uninitialized region). This way, it’s easy and safe to retrieve a
mutable slice of the unfilled (but already initialized) region. This slice can be used to build
a `WriteCursor`. Another purpose of the filled region tracking is to enable multiple items to be
written consecutively in the same buffer. For instance, the `encode_buf` function looks like this:

```rust
pub fn encode_buf<T: PduEncode + ?Sized>(pdu: &T, buf: &mut WriteBuf) -> PduResult<usize> {
    let pdu_size = pdu.size();
    let dst = buf.unfilled_to(pdu_size);
    let mut cursor = WriteCursor::new(dst);
    encode_cursor(pdu, &mut cursor)?;
    let written = cursor.pos();
    buf.advance(written);
    Ok(written)
}
```

A big enough mutable slice of the unfilled region is retrieved by using the `unfilled_to` helper
method (allocating more memory as necessary), a `WriteCursor` wrapping this slice is created and
used to actually encode the PDU. The filled region cursor of the `WriteBuf` is moved forward so that
the `filled` method returns the right region, and subsequent calls to `encode_buf` do not override
it until `clear` is called.

Consider this piece of code:

```rust
fn process(&mut self, payload: &[u8], output: &mut WriteBuf) -> PduResult<()> {
    let server_request = ServerRequest: ironrdp_pdu::decode(payload)?;

    match server_request.order {
        ServerOrder::DoThis => {
            // do this
            let response = DoThisResponse { … };

            // buffer is grown, or not, as appropriate, and `DoThisResponse` is encoded in the "unfilled" region
            ironrdp_pdu::encode_buf(response, output)?;
        }
        ServerOrder::DoThat => {
            // do that
            let response = DoThatResponse { … };

            // same as above
            ironrdp_pdu::encode_buf(response, output)?;
        }
    }

    Ok(())
}
```

Methods such as `write_u8` are overlapping with the `WriteCursor` API, but it’s mostly for
convenience if one needs to manually write something in an ad-hoc fashion.

Otherwise, using `WriteCursor` is prefered in order to write `no-std` and `no-alloc` friendly code.

## Most PDUs are "plain old data" structures with public fields

In general, it is desirable for library users to be able to manipulate the fields directly because:

- One can deconstruct the types using pattern matching
- One can construct the type using a [field struct expression][1] (which is arguably more readable
  than a `new` method taking tons of parameters, and less boilerplate than a builder)
- One can use the [struct (functional) update syntax][2]
- One can move fields out of the struct without us having to add additional `into_xxx` API for each
  combination
- One can mutably borrow multiple fields of the struct at the same time (a getter will cause the
  entire struct to be considered borrowed by the borrow checker)

Keeping the fields private is opting out of all of this and making the API stiffer. Of course,
all of this applies because for most PDU structs there is no important invariants to uphold: they
are mostly dumb data holders or "plain old data structures" with no particular logic to run at
construction or destruction time. The story is not the same for objects with business logic (which
are mostly not part of `ironrdp-pdu`)

When hiding some fields is really required, one of the following approach is suggested:
- a "[Builder Lite][3]" pattern,
- a "[Init Struct][4]" pattern, or
- a standard handcrafted Builder Pattern

[1]: https://doc.rust-lang.org/reference/expressions/struct-expr.html#field-struct-expression
[2]: https://doc.rust-lang.org/reference/expressions/struct-expr.html#functional-update-syntax
[3]: https://matklad.github.io/2022/05/29/builder-lite.html
[4]: https://xaeroxe.github.io/init-struct-pattern/

