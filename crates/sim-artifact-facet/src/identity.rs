//! Canonical facet identity roles.

use sim_kernel::{ContentId, Datum, NumberLiteral, Symbol};
use std::{collections::BTreeSet, fmt, marker::PhantomData};

use crate::FacetError;

/// Exact domain tag for a semantic facet id role.
pub trait FacetIdKind {
    /// Domain included in the canonical preimage.
    const DOMAIN: &'static str;
}

/// Semantic id that cannot be reassigned to another facet role.
pub struct FacetSemanticId<K: FacetIdKind> {
    raw: ContentId,
    marker: PhantomData<fn() -> K>,
}

impl<K: FacetIdKind> Clone for FacetSemanticId<K> {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            marker: PhantomData,
        }
    }
}
impl<K: FacetIdKind> PartialEq for FacetSemanticId<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<K: FacetIdKind> Eq for FacetSemanticId<K> {}
impl<K: FacetIdKind> PartialOrd for FacetSemanticId<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K: FacetIdKind> Ord for FacetSemanticId<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}
impl<K: FacetIdKind> std::hash::Hash for FacetSemanticId<K> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}
impl<K: FacetIdKind> fmt::Debug for FacetSemanticId<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::DOMAIN).field(&self.raw).finish()
    }
}

impl<K: FacetIdKind> FacetSemanticId<K> {
    /// Constructs an id from every semantic field except the id itself.
    pub fn from_fields(fields: Vec<(Symbol, Datum)>) -> Result<Self, FacetError> {
        let mut seen = BTreeSet::new();
        if fields.iter().any(|(name, _)| !seen.insert(name)) {
            return Err(FacetError::NoncanonicalIdentity);
        }
        let raw = Datum::Node {
            tag: domain_symbol(K::DOMAIN)?,
            fields,
        }
        .content_id()
        .map_err(|_| FacetError::NoncanonicalIdentity)?;
        Ok(Self {
            raw,
            marker: PhantomData,
        })
    }

    /// Constructs an id from one nonempty stable name.
    pub fn from_text(value: &str) -> Result<Self, FacetError> {
        if value.is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
            return Err(FacetError::BoundExceeded("id text"));
        }
        Self::from_fields(vec![field("value", Datum::String(value.into()))?])
    }

    /// Returns the underlying canonical kernel identity without changing its role.
    pub const fn content_id(&self) -> &ContentId {
        &self.raw
    }

    pub(crate) fn datum(&self) -> Datum {
        content_id_datum(&self.raw)
    }
}

macro_rules! facet_ids {
    ($(($kind:ident, $alias:ident, $domain:literal, $doc:literal)),+ $(,)?) => {$ (
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $kind;
        impl FacetIdKind for $kind { const DOMAIN: &'static str = $domain; }
        #[doc = $doc]
        pub type $alias = FacetSemanticId<$kind>;
    )+};
}

facet_ids! {
    (ArtifactKind, ArtifactId, "artifact/facet-artifact-v1", "Identity of the complete containing artifact."),
    (OwnerKind, OwnerId, "artifact/facet-owner-v1", "Identity of the one authoritative owner."),
    (RegionKind, RegionId, "artifact/facet-region-v1", "Identity of the selected artifact region."),
    (ProjectionKind, ProjectionId, "artifact/facet-projection-v1", "Identity of the canonical image projection."),
    (MergePolicyKind, MergePolicyId, "artifact/facet-merge-policy-v1", "Identity of the registered pure merge policy."),
    (DisclosureKind, DisclosureDecisionId, "artifact/facet-disclosure-v1", "Identity of an externally supplied disclosure decision."),
    (FacetKind, FacetId, "artifact/facet-v1", "Identity of a complete facet specification."),
    (ImageKind, ImageId, "artifact/image-v1", "Identity of exact file presence, mode, and bytes."),
    (BaseImageKind, BaseImageId, "artifact/base-image-v1", "Role identity of the checked base image."),
    (ObservedImageKind, ObservedImageId, "artifact/observed-image-v1", "Role identity of the checked observed image."),
    (IntendedImageKind, IntendedImageId, "artifact/intended-image-v1", "Role identity of the checked intended image."),
    (PostimageKind, PostimageId, "artifact/checked-postimage-v1", "Role identity of a checked merge postimage."),
}

pub(crate) fn field(name: &str, value: Datum) -> Result<(Symbol, Datum), FacetError> {
    let name = Symbol::checked(name).map_err(|_| FacetError::NoncanonicalIdentity)?;
    Ok((Symbol::qualified("artifact", name.name), value))
}

pub(crate) fn number(value: u64) -> Datum {
    Datum::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "u64"),
        canonical: value.to_string(),
    })
}

fn domain_symbol(domain: &str) -> Result<Symbol, FacetError> {
    let (namespace, name) = domain
        .split_once('/')
        .ok_or(FacetError::NoncanonicalIdentity)?;
    if name.contains('/') || namespace.is_empty() || name.is_empty() {
        return Err(FacetError::NoncanonicalIdentity);
    }
    Ok(Symbol::qualified(namespace, name))
}

fn content_id_datum(id: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("artifact", "content-id-v1"),
        fields: vec![
            (
                Symbol::qualified("artifact", "algorithm"),
                Datum::Symbol(id.algorithm.clone()),
            ),
            (
                Symbol::qualified("artifact", "digest"),
                Datum::Bytes(id.bytes.to_vec()),
            ),
        ],
    }
}
