//! Microsoft Point-to-Point Compression (MPPC) decompressor (RDP bulk compression).
//!
//! Implements the decompressor side of MPPC per RFC 2118 and the RDP4/RDP5
//! bulk compression variants (8KiB/64KiB history window). The implementation is
//! stateful across calls to support history reuse between PDUs. Flags
//! `PACKET_FLUSHED` and `PACKET_AT_FRONT` from RDP header compression are honored.

use core::cell::RefCell;

use crate::rdp::client_info::CompressionType;
use crate::rdp::headers::CompressionFlags;

#[derive(Debug, Clone, Copy)]
pub struct MppcConfig {
    pub history_size: usize, // 8192 (K8) or 65536 (K64)
    pub rdp5: bool,          // true for K64 (RDP5), false for K8 (RDP4)
}

impl From<CompressionType> for MppcConfig {
    fn from(ct: CompressionType) -> Self {
        match ct {
            CompressionType::K8 => MppcConfig {
                history_size: 8192,
                rdp5: false,
            },
            CompressionType::K64 | CompressionType::Rdp6 | CompressionType::Rdp61 => MppcConfig {
                history_size: 65536,
                rdp5: true,
            },
        }
    }
}

#[derive(Debug)]
pub struct MppcDecompressor {
    history: Vec<u8>,
    write_pos: usize,
    cfg: MppcConfig,
}

impl MppcDecompressor {
    pub fn new(cfg: MppcConfig) -> Self {
        let mut history = vec![0u8; cfg.history_size];
        // Initialized to zeroes per spec
        history.fill(0);
        Self {
            history,
            write_pos: 0,
            cfg,
        }
    }

    pub fn reset(&mut self, cfg: MppcConfig) {
        if self.history.len() != cfg.history_size {
            self.history.resize(cfg.history_size, 0);
        }
        self.history.fill(0);
        self.write_pos = 0;
        self.cfg = cfg;
    }

    fn at_front(&mut self) {
        // Move write pointer to the beginning of the dictionary
        self.write_pos = 0;
    }

    fn flush(&mut self) {
        self.history.fill(0);
        self.write_pos = 0;
    }

    /// Decompresses a single MPPC-compressed PDU payload using the current history.
    ///
    /// If `flags` does not include COMPRESSED, returns the input as-is (no copy).
    pub fn decompress(
        &mut self,
        flags: CompressionFlags,
        ctype: CompressionType,
        input: &[u8],
    ) -> Result<Vec<u8>, &'static str> {
        // Update configuration if compression type changes between PDUs
        let desired_cfg: MppcConfig = ctype.into();
        if self.cfg.history_size != desired_cfg.history_size || self.cfg.rdp5 != desired_cfg.rdp5 {
            self.reset(desired_cfg);
        }

        if flags.contains(CompressionFlags::AT_FRONT) {
            self.at_front();
        }
        if flags.contains(CompressionFlags::FLUSHED) {
            self.flush();
        }

        if !flags.contains(CompressionFlags::COMPRESSED) {
            return Ok(input.to_vec());
        }

        let mut br = BitReader::new(input);
        let start_pos = self.write_pos;

        // safety bounds
        let end_index = self.history.len() - 1;

        while br.bits_remaining() >= 8 {
            let acc = br.peek32();

            // Literal < 0x80: leading 0 + 7 bits of value
            if (acc & 0x8000_0000) == 0 {
                let lit = ((acc & 0x7F00_0000) >> 24) as u8;
                if self.write_pos > end_index { return Err("history overflow"); }
                self.history[self.write_pos] = lit;
                self.write_pos += 1;
                br.shift(8);
                continue;
            }
            // Literal >= 0x80: bits 10 + 7 bits of (value - 0x80)
            if (acc & 0xC000_0000) == 0x8000_0000 {
                let lit = (((acc & 0x3F80_0000) >> 23) as u8).wrapping_add(0x80);
                if self.write_pos > end_index { return Err("history overflow"); }
                self.history[self.write_pos] = lit;
                self.write_pos += 1;
                br.shift(9);
                continue;
            }

            // Copy tuple: decode offset first
            let mut copy_offset: usize;
            if self.cfg.rdp5 {
                // RDP5 variant (K64)
                if (acc & 0xF800_0000) == 0xF800_0000 {
                    // 11111 + 6 bits
                    copy_offset = ((acc >> 21) & 0x3F) as usize;
                    br.shift(11);
                } else if (acc & 0xF800_0000) == 0xF000_0000 {
                    // 11110 + 8 bits of (off-64)
                    copy_offset = (((acc >> 19) & 0xFF) as usize) + 64;
                    br.shift(13);
                } else if (acc & 0xF000_0000) == 0xE000_0000 {
                    // 1110 + 11 bits of (off-320)
                    copy_offset = (((acc >> 17) & 0x7FF) as usize) + 320;
                    br.shift(15);
                } else if (acc & 0xE000_0000) == 0xC000_0000 {
                    // 110 + 16 bits of (off-2368)
                    copy_offset = (((acc >> 13) & 0xFFFF) as usize) + 2368;
                    br.shift(19);
                } else {
                    return Err("invalid copy offset (rdp5)");
                }
            } else {
                // RDP4 variant (K8)
                if (acc & 0xF000_0000) == 0xF000_0000 {
                    // 1111 + 6 bits
                    copy_offset = ((acc >> 22) & 0x3F) as usize;
                    br.shift(10);
                } else if (acc & 0xF000_0000) == 0xE000_0000 {
                    // 1110 + 8 bits (off-64)
                    copy_offset = (((acc >> 20) & 0xFF) as usize) + 64;
                    br.shift(12);
                } else if (acc & 0xE000_0000) == 0xC000_0000 {
                    // 110 + 13 bits (off-320)
                    copy_offset = (((acc >> 16) & 0x1FFF) as usize) + 320;
                    br.shift(16);
                } else {
                    return Err("invalid copy offset (rdp4)");
                }
            }

            // Decode length-of-match
            let acc = br.peek32();
            let length: usize;
            if (acc & 0x8000_0000) == 0 {
                length = 3;
                br.shift(1);
            } else if (acc & 0xC000_0000) == 0x8000_0000 {
                length = (((acc >> 28) & 0x3) + 0x4) as usize; // 4..7
                br.shift(4);
            } else if (acc & 0xE000_0000) == 0xC000_0000 {
                length = (((acc >> 26) & 0x7) + 0x8) as usize; // 8..15
                br.shift(6);
            } else if (acc & 0xF000_0000) == 0xE000_0000 {
                length = (((acc >> 24) & 0xF) + 0x10) as usize; // 16..31
                br.shift(8);
            } else if (acc & 0xF800_0000) == 0xF000_0000 {
                length = (((acc >> 22) & 0x1F) + 0x20) as usize; // 32..63
                br.shift(10);
            } else if (acc & 0xFC00_0000) == 0xF800_0000 {
                length = (((acc >> 20) & 0x3F) + 0x40) as usize; // 64..127
                br.shift(12);
            } else if (acc & 0xFE00_0000) == 0xFC00_0000 {
                length = (((acc >> 18) & 0x7F) + 0x80) as usize; // 128..255
                br.shift(14);
            } else if (acc & 0xFF00_0000) == 0xFE00_0000 {
                length = (((acc >> 16) & 0xFF) + 0x100) as usize; // 256..511
                br.shift(16);
            } else if (acc & 0xFF80_0000) == 0xFF00_0000 {
                length = (((acc >> 14) & 0x1FF) + 0x200) as usize; // 512..1023
                br.shift(18);
            } else if (acc & 0xFFC0_0000) == 0xFF80_0000 {
                length = (((acc >> 12) & 0x3FF) + 0x400) as usize; // 1024..2047
                br.shift(20);
            } else if (acc & 0xFFE0_0000) == 0xFFC0_0000 {
                length = (((acc >> 10) & 0x7FF) + 0x800) as usize; // 2048..4095
                br.shift(22);
            } else if (acc & 0xFFF0_0000) == 0xFFE0_0000 {
                length = (((acc >> 8) & 0xFFF) + 0x1000) as usize; // 4096..8191
                br.shift(24);
            } else if ((acc & 0xFFFE_0000) == 0xFFFC_0000) && self.cfg.rdp5 {
                // 111111111111110 + 15 bits => 32768..65535 (RDP5-only)
                length = (((acc >> 2) & 0x7FFF) + 0x8000) as usize;
                br.shift(30);
            } else {
                return Err("invalid length-of-match");
            }

            // Perform the copy from history
            if self.write_pos + length - 1 > end_index {
                return Err("history overflow");
            }
            let hist_mask = if self.cfg.rdp5 { self.history.len() - 1 } else { 0x1FFF };
            let mut src_idx = (self.write_pos + self.history.len() - copy_offset) & hist_mask;
            for _ in 0..length {
                let b = self.history[src_idx];
                self.history[self.write_pos] = b;
                self.write_pos += 1;
                src_idx = (src_idx + 1) & hist_mask;
            }
        }

        Ok(self.history[start_pos..self.write_pos].to_vec())
    }
}

/// Bitstream reader that exposes a 32-bit big-endian window for prefix decoding.
struct BitReader<'a> {
    data: &'a [u8],
    byte: usize,
    bit: u8, // 0..7 (number of bits already consumed in current byte)
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, byte: 0, bit: 0 }
    }

    fn bits_remaining(&self) -> usize {
        (self.data.len().saturating_sub(self.byte)) * 8 - usize::from(self.bit)
    }

    fn peek32(&self) -> u32 {
        // Read 5 bytes and align to current bit position; produce the top 32 bits.
        let b0 = *self.data.get(self.byte).unwrap_or(&0);
        let b1 = *self.data.get(self.byte + 1).unwrap_or(&0);
        let b2 = *self.data.get(self.byte + 2).unwrap_or(&0);
        let b3 = *self.data.get(self.byte + 3).unwrap_or(&0);
        let b4 = *self.data.get(self.byte + 4).unwrap_or(&0);
        let val = ((b0 as u64) << 32)
            | ((b1 as u64) << 24)
            | ((b2 as u64) << 16)
            | ((b3 as u64) << 8)
            | (b4 as u64);
        // Align so that the MSB of the returned u32 is the next bit in stream
        ((val << self.bit) >> 8) as u32
    }

    fn shift(&mut self, n: usize) {
        let total = usize::from(self.bit) + n;
        self.byte += total / 8;
        self.bit = (total % 8) as u8;
    }
}

thread_local! {
    // One global (per-thread) decompressor state for slow-path Share Data compression.
    static GLOBAL_MPPC: RefCell<MppcDecompressor> = RefCell::new(MppcDecompressor::new(MppcConfig { history_size: 8192, rdp5: false }));
}

/// Convenience: access the global MPPC state and run decompression.
pub fn global_decompress(
    flags: CompressionFlags,
    ctype: CompressionType,
    input: &[u8],
) -> Result<Vec<u8>, &'static str> {
    GLOBAL_MPPC.with(|m| m.borrow_mut().decompress(flags, ctype, input))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a compressed stream consisting only of literals < 0x80.
    fn encode_literals(values: &[u8]) -> Vec<u8> {
        // For each literal v (< 0x80), encoding is: 0 bit followed by lower 7 bits of v.
        // Pack these bits MSB-first into bytes.
        let mut out = Vec::new();
        let mut bitbuf: u64 = 0;
        let mut bitlen: usize = 0;
        for &v in values {
            let val = (v & 0x7F) as u64;
            let pattern: u64 = (0u64 << 7) | val; // 8 bits: 0 + 7 bits
            bitbuf = (bitbuf << 8) | pattern;
            bitlen += 8;
            while bitlen >= 8 {
                let byte = (bitbuf >> (bitlen - 8)) as u8;
                out.push(byte);
                bitlen -= 8;
                bitbuf &= (1u64 << bitlen).wrapping_sub(1);
            }
        }
        if bitlen > 0 {
            out.push((bitbuf << (8 - bitlen)) as u8);
        }
        out
    }

    #[test]
    fn mppc_literal_only_decompresses() {
        let data = b"Hello, MPPC!";
        let compressed = encode_literals(data);

        let mut dec = MppcDecompressor::new(CompressionType::K8.into());
        let out = dec
            .decompress(CompressionFlags::COMPRESSED, CompressionType::K8, &compressed)
            .unwrap();
        assert_eq!(out, data);
    }
}
