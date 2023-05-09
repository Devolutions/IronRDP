# IronRDP fuzzing

## Difference between fuzzing and property testing

`ironrdp` correctness is validated in various ways. Two of these are fuzzing and property testing.
Both of these methods involve feeding random inputs to the API in order to check if the program is
behaving as expected or not.

However,

- Fuzzing is well suited for black-box-like testing.
  Inputs are typically guided by instrumentalizing the code (coverage…) rather than manually informed.

- Property testing requires the developer to describe the interesting inputs and properties to test.

- When fuzzing, some properties are tested as well, but those are typically simple (absence of crash, round-trip…).

- In contrast, property testing is well suited when testing more complex properties.

- With fuzzing, we are actively trying to show that something is (unexpectedly) broken.

- With property testing, we are actively trying to show that the properties are holding (as expected).

## Targets

### `pdu_decoding`

Feeds random inputs to PDU decoding code.

### `bitmap_stream`

Feeds random inputs to the RDP6 bitmap decoder.

### `rle_decompression`

Feeds random inputs to the interleaved Run-Length Encoding (RLE) bitmap decoder.
