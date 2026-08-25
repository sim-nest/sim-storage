//! Errors produced by the sealed table decorator.

use std::fmt;

/// A deliberately redacted sealed-storage failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SealedError {
    /// The configured grant is absent or revoked.
    GrantUnavailable,
    /// The object is malformed, unsupported, moved, or unauthentic.
    Authentication,
    /// A nonce was repeated by the injected source.
    NonceReuse,
    /// The configured bounded nonce-tracking budget was consumed.
    NonceBudgetExhausted,
    /// A configured or encoded size bound was exceeded.
    Oversized,
    /// The wrapped value is outside the portable data subset.
    UnsupportedValue,
}

impl fmt::Display for SealedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::GrantUnavailable => "sealed object grant unavailable",
            Self::Authentication => "sealed object authentication failed",
            Self::NonceReuse => "sealed object nonce reuse refused",
            Self::NonceBudgetExhausted => "sealed object nonce budget exhausted",
            Self::Oversized => "sealed object size limit exceeded",
            Self::UnsupportedValue => "sealed table accepts portable data values only",
        };
        f.write_str(text)
    }
}

impl std::error::Error for SealedError {}
