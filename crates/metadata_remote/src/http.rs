// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Shared HTTP helpers for the remote metadata providers.

/// Percent-encodes a query-string component, leaving only the RFC 3986
/// unreserved set (`A-Z a-z 0-9 - _ . ~`) literal and encoding every other byte
/// as `%XX`.
///
/// All three providers (MusicBrainz, LRClib, AcoustID) build their request URLs
/// by interpolating user-derived text into a query string, and each server
/// percent-decodes the value before parsing or matching. Encoding
/// conservatively here keeps reserved characters — notably the brackets and
/// colons of a MusicBrainz Lucene query — from being rejected by HTTP clients
/// that refuse raw reserved characters in a URL.
pub(crate) fn url_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        let safe = matches!(
            byte,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if safe {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::url_encode;

    #[test]
    fn unreserved_characters_pass_through() {
        assert_eq!(url_encode("abc-123.xyz"), "abc-123.xyz");
        assert_eq!(url_encode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn reserved_characters_are_percent_encoded() {
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("foo&bar"), "foo%26bar");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }
}
