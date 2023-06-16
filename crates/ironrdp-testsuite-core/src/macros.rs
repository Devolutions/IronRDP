#[macro_export]
macro_rules! encode_decode_test {
    ($test_name:ident : $pdu:expr , $encoded_pdu:expr) => {
        $crate::paste! {
            #[test]
            fn [< $test_name _encode >]() {
                let pdu = $pdu;
                let expected = $encoded_pdu;

                let mut encoded = Vec::new();
                ::ironrdp_pdu::encode_buf(&pdu, &mut encoded).unwrap();

                ::assert_hex::assert_eq_hex!(encoded, expected);
            }

            #[test]
            fn [< $test_name _decode >]() {
                let encoded = $encoded_pdu;
                let expected = $pdu;

                let decoded = ::ironrdp_pdu::decode(&encoded).unwrap();

                let _ = expected == decoded; // type inference trick

                ::assert_hex::assert_eq_hex!(decoded, expected);
            }

            #[test]
            fn [< $test_name _size >]() {
                let pdu = $pdu;
                let expected = $encoded_pdu.len();

                let pdu_size = ::ironrdp_pdu::size(&pdu);

                ::assert_hex::assert_eq_hex!(pdu_size, expected);
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
                let mut cursor = ::ironrdp_pdu::cursor::WriteCursor::new(&mut encoded);
                pdu.mcs_body_encode(&mut cursor).unwrap();

                ::assert_hex::assert_eq_hex!(encoded, expected);
            }

            #[test]
            fn [< $test_name _decode >]() {
                use ::ironrdp_pdu::mcs::McsPdu;

                let encoded = $encoded_pdu;
                let expected = $pdu;

                let mut cursor = ::ironrdp_pdu::cursor::ReadCursor::new(&encoded);
                let decoded = McsPdu::mcs_body_decode(&mut cursor, encoded.len()).unwrap();

                let _ = expected == decoded; // type inference trick

                ::assert_hex::assert_eq_hex!(decoded, expected);
            }

            #[test]
            fn [< $test_name _size >]() {
                use ::ironrdp_pdu::mcs::McsPdu;

                let pdu = $pdu;
                let expected = $encoded_pdu.len();

                let pdu_size = pdu.mcs_size();

                ::assert_hex::assert_eq_hex!(pdu_size, expected);
            }
        }
    };
    ($( $test_name:ident : $pdu:expr , $encoded_pdu:expr ; )+) => {
        $(
            $crate::mcs_encode_decode_test!($test_name: $pdu, $encoded_pdu);
        )+
    };
}
