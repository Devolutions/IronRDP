#[macro_export]
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

#[macro_export]
macro_rules! try_write_optional {
    ($val:expr, $f:expr) => {
        if let Some(ref val) = $val {
            $f(val)?
        } else {
            return Ok(());
        }
    };
}
