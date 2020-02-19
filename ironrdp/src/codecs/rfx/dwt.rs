#[cfg(test)]
mod tests;

use crate::split_to;

pub fn decode(buffer: &mut [i16], temp_buffer: &mut [i16]) {
    decode_block(&mut buffer[3840..], temp_buffer, 8);
    decode_block(&mut buffer[3072..], temp_buffer, 16);
    decode_block(&mut buffer[..], temp_buffer, 32);
}

fn decode_block(buffer: &mut [i16], temp_buffer: &mut [i16], subband_width: usize) {
    inverse_horizontal(buffer, temp_buffer, subband_width);
    inverse_vertical(buffer, temp_buffer, subband_width);
}

// Inverse DWT in horizontal direction, results in 2 sub-bands in L, H order in output buffer
// The 4 sub-bands are stored in HL(0), LH(1), HH(2), LL(3) order.
// The lower part L uses LL(3) and HL(0).
// The higher part H uses LH(1) and HH(2).
fn inverse_horizontal(mut buffer: &[i16], temp_buffer: &mut [i16], subband_width: usize) {
    let total_width = subband_width * 2;
    let squared_subband_width = subband_width.pow(2);

    let mut hl = split_to!(buffer, squared_subband_width);
    let mut lh = split_to!(buffer, squared_subband_width);
    let mut hh = split_to!(buffer, squared_subband_width);
    let mut ll = buffer;

    let (mut l_dst, mut h_dst) = temp_buffer.split_at_mut(squared_subband_width * 2);

    for _ in 0..subband_width {
        // Even coefficients
        l_dst[0] = (i32::from(ll[0]) - ((i32::from(hl[0]) + i32::from(hl[0]) + 1) >> 1)) as i16;
        h_dst[0] = (i32::from(lh[0]) - ((i32::from(hh[0]) + i32::from(hh[0]) + 1) >> 1)) as i16;
        for n in 1..subband_width {
            let x = n * 2;
            l_dst[x] =
                (i32::from(ll[n]) - ((i32::from(hl[n - 1]) + i32::from(hl[n]) + 1) >> 1)) as i16;
            h_dst[x] =
                (i32::from(lh[n]) - ((i32::from(hh[n - 1]) + i32::from(hh[n]) + 1) >> 1)) as i16;
        }

        // Odd coefficients
        for n in 0..subband_width - 1 {
            let x = n * 2;
            l_dst[x + 1] = (i32::from(hl[n] << 1)
                + ((i32::from(l_dst[x]) + i32::from(l_dst[x + 2])) >> 1))
                as i16;
            h_dst[x + 1] = (i32::from(hh[n] << 1)
                + ((i32::from(h_dst[x]) + i32::from(h_dst[x + 2])) >> 1))
                as i16;
        }
        let n = subband_width - 1;
        let x = n * 2;
        l_dst[x + 1] = (i32::from(hl[n] << 1) + i32::from(l_dst[x])) as i16;
        h_dst[x + 1] = (i32::from(hh[n] << 1) + i32::from(h_dst[x])) as i16;

        hl = &hl[subband_width..];
        lh = &lh[subband_width..];
        hh = &hh[subband_width..];
        ll = &ll[subband_width..];

        l_dst = &mut l_dst[total_width..];
        h_dst = &mut h_dst[total_width..];
    }
}

fn inverse_vertical(mut buffer: &mut [i16], mut temp_buffer: &[i16], subband_width: usize) {
    let total_width = subband_width * 2;

    for _ in 0..total_width {
        buffer[0] = (i32::from(temp_buffer[0])
            - ((i32::from(temp_buffer[subband_width * total_width]) * 2 + 1) >> 1))
            as i16;

        let mut l = &temp_buffer[..];
        let mut lh = &temp_buffer[(subband_width - 1) * total_width..];
        let mut h = &temp_buffer[subband_width * total_width..];
        let mut dst = &mut buffer[..];

        for _ in 1..subband_width {
            l = &l[total_width..];
            lh = &lh[total_width..];
            h = &h[total_width..];

            // Even coefficients
            dst[2 * total_width] =
                (i32::from(l[0]) - ((i32::from(lh[0]) + i32::from(h[0]) + 1) >> 1)) as i16;

            // Odd coefficients
            dst[total_width] = (i32::from(lh[0] << 1)
                + ((i32::from(dst[0]) + i32::from(dst[2 * total_width])) >> 1))
                as i16;

            dst = &mut dst[2 * total_width..];
        }

        dst[total_width] = (i32::from(lh[total_width] << 1)
            + ((i32::from(dst[0]) + i32::from(dst[0])) >> 1)) as i16;

        temp_buffer = &temp_buffer[1..];
        buffer = &mut buffer[1..];
    }
}
