use std::cmp::min;
use std::io;

use bitvec::field::BitField as _;
use bitvec::order::Msb0;
use bitvec::prelude::*;
use bitvec::slice::BitSlice;
use ironrdp_pdu::codecs::rfx::EntropyAlgorithm;
use thiserror::Error;

use crate::utils::Bits;

const KP_MAX: u32 = 80;
const LS_GR: u32 = 3;
const UP_GR: u32 = 4;
const DN_GR: u32 = 6;
const UQ_GR: u32 = 3;
const DQ_GR: u32 = 3;

macro_rules! write_byte {
    ($output:ident, $value:ident) => {
        if !$output.is_empty() {
            $output[0] = $value;
            $output = &mut $output[1..];
        } else {
            break;
        }
    };
}

macro_rules! try_split_bits {
    ($bits:ident, $n:expr) => {
        if $bits.len() < $n {
            break;
        } else {
            $bits.split_to($n)
        }
    };
}

struct BitStream<'a> {
    bits: &'a mut BitSlice<u8, Msb0>,
    idx: usize,
}

impl<'a> BitStream<'a> {
    fn new(slice: &'a mut [u8]) -> Self {
        let bits = slice.view_bits_mut::<Msb0>();
        Self { bits, idx: 0 }
    }

    fn output_bit(&mut self, count: usize, val: bool) {
        self.bits[self.idx..self.idx + count].fill(val);
        self.idx += count;
    }

    fn output_bits(&mut self, num_bits: usize, val: u32) {
        self.bits[self.idx..self.idx + num_bits].store_be(val);
        self.idx += num_bits;
    }

    fn len(&self) -> usize {
        self.idx.div_ceil(8)
    }
}

pub fn encode(mode: EntropyAlgorithm, input: &[i16], tile: &mut [u8]) -> Result<usize, RlgrError> {
    let mut k: u32 = 1;
    let kr: u32 = 1;
    let mut kp: u32 = k << LS_GR;
    let mut krp: u32 = kr << LS_GR;

    if input.is_empty() {
        return Err(RlgrError::EmptyTile);
    }

    let mut bits = BitStream::new(tile);
    let mut input = input.iter().peekable();
    while input.peek().is_some() {
        match CompressionMode::from(k) {
            CompressionMode::RunLength => {
                let mut nz = 0;
                while let Some(&&x) = input.peek() {
                    if x == 0 {
                        nz += 1;
                        input.next();
                    } else {
                        break;
                    }
                }
                let mut runmax: u32 = 1 << k;
                while nz >= runmax {
                    bits.output_bit(1, false);
                    nz -= runmax;
                    kp = min(kp + UP_GR, KP_MAX);
                    k = kp >> LS_GR;
                    runmax = 1 << k;
                }
                bits.output_bit(1, true);
                bits.output_bits(k as usize, nz);

                if let Some(val) = input.next() {
                    let mag = val.unsigned_abs() as u32;
                    bits.output_bit(1, *val < 0);
                    code_gr(&mut bits, &mut krp, mag - 1);
                }
                kp = kp.saturating_sub(DN_GR);
                k = kp >> LS_GR;
            }
            CompressionMode::GolombRice => match mode {
                EntropyAlgorithm::Rlgr1 => {
                    let two_ms = get_2magsign(*input.next().unwrap());
                    code_gr(&mut bits, &mut krp, two_ms);
                    if two_ms == 0 {
                        kp = min(kp + UP_GR, KP_MAX);
                    } else {
                        kp = kp.saturating_sub(DQ_GR);
                    }
                    k = kp >> LS_GR;
                }
                EntropyAlgorithm::Rlgr3 => {
                    let two_ms1 = input.next().map(|&n| get_2magsign(n)).unwrap();
                    let two_ms2 = input.next().map(|&n| get_2magsign(n)).unwrap_or(1);
                    let sum2ms = two_ms1 + two_ms2;
                    code_gr(&mut bits, &mut krp, sum2ms);

                    let m = 32 - sum2ms.leading_zeros() as usize;
                    if m != 0 {
                        bits.output_bits(m, two_ms1);
                    }

                    if two_ms1 != 0 && two_ms2 != 0 {
                        kp = kp.saturating_sub(2 * DQ_GR);
                        k = kp >> LS_GR;
                    } else if two_ms1 == 0 && two_ms2 == 0 {
                        kp = min(kp + 2 * UQ_GR, KP_MAX);
                        k = kp >> LS_GR;
                    }
                }
            },
        }
    }

    Ok(bits.len())
}

fn get_2magsign(val: i16) -> u32 {
    let sign = if val < 0 { 1 } else { 0 };

    (val.unsigned_abs() as u32) * 2 - sign
}

fn code_gr(bits: &mut BitStream<'_>, krp: &mut u32, val: u32) {
    let kr = (*krp >> LS_GR) as usize;
    let vk = (val >> kr) as usize;

    bits.output_bit(vk, true);
    bits.output_bit(1, false);
    if kr != 0 {
        let remainder = val & ((1 << kr) - 1);
        bits.output_bits(kr, remainder);
    }
    if vk == 0 {
        *krp = krp.saturating_sub(2);
    } else if vk > 1 {
        *krp = min(*krp + vk as u32, KP_MAX);
    }
}

pub fn decode(mode: EntropyAlgorithm, tile: &[u8], mut output: &mut [i16]) -> Result<(), RlgrError> {
    let mut k: u32 = 1;
    let mut kr: u32 = 1;
    let mut kp: u32 = k << LS_GR;
    let mut krp: u32 = kr << LS_GR;

    if tile.is_empty() {
        return Err(RlgrError::EmptyTile);
    }

    let mut bits = Bits::new(BitSlice::from_slice(tile));
    while !bits.is_empty() && !output.is_empty() {
        match CompressionMode::from(k) {
            CompressionMode::RunLength => {
                let number_of_zeros = truncate_leading_value(&mut bits, false);
                try_split_bits!(bits, 1);
                let run = count_run(number_of_zeros, &mut k, &mut kp) + load_be_u32(try_split_bits!(bits, k as usize));

                let sign_bit = try_split_bits!(bits, 1).load_be::<u8>();

                let number_of_ones = truncate_leading_value(&mut bits, true);
                try_split_bits!(bits, 1);

                let code_remainder = load_be_u32(try_split_bits!(bits, kr as usize)) + ((number_of_ones as u32) << kr);

                update_parameters_according_to_number_of_ones(number_of_ones, &mut kr, &mut krp);
                kp = kp.saturating_sub(DN_GR);
                k = kp >> LS_GR;

                let magnitude = compute_rl_magnitude(sign_bit, code_remainder);

                let size = min(run as usize, output.len());
                fill(&mut output[..size], 0);
                output = &mut output[size..];
                write_byte!(output, magnitude);
            }
            CompressionMode::GolombRice => {
                let number_of_ones = truncate_leading_value(&mut bits, true);
                try_split_bits!(bits, 1);

                let code_remainder = load_be_u32(try_split_bits!(bits, kr as usize)) + ((number_of_ones as u32) << kr);

                update_parameters_according_to_number_of_ones(number_of_ones, &mut kr, &mut krp);

                match mode {
                    EntropyAlgorithm::Rlgr1 => {
                        let magnitude = compute_rlgr1_magnitude(code_remainder, &mut k, &mut kp);
                        write_byte!(output, magnitude);
                    }
                    EntropyAlgorithm::Rlgr3 => {
                        let n_index = compute_n_index(code_remainder);

                        let val1 = load_be_u32(try_split_bits!(bits, n_index));
                        let val2 = code_remainder - val1;
                        if val1 != 0 && val2 != 0 {
                            kp = kp.saturating_sub(2 * DQ_GR);
                            k = kp >> LS_GR;
                        } else if val1 == 0 && val2 == 0 {
                            kp = min(kp + 2 * UQ_GR, KP_MAX);
                            k = kp >> LS_GR;
                        }

                        let magnitude = compute_rlgr3_magnitude(val1);
                        write_byte!(output, magnitude);

                        let magnitude = compute_rlgr3_magnitude(val2);
                        write_byte!(output, magnitude);
                    }
                }
            }
        }
    }

    // fill remaining buffer with zeros
    fill(output, 0);

    Ok(())
}

fn fill(buffer: &mut [i16], value: i16) {
    for v in buffer {
        *v = value;
    }
}

fn load_be_u32(s: &BitSlice<u8, Msb0>) -> u32 {
    if s.is_empty() {
        0
    } else {
        s.load_be::<u32>()
    }
}

// Returns number of truncated bits
fn truncate_leading_value(bits: &mut Bits<'_>, value: bool) -> usize {
    let leading_values = if value {
        bits.leading_ones()
    } else {
        bits.leading_zeros()
    };
    bits.split_to(leading_values);
    leading_values
}

fn count_run(number_of_zeros: usize, k: &mut u32, kp: &mut u32) -> u32 {
    (0..number_of_zeros)
        .map(|_| {
            let run = 1 << *k;
            *kp = min(*kp + UP_GR, KP_MAX);
            *k = *kp >> LS_GR;

            run
        })
        .sum()
}

fn compute_rl_magnitude(sign_bit: u8, code_remainder: u32) -> i16 {
    if sign_bit != 0 {
        -((code_remainder + 1) as i16)
    } else {
        (code_remainder + 1) as i16
    }
}

fn compute_rlgr1_magnitude(code_remainder: u32, k: &mut u32, kp: &mut u32) -> i16 {
    if code_remainder == 0 {
        *kp = min(*kp + UQ_GR, KP_MAX);
        *k = *kp >> LS_GR;

        0
    } else {
        *kp = kp.saturating_sub(DQ_GR);
        *k = *kp >> LS_GR;

        if code_remainder % 2 != 0 {
            -(((code_remainder + 1) >> 1) as i16)
        } else {
            (code_remainder >> 1) as i16
        }
    }
}

fn compute_rlgr3_magnitude(val: u32) -> i16 {
    if val % 2 != 0 {
        -(((val + 1) >> 1) as i16)
    } else {
        (val >> 1) as i16
    }
}

fn compute_n_index(code_remainder: u32) -> usize {
    if code_remainder == 0 {
        return 0;
    }

    let code_bytes = code_remainder.to_be_bytes();
    let code_bits = BitSlice::<u8, Msb0>::from_slice(code_bytes.as_ref());
    let leading_zeros = code_bits.leading_zeros();

    32 - leading_zeros
}

fn update_parameters_according_to_number_of_ones(number_of_ones: usize, kr: &mut u32, krp: &mut u32) {
    if number_of_ones == 0 {
        *krp = (*krp).saturating_sub(2);
        *kr = *krp >> LS_GR;
    } else if number_of_ones > 1 {
        *krp = min(*krp + number_of_ones as u32, KP_MAX);
        *kr = *krp >> LS_GR;
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum CompressionMode {
    RunLength,
    GolombRice,
}

impl From<u32> for CompressionMode {
    fn from(m: u32) -> Self {
        if m != 0 {
            Self::RunLength
        } else {
            Self::GolombRice
        }
    }
}

#[derive(Debug, Error)]
pub enum RlgrError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
    #[error("the input tile is empty")]
    EmptyTile,
}
