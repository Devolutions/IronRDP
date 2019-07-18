macro_rules! try_read_optional {
    ($e:expr, $ret:expr) => {
        match $e {
            Ok(v) => v,
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok($ret);
            }
            Err(e) => return Err(From::from(e)),
        }
    };
}

macro_rules! try_write_optional {
    ($val:expr, $f:expr) => {
        if let Some(ref val) = $val {
            $f(val)?
        } else {
            return Ok(());
        }
    };
}

macro_rules! impl_from_error {
    ($from_e:ty, $to_e:ty, $to_e_variant:expr) => {
        impl From<$from_e> for $to_e {
            fn from(e: $from_e) -> Self {
                $to_e_variant(e)
            }
        }
    };
}
