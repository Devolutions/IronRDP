use ironrdp_pdu::utils::SplitTo as _;

pub fn encode(buffer: &mut [i16], temp_buffer: &mut [i16]) {
    encode_block(&mut *buffer, temp_buffer, 32);
    encode_block(&mut buffer[3072..], temp_buffer, 16);
    encode_block(&mut buffer[3840..], temp_buffer, 8);
}

fn encode_block(buffer: &mut [i16], temp_buffer: &mut [i16], subband_width: usize) {
    dwt_vertical(buffer, temp_buffer, subband_width);
    dwt_horizontal(buffer, temp_buffer, subband_width);
}

// DWT in vertical direction, results in 2 sub-bands in L, H order in tmp buffer dwt.
fn dwt_vertical(buffer: &[i16], dwt: &mut [i16], subband_width: usize) {
    let total_width = subband_width * 2;

    for x in 0..total_width {
        for n in 0..subband_width {
            let y = n * 2;
            let l_index = n * total_width + x;
            let h_index = l_index + subband_width * total_width;
            let src_index = y * total_width + x;

            dwt[h_index] = ((i32::from(buffer[src_index + total_width])
                - ((i32::from(buffer[src_index])
                    + i32::from(buffer[src_index + if n < subband_width - 1 { 2 * total_width } else { 0 }]))
                    >> 1))
                >> 1) as i16;
            dwt[l_index] = (i32::from(buffer[src_index])
                + if n == 0 {
                    i32::from(dwt[h_index])
                } else {
                    (i32::from(dwt[h_index - total_width]) + i32::from(dwt[h_index])) >> 1
                }) as i16;
        }
    }
}

// DWT in horizontal direction, results in 4 sub-bands in HL(0), LH(1), HH(2),
// LL(3) order, stored in original buffer.
// The lower part L generates LL(3) and HL(0).
// The higher part H generates LH(1) and HH(2).
fn dwt_horizontal(mut buffer: &mut [i16], dwt: &[i16], subband_width: usize) {
    let total_width = subband_width * 2;
    let squared_subband_width = subband_width.pow(2);

    let mut hl = buffer.split_to(squared_subband_width);
    let mut lh = buffer.split_to(squared_subband_width);
    let mut hh = buffer.split_to(squared_subband_width);
    let mut ll = buffer;
    let (mut l_src, mut h_src) = dwt.split_at(squared_subband_width * 2);

    for _ in 0..subband_width {
        // L
        for n in 0..subband_width {
            let x = n * 2;

            // HL
            hl[n] = ((i32::from(l_src[x + 1])
                - ((i32::from(l_src[x]) + i32::from(l_src[if n < subband_width - 1 { x + 2 } else { x }])) >> 1))
                >> 1) as i16;
            // LL
            ll[n] = (i32::from(l_src[x])
                + if n == 0 {
                    i32::from(hl[n])
                } else {
                    (i32::from(hl[n - 1]) + i32::from(hl[n])) >> 1
                }) as i16;
        }

        // H
        for n in 0..subband_width {
            let x = n * 2;

            // HH
            hh[n] = ((i32::from(h_src[x + 1])
                - ((i32::from(h_src[x]) + i32::from(h_src[if n < subband_width - 1 { x + 2 } else { x }])) >> 1))
                >> 1) as i16;
            // LH
            lh[n] = (i32::from(h_src[x])
                + if n == 0 {
                    i32::from(hh[n])
                } else {
                    (i32::from(hh[n - 1]) + i32::from(hh[n])) >> 1
                }) as i16;
        }

        hl = &mut hl[subband_width..];
        lh = &mut lh[subband_width..];
        hh = &mut hh[subband_width..];
        ll = &mut ll[subband_width..];

        l_src = &l_src[total_width..];
        h_src = &h_src[total_width..];
    }
}

pub fn decode(buffer: &mut [i16], temp_buffer: &mut [i16]) {
    decode_block(&mut buffer[3840..], temp_buffer, 8);
    decode_block(&mut buffer[3072..], temp_buffer, 16);
    decode_block(&mut *buffer, temp_buffer, 32);
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

    let mut hl = buffer.split_to(squared_subband_width);
    let mut lh = buffer.split_to(squared_subband_width);
    let mut hh = buffer.split_to(squared_subband_width);
    let mut ll = buffer;

    let (mut l_dst, mut h_dst) = temp_buffer.split_at_mut(squared_subband_width * 2);

    for _ in 0..subband_width {
        // Even coefficients
        l_dst[0] = (i32::from(ll[0]) - ((i32::from(hl[0]) + i32::from(hl[0]) + 1) >> 1)) as i16;
        h_dst[0] = (i32::from(lh[0]) - ((i32::from(hh[0]) + i32::from(hh[0]) + 1) >> 1)) as i16;
        for n in 1..subband_width {
            let x = n * 2;
            l_dst[x] = (i32::from(ll[n]) - ((i32::from(hl[n - 1]) + i32::from(hl[n]) + 1) >> 1)) as i16;
            h_dst[x] = (i32::from(lh[n]) - ((i32::from(hh[n - 1]) + i32::from(hh[n]) + 1) >> 1)) as i16;
        }

        // Odd coefficients
        for n in 0..subband_width - 1 {
            let x = n * 2;
            l_dst[x + 1] = (i32::from(hl[n] << 1) + ((i32::from(l_dst[x]) + i32::from(l_dst[x + 2])) >> 1)) as i16;
            h_dst[x + 1] = (i32::from(hh[n] << 1) + ((i32::from(h_dst[x]) + i32::from(h_dst[x + 2])) >> 1)) as i16;
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
        buffer[0] =
            (i32::from(temp_buffer[0]) - ((i32::from(temp_buffer[subband_width * total_width]) * 2 + 1) >> 1)) as i16;

        let mut l = temp_buffer;
        let mut lh = &temp_buffer[(subband_width - 1) * total_width..];
        let mut h = &temp_buffer[subband_width * total_width..];
        let mut dst = &mut *buffer;

        for _ in 1..subband_width {
            l = &l[total_width..];
            lh = &lh[total_width..];
            h = &h[total_width..];

            // Even coefficients
            dst[2 * total_width] = (i32::from(l[0]) - ((i32::from(lh[0]) + i32::from(h[0]) + 1) >> 1)) as i16;

            // Odd coefficients
            dst[total_width] =
                (i32::from(lh[0] << 1) + ((i32::from(dst[0]) + i32::from(dst[2 * total_width])) >> 1)) as i16;

            dst = &mut dst[2 * total_width..];
        }

        dst[total_width] = (i32::from(lh[total_width] << 1) + ((i32::from(dst[0]) + i32::from(dst[0])) >> 1)) as i16;

        temp_buffer = &temp_buffer[1..];
        buffer = &mut buffer[1..];
    }
}
