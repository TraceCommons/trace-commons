//! Ethereum-style address decoding.
//!
//! Its own module, and deliberately outside the `receipt` feature, because it
//! is the one piece of the EIP-191 story that costs nothing. Decoding an
//! address is hex and a length check; *recovering* one from a signature is
//! secp256k1 and keccak, which is `k256` and `sha3` and nineteen transitive
//! crates that a client shipping inside a third-party agent harness should
//! only pay for if it actually verifies signatures.
//!
//! Splitting them means a client that pins a witness's signing address, and
//! compares it against the address a quote's report data names, needs no
//! curve implementation at all. That comparison is the check that says a
//! quote describes the machine that will sign; it is worth having on the
//! cheap side of the line.

/// Decode a `0x`-prefixed 20-byte hex address, in either case.
///
/// `None` for anything that is not exactly `0x` plus 40 hex characters. The
/// length is checked before the decode so that a longer string cannot be
/// truncated into a valid-looking address.
pub fn decode_address(s: &str) -> Option<[u8; 20]> {
    let body = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if body.len() != 40 {
        return None;
    }
    let bytes = hex::decode(body).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_zero_x_and_exactly_forty_hex_characters() {
        assert_eq!(
            decode_address("0x0102030405060708090a0b0c0d0e0f1011121314"),
            Some([
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20
            ])
        );
        // Case-insensitive in both the prefix and the body: the same 20 bytes,
        // not a second encoding.
        assert_eq!(
            decode_address("0XAABBCCDDEEFF00112233445566778899AABBCCDD"),
            decode_address("0xaabbccddeeff00112233445566778899aabbccdd")
        );

        // Collected rather than asserted in a loop, so the first accepted
        // case cannot hide every one after it.
        let accepted: Vec<&str> = [
            "",
            "0x",
            "0102030405060708090a0b0c0d0e0f1011121314",
            "0x0102030405060708090a0b0c0d0e0f10111213",
            "0x0102030405060708090a0b0c0d0e0f101112131415",
            "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "0x0102030405060708090a0b0c0d0e0f101112131 ",
        ]
        .into_iter()
        .filter(|value| decode_address(value).is_some())
        .collect();
        assert!(
            accepted.is_empty(),
            "malformed addresses accepted: {accepted:?}"
        );
    }
}
