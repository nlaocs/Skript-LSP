use super::*;

#[test]
fn simulated_native_target_uses_generic_storage_dispatch_and_qualified_provenance() {
    let (source,manifest,types,events)=super::super::evidence::observation::interval::tests::synthetic();
    let bound=BoundObservation::split_manifest(&source,&serde_json::to_vec(&manifest).unwrap(),&source).unwrap().1.unwrap().unwrap();
    bound.check_projection(&types,&events).unwrap();
    let model=make_standard_native_model(&source,&bound,&manifest,types).unwrap();
    assert!(model.stored("org.bukkit.entity.Player",IMPLEMENTATION));
    let values=model.result("org.bukkit.entity.Player",&model.target.as_ref().unwrap().getter);
    let result:ResultType=serde_json::from_str(&values[0].value).unwrap();
    assert_eq!(result.types,["org.bukkit.entity.Player"]);assert!(result.maybe_unset);
    let provenance:Value=serde_json::from_str(&values[1].value).unwrap();
    assert_eq!(provenance["basis"],super::super::evidence::observation::interval::BASIS);
    assert_eq!(provenance["premise"],"initialized-quiescent-registry");
    assert_eq!(provenance["eventRegistration"],JOIN_REGISTRATION);
    let basis=model.analysis_basis();
    assert!(basis.matches(STANDARD_MODEL,basis.basis,Some("initialized-quiescent-registry"),Some(JOIN_REGISTRATION)));
    assert!(!basis.matches(MODEL,basis.basis,basis.premise,basis.event_registration));
    assert!(!basis.matches(STANDARD_MODEL,basis.basis,None,basis.event_registration));
    assert!(!basis.matches(STANDARD_MODEL,basis.basis,basis.premise,Some("addon.custom-event")));
    let mut changed=source.clone();changed.source_digest="a".repeat(64);
    assert!(make_standard_native_model(&changed,&bound,&manifest,model.records.clone()).is_none());
}
