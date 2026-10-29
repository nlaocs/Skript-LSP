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
fn declared_storage_bound_is_complete_but_does_not_mint_identity_or_presence() {
    let change:Change=serde_json::from_value(serde_json::json!({"version":5,"mode":"set","targetCapture":0,"valueCapture":1,
        "bindingRole":"binding","declaredStorage":{"version":1,"basis":"owned.standard-contract.v1","types":["owned.UnknownType"],
        "complete":true,"maybeUnset":true,"preservesLocalBindings":false}})).unwrap();
    let ty=declared_storage(&change).unwrap().ty().unwrap();
    assert!(ty.proven_types().is_some());assert!(ty.proven_present().is_none());
    assert!(!identity_delta(&change,&ty));assert!(storage_result(&change).is_none());
    assert!(!declared_storage(&change).unwrap().preserves_local_bindings);
    let mut old=change;old.version=4;assert!(declared_storage(&old).is_none());
}

#[test]
fn complete_declared_upper_bounds_and_joins_do_not_supply_negative_runtime_proof() {
    let declared=DeclaredStorage {version:1,basis:"owned.contract".into(),types:vec!["org.bukkit.entity.Entity".into()],
        complete:true,maybe_unset:false,preserves_local_bindings:false}.ty().unwrap();
    assert!(declared.proven_present().is_some());
    assert!(declared.exclusion_types().is_none());
    let joined=declared.join(&value("java.lang.String",false));
    assert!(joined.proven_types().is_some());assert!(joined.exclusion_types().is_none());
    assert!(value("java.lang.String",false).exclusion_types().is_some());
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
        missing_value: None,
    };
    let mut change = Change {
        version: 2,
        mode: "set".into(),
        target_capture: 0,
        value_capture: Some(1),
        target_only: true,
        identity_delta: false,
        identity_delta_types: vec!["java.lang.Number".into()],
        storage_result: None,
        binding_role: String::new(),
        dispatch_rules: Vec::new(),
        declared_storage: None,
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

#[test]
fn nullable_reads_require_an_explicit_empty_missing_contract_and_safe_conversion() {
    for accepted_version in [2,3] {
        let catalog = crate::lsp::test_support::multi_addon_catalog();
        let mut read = VariableRead {version:accepted_version,requested_types:vec!["java.lang.Object".into()],
            may_refresh_player:true,no_refresh_types:vec!["java.lang.String".into()],
            missing_value:Some(MissingVariableValue::Empty)};
        let nullable=value("java.lang.String",true);
        assert!(read_preserves_type_facts(&read,&nullable,&catalog));
        assert!(matches!(nullable,InferredType::Values {maybe_unset:true,..}),"lookup cannot invent presence");
        for version in [0,1,4] {
            read.version=version;
            assert!(!read_preserves_type_facts(&read,&nullable,&catalog));
        }
        read.version=accepted_version;read.missing_value=None;
        assert!(!read_preserves_type_facts(&read,&nullable,&catalog),"unknown fallback is not empty");
        read.missing_value=Some(MissingVariableValue::Empty);
        read.requested_types=vec!["java.lang.Number".into()];
        assert!(!read_preserves_type_facts(&read,&nullable,&catalog),"typed converter still requires its own contract");
        read.requested_types=vec!["java.lang.Object".into()];
        assert!(!read_preserves_type_facts(&read,&value("org.bukkit.entity.Player",true),&catalog),
            "missing-value closure does not close Player refresh");
        for (certainty,complete) in [(Certainty::Heuristic,true),(Certainty::Inferred,false)] {
            let mut partial=nullable.clone();
            if let InferredType::Values {support,..}=&mut partial {support.certainty=certainty;support.candidates_complete=complete;}
            assert!(!read_preserves_type_facts(&read,&partial,&catalog));
        }
        for missing in ["unknown","global-default","invented"] {
            assert!(serde_json::from_value::<VariableRead>(serde_json::json!({"version":accepted_version,
                "requestedTypes":["java.lang.Object"],"mayRefreshPlayer":false,"missingValue":missing})).is_err());
        }
    }
}
