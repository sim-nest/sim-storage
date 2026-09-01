//! Logical-key blinding for physical backend names.

use ring::hmac;

/// Produce a lane-separated, keyed physical name.
pub(crate) fn blind_key(key: &[u8; 32], lane: &[u8], logical: &str) -> String {
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut input = b"sim-table-sealed/key/v1\0".to_vec();
    input.extend_from_slice(&(lane.len() as u64).to_be_bytes());
    input.extend_from_slice(lane);
    input.extend_from_slice(logical.as_bytes());
    hex(hmac::sign(&hmac_key, &input).as_ref())
}

/// Produce the opaque prefix used to recognize entries belonging to one lane.
pub(crate) fn blind_lane(key: &[u8; 32], lane: &[u8]) -> String {
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut input = b"sim-table-sealed/lane/v1\0".to_vec();
    input.extend_from_slice(&(lane.len() as u64).to_be_bytes());
    input.extend_from_slice(lane);
    hex(hmac::sign(&hmac_key, &input).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}
