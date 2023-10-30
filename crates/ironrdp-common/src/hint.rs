use core::any::Any;

use ironrdp_error::err_desc;

use crate::{AsAny, DecodeError, DecodeErrorKind, DecodeResult};

pub trait Hint: AsAny + Send + Sync + core::fmt::Debug + 'static {
    /// Finds the encoded length of the associated structure by the first few bytes.
    fn find_size(&self, bytes: &[u8]) -> DecodeResult<Option<usize>> {
        let _ = bytes; // silent the error without modifying the name of the argument
        Err(DecodeError::new("Hint::find_size", DecodeErrorKind::Other).with_source(err_desc!("unimplemented")))
    }
}

/// A type with a static hint.
pub trait StaticHint {
    /// Hint associated to this type.
    const HINT: &'static dyn Hint;
}

/// A type from which a hint can be retrieved.
pub trait GetHint {
    /// Returns the hint associated to this value.
    fn get_hint(&self) -> &'static dyn Hint;
}

impl<T: StaticHint> GetHint for T {
    fn get_hint(&self) -> &'static dyn Hint {
        T::HINT
    }
}

assert_obj_safe!(GetHint);

/// Gets the hint of this value.
pub fn get_hint<T: GetHint>(value: &T) -> &'static dyn Hint {
    value.get_hint()
}

pub fn hint_downcast<T: Hint + Any>(hint: &dyn Hint) -> Option<&T> {
    hint.as_any().downcast_ref()
}

pub fn hint_is<T: Hint + Any>(hint: &dyn Hint) -> bool {
    hint.as_any().is::<T>()
}
