//! Plaintext payload coding. Payloads contain the logical key to make listing reversible.

use sim_kernel::{CodecId, Expr};
use zeroize::Zeroizing;

use crate::SealedError;

pub(crate) fn encode(
    key: &str,
    expr: &Expr,
    max: usize,
) -> Result<Zeroizing<Vec<u8>>, SealedError> {
    let value =
        sim_codec::encode_portable(CodecId(0), expr).map_err(|_| SealedError::UnsupportedValue)?;
    let key_len = u32::try_from(key.len()).map_err(|_| SealedError::Oversized)?;
    let mut out = Zeroizing::new(Vec::with_capacity(4 + key.len() + value.len()));
    out.extend_from_slice(&key_len.to_be_bytes());
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(value.as_bytes());
    if out.len() > max {
        return Err(SealedError::Oversized);
    }
    Ok(out)
}

pub(crate) fn decode(bytes: &[u8], max: usize) -> Result<(String, Expr), SealedError> {
    if bytes.len() > max || bytes.len() < 4 {
        return Err(SealedError::Authentication);
    }
    let key_len = u32::from_be_bytes(
        bytes[..4]
            .try_into()
            .map_err(|_| SealedError::Authentication)?,
    ) as usize;
    let split = 4usize
        .checked_add(key_len)
        .ok_or(SealedError::Authentication)?;
    let key = std::str::from_utf8(bytes.get(4..split).ok_or(SealedError::Authentication)?)
        .map_err(|_| SealedError::Authentication)?
        .to_owned();
    let text = std::str::from_utf8(bytes.get(split..).ok_or(SealedError::Authentication)?)
        .map_err(|_| SealedError::Authentication)?;
    let expr =
        sim_codec::decode_portable(CodecId(0), text).map_err(|_| SealedError::Authentication)?;
    Ok((key, expr))
}
