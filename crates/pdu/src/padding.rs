use crate::cursor::{ReadCursor, WriteCursor};

/// Use this when handling padding
pub struct Padding<const N: usize>();

impl<const N: usize> Padding<N> {
    //= https://www.rfc-editor.org/rfc/rfc6143.html#section-7
    //# For maximum
    //# compatibility, messages should be generated with padding set to zero,
    //# but message recipients should not assume padding has any particular
    //# value.

    pub fn write(dst: &mut WriteCursor<'_>) {
        dst.write_array([0; N]);
    }

    pub fn read(src: &mut ReadCursor<'_>) {
        src.advance(N);
    }
}
