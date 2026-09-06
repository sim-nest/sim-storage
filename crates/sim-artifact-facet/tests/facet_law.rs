use sim_artifact_facet::*;

// conformance: singular pure role-safe artifact facet merge law.

fn id<K: FacetIdKind>(value: &str) -> FacetSemanticId<K> {
    FacetSemanticId::from_text(value).unwrap()
}

fn spec(policy: MergePolicy) -> ArtifactFacet {
    ArtifactFacet::new(
        id("repo/file"),
        id("owner/crate"),
        RegionSelector::ExactPath("crates/demo/src/lib.rs".into()),
        id("projection/file"),
        policy,
        id("disclosure/public"),
        RegionOwnership::Authored,
    )
    .unwrap()
}

fn file(text: &str) -> PortableImage {
    PortableImage::file(text.as_bytes(), 0o644).unwrap()
}

#[test]
fn exact_cases_are_idempotent_and_unchanged_retains_observation() {
    let spec = spec(MergePolicy::Exact);
    let base = BaseImage::check(&spec, file("base\n")).unwrap();
    let observed = ObservedImage::check(&spec, file("intended\n")).unwrap();
    let intended = IntendedImage::check(&spec, file("intended\n")).unwrap();
    assert_eq!(
        merge_facet(&spec, &base, &observed, &intended).unwrap(),
        MergeOutcome::AlreadyTrue
    );

    let base = BaseImage::check(&spec, file("base\n")).unwrap();
    let observed = ObservedImage::check(&spec, file("foreign\n")).unwrap();
    let retained = observed.id().clone();
    let intended = IntendedImage::check(&spec, file("base\n")).unwrap();
    assert_eq!(
        merge_facet(&spec, &base, &observed, &intended).unwrap(),
        MergeOutcome::Unchanged { retained }
    );
}

#[test]
fn linewise_merge_preserves_disjoint_edits() {
    let spec = spec(MergePolicy::Linewise {
        max_lines: 16,
        max_cells: 1_024,
    });
    let base = BaseImage::check(&spec, file("one\ntwo\nthree\n")).unwrap();
    let observed = ObservedImage::check(&spec, file("ONE\ntwo\nthree\n")).unwrap();
    let intended = IntendedImage::check(&spec, file("one\ntwo\nTHREE\n")).unwrap();
    let MergeOutcome::Merged { postimage } =
        merge_facet(&spec, &base, &observed, &intended).unwrap()
    else {
        panic!("disjoint changes did not merge");
    };
    assert_eq!(postimage.image(), &file("ONE\ntwo\nTHREE\n"));
}

#[test]
fn stale_overlapping_edits_conflict_symmetrically() {
    let spec = spec(MergePolicy::Linewise {
        max_lines: 16,
        max_cells: 1_024,
    });
    let base_bytes = file("one\ntwo\nthree\n");
    let left = file("one\nLEFT\nthree\n");
    let right = file("one\nRIGHT\nthree\n");
    let run = |observed: PortableImage, intended: PortableImage| {
        merge_facet(
            &spec,
            &BaseImage::check(&spec, base_bytes.clone()).unwrap(),
            &ObservedImage::check(&spec, observed).unwrap(),
            &IntendedImage::check(&spec, intended).unwrap(),
        )
        .unwrap()
    };
    assert_eq!(
        run(left.clone(), right.clone()),
        MergeOutcome::Conflict {
            reason: ConflictReason::Overlap
        }
    );
    assert_eq!(
        run(right, left),
        MergeOutcome::Conflict {
            reason: ConflictReason::Overlap
        }
    );
}

#[test]
fn generated_owner_region_and_work_bounds_fail_closed() {
    assert_eq!(
        ArtifactFacet::new(
            id("repo/file"),
            id("owner/authored"),
            RegionSelector::ExactPath("src/lib.rs".into()),
            id("projection/file"),
            MergePolicy::Exact,
            id("disclosure/public"),
            RegionOwnership::Generated {
                generator: id("owner/generator")
            },
        ),
        Err(FacetError::WrongOwner)
    );
    let spec = spec(MergePolicy::Linewise {
        max_lines: 2,
        max_cells: 4,
    });
    let outcome = merge_facet(
        &spec,
        &BaseImage::check(&spec, file("a\nb\nc\n")).unwrap(),
        &ObservedImage::check(&spec, file("A\nb\nc\n")).unwrap(),
        &IntendedImage::check(&spec, file("a\nb\nC\n")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Conflict {
            reason: ConflictReason::WorkBound
        }
    );
}

#[test]
fn line_region_refuses_out_of_region_content_and_mode() {
    let spec = ArtifactFacet::new(
        id("repo/file"),
        id("owner/crate"),
        RegionSelector::LineRange {
            path: "src/lib.rs".into(),
            start: 1,
            end: 2,
        },
        id("projection/file"),
        MergePolicy::Linewise {
            max_lines: 16,
            max_cells: 1_024,
        },
        id("disclosure/public"),
        RegionOwnership::Authored,
    )
    .unwrap();
    let base_image = file("one\ntwo\nthree\n");
    let base = BaseImage::check(&spec, base_image.clone()).unwrap();
    let observed = ObservedImage::check(&spec, base_image).unwrap();
    let outside = IntendedImage::check(&spec, file("ONE\ntwo\nthree\n")).unwrap();
    assert_eq!(
        merge_facet(&spec, &base, &observed, &outside),
        Err(FacetError::OutOfRegion)
    );
    let mode = IntendedImage::check(
        &spec,
        PortableImage::file("one\nTWO\nthree\n", 0o755).unwrap(),
    )
    .unwrap();
    assert_eq!(
        merge_facet(&spec, &base, &observed, &mode),
        Err(FacetError::OutOfRegion)
    );
}

#[test]
fn transition_classifier_is_the_shared_whole_file_law() {
    let base = file("base");
    let intended = file("post");
    assert_eq!(
        classify_transition(&base, &intended, &base).unwrap(),
        TransitionState::Base
    );
    assert_eq!(
        classify_transition(&base, &intended, &intended).unwrap(),
        TransitionState::Intended
    );
    assert_eq!(
        classify_transition(&base, &intended, &file("foreign")).unwrap(),
        TransitionState::Foreign
    );
}
