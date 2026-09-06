//! Pure exact comparison and bounded linewise three-way merge.

use crate::{
    ArtifactFacet, BaseImage, CheckedPostimage, IntendedImage, MergePolicy, ObservedImage,
    ObservedImageId, PortableImage, RegionSelector,
};

/// Pure state of an observed image relative to an exact transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionState {
    /// Base and intended are equal and the observation retains that image.
    Unchanged,
    /// Observation still equals the base.
    Base,
    /// Observation already equals the intended image.
    Intended,
    /// Observation is neither base nor intended.
    Foreign,
}

/// Shared exact comparison used by mutation and packet consumers.
pub fn classify_transition(
    base: &PortableImage,
    intended: &PortableImage,
    observed: &PortableImage,
) -> Result<TransitionState, FacetError> {
    base.validate()?;
    intended.validate()?;
    observed.validate()?;
    Ok(if base == intended {
        TransitionState::Unchanged
    } else if observed == intended {
        TransitionState::Intended
    } else if observed == base {
        TransitionState::Base
    } else {
        TransitionState::Foreign
    })
}

/// Checked result of the pure facet law.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// Observation already has the intended content.
    AlreadyTrue,
    /// Proposal changes nothing, so the observed foreign edit is retained.
    Unchanged {
        /// Exact observed image retained.
        retained: ObservedImageId,
    },
    /// Intended content applies directly because observation still equals base.
    Apply {
        /// Checked postimage.
        postimage: CheckedPostimage,
    },
    /// Disjoint changes were preserved by the registered pure policy.
    Merged {
        /// Checked merged postimage.
        postimage: CheckedPostimage,
    },
    /// Both sides changed overlapping content or the selected policy disallows merge.
    Conflict {
        /// Stable refusal reason.
        reason: ConflictReason,
    },
}

/// Stable pure conflict class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictReason {
    /// Exact policy rejects all divergent two-sided changes.
    BothChanged,
    /// One side creates or removes a file while the other changes it.
    Presence,
    /// Both sides changed portable mode differently.
    Mode,
    /// A changed image is not UTF-8 under linewise policy.
    NonText,
    /// Line count or comparison work exceeds policy.
    WorkBound,
    /// Changed line ranges overlap.
    Overlap,
}

/// Applies the singular pure facet law.
///
/// Base, observed, and intended values use distinct types. Swapping roles does
/// not compile:
///
/// ```compile_fail
/// # use sim_artifact_facet::*;
/// # fn wrong(spec: &ArtifactFacet, base: &BaseImage, intended: &IntendedImage) {
/// let _ = merge_facet(spec, base, intended, intended);
/// # }
/// ```
pub fn merge_facet(
    spec: &ArtifactFacet,
    base: &BaseImage,
    observed: &ObservedImage,
    intended: &IntendedImage,
) -> Result<MergeOutcome, FacetError> {
    if base.facet() != spec.id() || observed.facet() != spec.id() || intended.facet() != spec.id() {
        return Err(FacetError::WrongFacet);
    }
    verify_intended_region(spec, base.image(), intended.image())?;
    match classify_transition(base.image(), intended.image(), observed.image())? {
        TransitionState::Intended => return Ok(MergeOutcome::AlreadyTrue),
        TransitionState::Unchanged => {
            return Ok(MergeOutcome::Unchanged {
                retained: observed.id().clone(),
            });
        }
        TransitionState::Base => {
            return Ok(MergeOutcome::Apply {
                postimage: CheckedPostimage::new(spec, intended.image().clone())?,
            });
        }
        TransitionState::Foreign => {}
    }
    match spec.merge {
        MergePolicy::Exact => Ok(MergeOutcome::Conflict {
            reason: ConflictReason::BothChanged,
        }),
        MergePolicy::Linewise {
            max_lines,
            max_cells,
        } => merge_linewise(
            spec,
            base.image(),
            observed.image(),
            intended.image(),
            max_lines,
            max_cells,
        ),
    }
}

fn verify_intended_region(
    spec: &ArtifactFacet,
    base: &PortableImage,
    intended: &PortableImage,
) -> Result<(), FacetError> {
    let RegionSelector::LineRange { start, end, .. } = spec.region else {
        return Ok(());
    };
    let (Some(base_bytes), Some(intended_bytes)) = (&base.bytes, &intended.bytes) else {
        return Err(FacetError::OutOfRegion);
    };
    if base.mode != intended.mode {
        return Err(FacetError::OutOfRegion);
    }
    let base_text = std::str::from_utf8(base_bytes).map_err(|_| FacetError::OutOfRegion)?;
    let intended_text = std::str::from_utf8(intended_bytes).map_err(|_| FacetError::OutOfRegion)?;
    let base_lines = lines(base_text);
    let intended_lines = lines(intended_text);
    let start = start as usize;
    let end = end as usize;
    if end > base_lines.len() {
        return Err(FacetError::OutOfRegion);
    }
    let suffix_len = base_lines.len() - end;
    if intended_lines.len() < start.saturating_add(suffix_len)
        || intended_lines[..start] != base_lines[..start]
        || intended_lines[intended_lines.len() - suffix_len..] != base_lines[end..]
    {
        return Err(FacetError::OutOfRegion);
    }
    Ok(())
}

fn merge_linewise(
    spec: &ArtifactFacet,
    base: &PortableImage,
    observed: &PortableImage,
    intended: &PortableImage,
    max_lines: u32,
    max_cells: u64,
) -> Result<MergeOutcome, FacetError> {
    let (Some(base_bytes), Some(observed_bytes), Some(intended_bytes)) =
        (&base.bytes, &observed.bytes, &intended.bytes)
    else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::Presence,
        });
    };
    let mode = merge_mode(
        base.mode.expect("validated present image"),
        observed.mode.expect("validated present image"),
        intended.mode.expect("validated present image"),
    );
    let Some(mode) = mode else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::Mode,
        });
    };
    let Ok(base_text) = std::str::from_utf8(base_bytes) else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::NonText,
        });
    };
    let Ok(observed_text) = std::str::from_utf8(observed_bytes) else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::NonText,
        });
    };
    let Ok(intended_text) = std::str::from_utf8(intended_bytes) else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::NonText,
        });
    };
    let base_lines = lines(base_text);
    let observed_lines = lines(observed_text);
    let intended_lines = lines(intended_text);
    if [base_lines.len(), observed_lines.len(), intended_lines.len()]
        .into_iter()
        .any(|count| count > max_lines as usize)
    {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::WorkBound,
        });
    }
    let Some(observed_changes) = changes(&base_lines, &observed_lines, max_cells) else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::WorkBound,
        });
    };
    let Some(intended_changes) = changes(&base_lines, &intended_lines, max_cells) else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::WorkBound,
        });
    };
    let Some(changes) = combine_changes(observed_changes, intended_changes) else {
        return Ok(MergeOutcome::Conflict {
            reason: ConflictReason::Overlap,
        });
    };
    let mut merged = Vec::new();
    let mut cursor = 0usize;
    for change in changes {
        merged.extend_from_slice(&base_lines[cursor..change.start]);
        merged.extend(change.replacement);
        cursor = change.end;
    }
    merged.extend_from_slice(&base_lines[cursor..]);
    let image = PortableImage::file(merged.concat().into_bytes(), mode)?;
    Ok(MergeOutcome::Merged {
        postimage: CheckedPostimage::new(spec, image)?,
    })
}

fn merge_mode(base: u32, observed: u32, intended: u32) -> Option<u32> {
    if observed == intended {
        Some(observed)
    } else if observed == base {
        Some(intended)
    } else if intended == base {
        Some(observed)
    } else {
        None
    }
}

fn lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split_inclusive('\n').map(str::to_owned).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    start: usize,
    end: usize,
    replacement: Vec<String>,
}

fn changes(base: &[String], side: &[String], max_cells: u64) -> Option<Vec<Change>> {
    let rows = base.len().checked_add(1)?;
    let columns = side.len().checked_add(1)?;
    if (rows as u64).checked_mul(columns as u64)? > max_cells {
        return None;
    }
    let mut lengths = vec![0u32; rows.checked_mul(columns)?];
    let at = |row: usize, column: usize| row * columns + column;
    for row in (0..base.len()).rev() {
        for column in (0..side.len()).rev() {
            lengths[at(row, column)] = if base[row] == side[column] {
                lengths[at(row + 1, column + 1)] + 1
            } else {
                lengths[at(row + 1, column)].max(lengths[at(row, column + 1)])
            };
        }
    }
    let mut matches = Vec::new();
    let (mut row, mut column) = (0usize, 0usize);
    while row < base.len() && column < side.len() {
        if base[row] == side[column] {
            matches.push((row, column));
            row += 1;
            column += 1;
        } else if lengths[at(row + 1, column)] >= lengths[at(row, column + 1)] {
            row += 1;
        } else {
            column += 1;
        }
    }
    matches.push((base.len(), side.len()));
    let mut changes = Vec::new();
    let (mut base_cursor, mut side_cursor) = (0usize, 0usize);
    for (base_match, side_match) in matches {
        if base_cursor != base_match || side_cursor != side_match {
            changes.push(Change {
                start: base_cursor,
                end: base_match,
                replacement: side[side_cursor..side_match].to_vec(),
            });
        }
        base_cursor = base_match.saturating_add(1).min(base.len());
        side_cursor = side_match.saturating_add(1).min(side.len());
    }
    Some(changes)
}

fn combine_changes(mut left: Vec<Change>, right: Vec<Change>) -> Option<Vec<Change>> {
    for candidate in right {
        if let Some(equal) = left.iter().find(|change| {
            change.start == candidate.start
                && change.end == candidate.end
                && change.replacement == candidate.replacement
        }) {
            let _ = equal;
            continue;
        }
        if left.iter().any(|change| overlaps(change, &candidate)) {
            return None;
        }
        left.push(candidate);
    }
    left.sort_by_key(|change| (change.start, change.end));
    Some(left)
}

fn overlaps(left: &Change, right: &Change) -> bool {
    match (left.start == left.end, right.start == right.end) {
        (true, true) => left.start == right.start,
        (true, false) => left.start >= right.start && left.start <= right.end,
        (false, true) => right.start >= left.start && right.start <= left.end,
        (false, false) => left.start < right.end && right.start < left.end,
    }
}

/// Typed pure facet refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FacetError {
    /// An image's presence, mode, and bytes disagree.
    InvalidImage,
    /// Permission bits are not portable.
    InvalidMode,
    /// Region selector is empty, unsafe, or inverted.
    InvalidRegion,
    /// Intended content changes bytes or mode outside its selected region.
    OutOfRegion,
    /// Merge policy has no usable bound.
    InvalidMergePolicy,
    /// A generated region named a different owner.
    WrongOwner,
    /// An image was checked for another facet.
    WrongFacet,
    /// Canonical identity construction failed.
    NoncanonicalIdentity,
    /// A bounded identity, image, or merge input exceeded policy.
    BoundExceeded(&'static str),
}

impl std::fmt::Display for FacetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FacetError {}
