/// Same macro as in `assert_hex` crate, but use `{:02X?}` instead of `{:#x}` because the alternate formatting
/// for slice / Vec is inserting a newline between each element which is not very readable for binary payloads.
///
/// [Original macro](https://docs.rs/assert_hex/latest/src/assert_hex/lib.rs.html#19).
#[macro_export]
macro_rules! assert_eq_hex {
    ($left:expr, $right:expr $(,)?) => ({
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    // The reborrows below are intentional. Without them, the stack slot for the
                    // borrow is initialized even before the values are compared, leading to a
                    // noticeable slow down.
                    panic!(r#"assertion failed: `(left == right)`
  left: `{:02X?}`,
 right: `{:02X?}`"#, &*left_val, &*right_val)
                }
            }
        }
    });
    ($left:expr, $right:expr, $($arg:tt)+) => ({
        match (&($left), &($right)) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    // The reborrows below are intentional. Without them, the stack slot for the
                    // borrow is initialized even before the values are compared, leading to a
                    // noticeable slow down.
                    panic!(r#"assertion failed: `(left == right)`
  left: `{:02X?}`,
 right: `{:02X?}`: {}"#, &*left_val, &*right_val,
                           format_args!($($arg)+))
                }
            }
        }
    });
}

/// Same macro as in `assert_hex` crate, but use `{:02X?}` instead of `{:#x}` because the alternate formatting
/// for slice / Vec is inserting a newline between each element which is not very readable for binary payloads.
///
/// [Original macro](https://docs.rs/assert_hex/latest/src/assert_hex/lib.rs.html#56).
#[macro_export]
macro_rules! assert_ne_hex {
    ($left:expr, $right:expr $(,)?) => ({
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    // The reborrows below are intentional. Without them, the stack slot for the
                    // borrow is initialized even before the values are compared, leading to a
                    // noticeable slow down.
                    panic!(r#"assertion failed: `(left != right)`
  left: `{:02X?}`,
 right: `{:02X?}`"#, &*left_val, &*right_val)
                }
            }
        }
    });
    ($left:expr, $right:expr, $($arg:tt)+) => ({
        match (&($left), &($right)) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    // The reborrows below are intentional. Without them, the stack slot for the
                    // borrow is initialized even before the values are compared, leading to a
                    // noticeable slow down.
                    panic!(r#"assertion failed: `(left != right)`
  left: `{:02X?}`,
 right: `{:02X?}`: {}"#, &*left_val, &*right_val,
                           format_args!($($arg)+))
                }
            }
        }
    });
}

#[macro_export]
macro_rules! encode_decode_test {
    ($test_name:ident : $pdu:expr , $encoded_pdu:expr) => {
        $crate::paste! {
            #[test]
            fn [< $test_name _encode >]() {
                let pdu = $pdu;
                let expected = $encoded_pdu;

                let encoded = ::ironrdp_pdu::encode_vec(&pdu).unwrap();

                $crate::assert_eq_hex!(encoded, expected);
            }

            #[test]
            fn [< $test_name _decode >]() {
                let encoded = $encoded_pdu;
                let expected = $pdu;

                let decoded = ::ironrdp_pdu::decode(&encoded).unwrap();

                let _ = expected == decoded; // type inference trick

                $crate::assert_eq_hex!(decoded, expected);
            }

            #[test]
            fn [< $test_name _size >]() {
                let pdu = $pdu;
                let expected = $encoded_pdu.len();

                let pdu_size = ::ironrdp_pdu::size(&pdu);

                $crate::assert_eq_hex!(pdu_size, expected);
            }
        }
    };
    ($( $test_name:ident : $pdu:expr , $encoded_pdu:expr ; )+) => {
        $(
            $crate::encode_decode_test!($test_name: $pdu, $encoded_pdu);
        )+
    };
}

#[macro_export]
macro_rules! mcs_encode_decode_test {
    ($test_name:ident : $pdu:expr , $encoded_pdu:expr) => {
        $crate::paste! {
            #[test]
            fn [< $test_name _encode >]() {
                use ::ironrdp_pdu::mcs::McsPdu;

                let pdu = $pdu;
                let expected = $encoded_pdu;

                let mut encoded = vec![0; expected.len()];
                let mut cursor = ::ironrdp_core::WriteCursor::new(&mut encoded);
                pdu.mcs_body_encode(&mut cursor).unwrap();

                $crate::assert_eq_hex!(encoded, expected);
            }

            #[test]
            fn [< $test_name _decode >]() {
                use ::ironrdp_pdu::mcs::McsPdu;

                let encoded = $encoded_pdu;
                let expected = $pdu;

                let mut cursor = ::ironrdp_core::ReadCursor::new(&encoded);
                let decoded = McsPdu::mcs_body_decode(&mut cursor, encoded.len()).unwrap();

                let _ = expected == decoded; // type inference trick

                $crate::assert_eq_hex!(decoded, expected);
            }

            #[test]
            fn [< $test_name _size >]() {
                use ::ironrdp_pdu::mcs::McsPdu;

                let pdu = $pdu;
                let expected = $encoded_pdu.len();

                let pdu_size = pdu.mcs_size();

                $crate::assert_eq_hex!(pdu_size, expected);
            }
        }
    };
    ($( $test_name:ident : $pdu:expr , $encoded_pdu:expr ; )+) => {
        $(
            $crate::mcs_encode_decode_test!($test_name: $pdu, $encoded_pdu);
        )+
    };
}
