use std::borrow::Cow;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::tpdu::{TpduCode, TpduHeader};
use crate::tpkt::TpktHeader;
use crate::{IntoOwnedPdu, Pdu, PduDecode, PduEncode, PduError, PduErrorExt as _, PduResult};

pub trait X224Pdu<'de>: Sized {
    const X224_NAME: &'static str;

    const TPDU_CODE: TpduCode;

    fn x224_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;

    fn x224_body_decode(src: &mut ReadCursor<'de>, tpkt: &TpktHeader, tpdu: &TpduHeader) -> PduResult<Self>;

    fn tpdu_header_variable_part_size(&self) -> usize;

    fn tpdu_user_data_size(&self) -> usize;
}

impl<'de, T> Pdu for T
where
    T: X224Pdu<'de>,
{
    const NAME: &'static str = T::X224_NAME;
}

impl<'de, T> PduEncode for T
where
    T: X224Pdu<'de>,
{
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let packet_length = self.size();

        ensure_size!(in: dst, size: packet_length);

        TpktHeader {
            packet_length: u16::try_from(packet_length).unwrap(),
        }
        .write(dst)?;

        TpduHeader {
            li: u8::try_from(T::TPDU_CODE.header_fixed_part_size() + self.tpdu_header_variable_part_size() - 1)
                .unwrap(),
            code: T::TPDU_CODE,
        }
        .write(dst)?;

        self.x224_body_encode(dst)
    }

    fn name(&self) -> &'static str {
        T::X224_NAME
    }

    fn size(&self) -> usize {
        TpktHeader::SIZE
            + T::TPDU_CODE.header_fixed_part_size()
            + self.tpdu_header_variable_part_size()
            + self.tpdu_user_data_size()
    }
}

impl<'de, T> PduDecode<'de> for T
where
    T: X224Pdu<'de>,
{
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let tpkt = TpktHeader::read(src)?;

        ensure_size!(in: src, size: tpkt.packet_length().saturating_sub(TpktHeader::SIZE));

        let tpdu = TpduHeader::read(src, &tpkt)?;
        tpdu.code.check_expected(T::TPDU_CODE)?;

        if tpdu.size() < tpdu.fixed_part_size() {
            return Err(PduError::invalid_message(
                "TpduHeader",
                "li",
                "fixed part bigger than total header size",
            ));
        }

        T::x224_body_decode(src, &tpkt, &tpdu)
    }
}

pub struct X224Data<'a> {
    pub data: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(X224Data<'_>, OwnedX224Data);

impl IntoOwnedPdu for X224Data<'_> {
    type Owned = OwnedX224Data;

    fn into_owned_pdu(self) -> Self::Owned {
        X224Data {
            data: Cow::Owned(self.data.into_owned()),
        }
    }
}

impl<'de> X224Pdu<'de> for X224Data<'de> {
    const X224_NAME: &'static str = "X.224 Data";

    const TPDU_CODE: TpduCode = TpduCode::DATA;

    fn x224_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.data.len());
        dst.write_slice(&self.data);

        Ok(())
    }

    fn x224_body_decode(src: &mut ReadCursor<'de>, tpkt: &TpktHeader, tpdu: &TpduHeader) -> PduResult<Self> {
        let user_data_size = user_data_size(tpkt, tpdu);

        ensure_size!(in: src, size: user_data_size);
        let data = src.read_slice(user_data_size);

        Ok(Self {
            data: Cow::Borrowed(data),
        })
    }

    fn tpdu_header_variable_part_size(&self) -> usize {
        0
    }

    fn tpdu_user_data_size(&self) -> usize {
        self.data.len()
    }
}

pub fn user_data_size(tpkt: &TpktHeader, tpdu: &TpduHeader) -> usize {
    tpkt.packet_length() - TpktHeader::SIZE - tpdu.size()
}
