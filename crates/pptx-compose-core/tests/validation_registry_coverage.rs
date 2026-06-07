use pptx_compose_core::validation::{
    FINDING_COVERAGE, FINDING_REGISTRY, FindingCode, FindingCoverage,
};

#[test]
fn every_044_finding_has_producer_or_deferral() {
    assert_eq!(FINDING_REGISTRY.len(), 17);
    assert_eq!(FINDING_COVERAGE.len(), FINDING_REGISTRY.len());

    for (code, _, _) in FINDING_REGISTRY {
        let entries = coverage_entries(*code);
        assert_eq!(entries.len(), 1, "{code:?} must have one coverage entry");

        let entry = entries[0];
        let has_producer_coverage = entry.producer.is_some() && entry.producer_test.is_some();
        let has_deferral = entry.deferral.is_some();
        assert_ne!(
            has_producer_coverage, has_deferral,
            "{code:?} must have exactly one of producer coverage or explicit deferral"
        );

        if has_producer_coverage {
            assert!(entry.producer.unwrap_or_default().contains("::"));
            assert!(entry.producer_test.unwrap_or_default().contains("::"));
        }

        if let Some(deferral) = entry.deferral {
            assert!(!deferral.owner.is_empty());
            assert!(
                deferral.spec.contains("specs/"),
                "{code:?} deferral must cite a spec"
            );
            assert!(!deferral.reason.is_empty());
            assert!(
                entry.producer.is_none() && entry.producer_test.is_none(),
                "{code:?} deferral must not also name producer coverage"
            );
        }
    }

    for entry in FINDING_COVERAGE {
        assert!(
            FINDING_REGISTRY
                .iter()
                .any(|(code, _, _)| *code == entry.code),
            "{:?} is not present in the specs/044 registry",
            entry.code
        );
    }

    assert_has_runtime_producer(FindingCode::MediaContentTypeMismatch);
    assert_has_runtime_producer(FindingCode::UnreferencedMedia);
    assert_has_runtime_producer(FindingCode::UnresolvedRelationshipReference);
    assert_has_runtime_producer(FindingCode::DanglingCommentAuthorRef);
    assert_deferral(
        FindingCode::SlideOrderMismatch,
        "specs/050-roundtrip-invariants.md",
    );
    assert_deferral(FindingCode::OrphanPart, "specs/010");
}

fn coverage_entries(code: FindingCode) -> Vec<&'static FindingCoverage> {
    FINDING_COVERAGE
        .iter()
        .filter(|entry| entry.code == code)
        .collect()
}

fn assert_has_runtime_producer(code: FindingCode) {
    let entries = coverage_entries(code);
    let entry = entries
        .first()
        .copied()
        .expect("coverage entry must exist for runtime producer assertion");

    assert!(
        entry.producer.is_some() && entry.producer_test.is_some(),
        "{code:?} must have runtime producer coverage"
    );
    assert!(
        entry.deferral.is_none(),
        "{code:?} must not be satisfied by deferral"
    );
}

fn assert_deferral(code: FindingCode, spec_ref: &str) {
    let entries = coverage_entries(code);
    let entry = entries
        .first()
        .copied()
        .expect("coverage entry must exist for deferral assertion");
    let deferral = entry
        .deferral
        .expect("coverage entry must be an explicit deferral");

    assert!(
        deferral.spec.contains(spec_ref),
        "{code:?} deferral must cite {spec_ref}"
    );
}
