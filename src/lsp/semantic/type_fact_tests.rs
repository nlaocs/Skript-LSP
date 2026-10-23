use super::*;

fn value(class: &str, maybe_unset: bool) -> InferredType {
    InferredType::Values {
        classes: BTreeSet::from([class.into()]),
        multiple: false,
        maybe_unset,
        support: TypeSupport::inferred(),
    }
}

#[test]
fn presence_completeness_and_certainty_have_separate_proof_requirements() {
    let present = value("java.lang.Number", false);
    let nullable = value("java.lang.Number", true);
    assert!(present.proven_types().is_some());
    assert!(present.proven_present().is_some());
    // Type-preserving clone evidence may be usable without nonempty evidence.
    assert!(nullable.proven_types().is_some());
    assert!(nullable.proven_present().is_none());
    for (certainty, complete) in [
        (Certainty::Heuristic, true),
        (Certainty::Heuristic, false),
        (Certainty::Inferred, false),
        (Certainty::Unknown, true),
    ] {
        let mut candidate = present.clone();
        if let InferredType::Values { support, .. } = &mut candidate {
            support.certainty = certainty;
            support.candidates_complete = complete;
        }
        assert!(candidate.proven_types().is_none(), "{candidate:?}");
        assert!(candidate.proven_present().is_none(), "{candidate:?}");
    }
}

#[test]
fn a_complete_union_is_inferred_and_not_a_heuristic() {
    let joined = value("java.lang.Number", false).join(&value("java.lang.String", false));
    assert_eq!(joined.proven_present().unwrap().0.len(), 2);
    let InferredType::Values {
        support,
        maybe_unset,
        ..
    } = joined
    else {
        panic!()
    };
    assert_eq!(support.certainty, Certainty::Inferred);
    assert!(support.candidates_complete);
    assert!(!maybe_unset);
}

#[test]
fn unknown_current_branch_retains_only_incomplete_candidates() {
    let number = value("java.lang.Number", false);
    let joined = number.join(&InferredType::Unknown);
    assert_eq!(joined, InferredType::Unknown.join(&number));
    let InferredType::Values {
        classes,
        support,
        maybe_unset,
        ..
    } = &joined
    else {
        panic!()
    };
    assert_eq!(classes, &BTreeSet::from(["java.lang.Number".into()]));
    assert_eq!(support.certainty, Certainty::Inferred);
    assert!(!support.candidates_complete);
    assert!(*maybe_unset);
    assert!(joined.proven_present().is_none());
    assert!(joined.join(&number).proven_types().is_none());
}

#[test]
fn unset_path_does_not_make_the_known_candidate_set_incomplete() {
    let joined = value("java.lang.Number", false).join(&InferredType::unset());
    assert!(joined.proven_types().is_some());
    assert!(joined.proven_present().is_none());
    let InferredType::Values {
        support,
        maybe_unset,
        ..
    } = joined
    else {
        panic!()
    };
    assert!(support.candidates_complete);
    assert_eq!(support.certainty, Certainty::Inferred);
    assert!(maybe_unset);
}

#[test]
fn assignments_and_joins_cannot_promote_heuristic_candidates() {
    let mut heuristic = value("java.lang.String", false);
    if let InferredType::Values { support, .. } = &mut heuristic {
        support.certainty = Certainty::Heuristic;
    }
    let assigned = heuristic.with_evidence(Evidence::Assignment);
    let joined = assigned
        .join(&value("java.lang.Number", false))
        .with_evidence(Evidence::ControlFlowJoin);
    let InferredType::Values { support, .. } = &joined else {
        panic!()
    };
    assert_eq!(support.certainty, Certainty::Heuristic);
    assert_eq!(
        support.evidence,
        BTreeSet::from([
            Evidence::AcceptedResult,
            Evidence::Assignment,
            Evidence::ControlFlowJoin
        ])
    );
    assert!(joined.proven_types().is_none());
}

#[test]
fn heuristic_or_partial_types_cannot_certify_clone_or_read_preservation() {
    let catalog = crate::lsp::test_support::multi_addon_catalog();
    let read = VariableRead {
        version: 1,
        requested_types: vec!["java.lang.Number".into()],
        may_refresh_player: false,
        no_refresh_types: Vec::new(),
    };
    let mut change = Change {
        version: 2,
        mode: "set".into(),
        target_capture: 0,
        value_capture: Some(1),
        target_only: true,
        identity_delta: false,
        identity_delta_types: vec!["java.lang.Number".into()],
    };
    for heuristic in [false, true] {
        let mut candidate = value("java.lang.Number", false);
        if let InferredType::Values { support, .. } = &mut candidate {
            if heuristic {
                support.certainty = Certainty::Heuristic;
            } else {
                support.candidates_complete = false;
            }
        }
        assert!(!identity_delta(&change, &candidate));
        assert!(!read_preserves_type_facts(&read, &candidate, &catalog));
        change.identity_delta = true;
        assert!(identity_delta(&change, &candidate));
        change.identity_delta = false;
    }
}
