// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! DeviceSQL string encoding used throughout the PDB.
//!
//! Pioneer's embedded database stores strings in one of three forms,
//! distinguished by the first byte:
//!
//! - **Short ASCII** (≤ 126 bytes): a single length/flag byte
//!   `((len + 1) << 1) | 1` followed by the raw ASCII bytes.
//! - **Long ASCII** (> 126 bytes): flag `0x40`, a little-endian `u16`
//!   total length (`content + 4`), a padding byte, then the bytes.
//! - **Long UTF-16LE** (any non-ASCII content): flag `0x90`, a
//!   little-endian `u16` total length (`content_bytes + 4`), a padding
//!   byte, then UTF-16LE code units.

const SHORT_ASCII: u8 = 0x01;
const LONG_ASCII: u8 = 0x40;
const LONG_UTF16LE: u8 = 0x90;

/// Encode a string into its DeviceSQL byte form. An empty string
/// encodes to the single byte `0x03` (short ASCII, length 0).
pub fn encode(value: &str) -> Vec<u8> {
    if !value.is_ascii() {
        return encode_utf16(value);
    }

    let bytes = value.as_bytes();
    let len = bytes.len();
    if len <= 126 {
        let mut out = Vec::with_capacity(1 + len);
        out.push((((len + 1) << 1) as u8) | SHORT_ASCII);
        out.extend_from_slice(bytes);
        out
    } else {
        let mut out = Vec::with_capacity(4 + len);
        out.push(LONG_ASCII);
        out.extend_from_slice(&((len + 4) as u16).to_le_bytes());
        out.push(0);
        out.extend_from_slice(bytes);
        out
    }
}

fn encode_utf16(value: &str) -> Vec<u8> {
    let units: Vec<u16> = value.encode_utf16().collect();
    let byte_len = units.len() * 2;
    let mut out = Vec::with_capacity(4 + byte_len);
    out.push(LONG_UTF16LE);
    out.extend_from_slice(&((byte_len + 4) as u16).to_le_bytes());
    out.push(0);
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ascii() {
        let encoded = encode("Hello");
        assert_eq!(encoded[0], 0x0D); // ((5 + 1) << 1) | 1
        assert_eq!(&encoded[1..], b"Hello");
    }

    #[test]
    fn empty_string() {
        assert_eq!(encode(""), vec![0x03]);
    }

    #[test]
    fn utf16_for_non_ascii() {
        let encoded = encode("Déjà Vu");
        assert_eq!(encoded[0], 0x90);
        assert_eq!(encoded[1], 0x12); // 7 chars * 2 + 4
        assert_eq!(encoded[2], 0x00);
        assert_eq!(encoded[3], 0x00);
        assert_eq!(&encoded[4..6], &[0x44, 0x00]); // 'D'
        assert_eq!(&encoded[6..8], &[0xE9, 0x00]); // 'é'
    }

    #[test]
    fn long_ascii_over_126_bytes() {
        let value = "a".repeat(130);
        let encoded = encode(&value);
        assert_eq!(encoded[0], LONG_ASCII);
        assert_eq!(u16::from_le_bytes([encoded[1], encoded[2]]), 134);
        assert_eq!(encoded[3], 0x00);
        assert_eq!(encoded.len(), 4 + 130);
    }
}
