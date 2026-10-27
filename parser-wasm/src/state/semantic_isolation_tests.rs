//! Host-enforced reach, independent of optional cache observations.
use super::*;
const OWNER: &str = "test.isolation.a";
const OTHER: &str = "test.isolation.b";
const NS: &str = "private";
const SCHEMA: &str = "test.state";
fn store() -> StateStore {
    let store = StateStore::new(StateStoreConfig::default()).unwrap();
    store.register_component(OWNER, &[
        NamespaceDeclaration::private(NS, SCHEMA, 1),
        NamespaceDeclaration::shared("shared", SCHEMA, 1, [OTHER], [OTHER]),
    ]).unwrap();
    store.register_component(OTHER, &[NamespaceDeclaration::private(NS, SCHEMA, 1)]).unwrap();
    store
}
fn value() -> StateValue { StateValue::new(SCHEMA, StateEncoding::Raw, b"value") }
fn parse(store: &StateStore, revision: u64) -> ParseTransaction {
    store.begin_parse("file:///workspace", "file:///workspace/isolation.sk", revision).unwrap()
}
#[test]
fn only_actual_owner_private_short_lived_writes_are_isolated() {
    for (scope, isolated) in [(StateScope::Invocation,true),(StateScope::Parse,true),
        (StateScope::Document,false),(StateScope::Project,false)] {
        for visibility in [NamespaceVisibility::Private, NamespaceVisibility::Shared] {
            let store=store();let parse=parse(&store,1);let mut call=parse.begin_invocation(OWNER).unwrap();
            let namespace=if visibility==NamespaceVisibility::Private {NS} else {"shared"};
            call.put(scope,visibility,namespace,"key",value()).unwrap();
            let dependency=call.read_write_set();assert_eq!(dependency.writes.len(),1);
            assert_eq!(call.has_only_isolated_private_writes(), isolated && visibility==NamespaceVisibility::Private);
            if visibility==NamespaceVisibility::Private {
                assert_eq!(dependency.writes.first().unwrap().owner.as_deref(),Some(OWNER));
                let mut forged=dependency.writes.first().unwrap().clone();forged.owner=Some(OTHER.into());
                call.write_set.insert(forged);assert!(!call.has_only_isolated_private_writes());
            }
            call.rollback();assert!(parse.read_write_set().unwrap().writes.is_empty());parse.cancel().unwrap();
        }
    }
}
#[test]
fn caught_state_api_errors_prevent_local_proof_even_without_observation() {
    for observe in [false,true] { for operation in 0..5 {
        let store=store();let parse=parse(&store,1);let mut call=parse.begin_invocation(OWNER).unwrap();
        if observe {parse.enable_read_observations().unwrap();call.observe_reads();}
        call.put(StateScope::Parse,NamespaceVisibility::Private,NS,"key",value()).unwrap();
        assert!(call.has_only_isolated_private_writes());
        let failed=match operation {
            0=>call.get(StateScope::Parse,NamespaceVisibility::Private,"missing","key").is_err(),
            1=>call.scan_prefix(StateScope::Parse,NamespaceVisibility::Private,NS,"",usize::MAX).is_err(),
            2=>call.put(StateScope::Parse,NamespaceVisibility::Private,NS,"key",StateValue::new("wrong.schema",StateEncoding::Raw,b"x")).is_err(),
            3=>call.delete(StateScope::Parse,NamespaceVisibility::Shared,"missing","key").is_err(),
            _=>call.compare_and_swap(StateScope::Parse,NamespaceVisibility::Private,NS,"key",Some(&value()),Some(StateValue::new("wrong.schema",StateEncoding::Raw,b"x"))).is_err(),
        };
        assert!(failed,"operation {operation}");assert!(!call.has_only_isolated_private_writes());
        assert_eq!(call.read_write_set().writes.len(),1,"a failed operation cannot invent a committed write");
        call.rollback();parse.cancel().unwrap();
    }}
}
#[test]
fn private_state_and_dependencies_remain_separate_across_addons_and_lifetimes() {
    let store=store();let first=parse(&store,1);
    let mut a=first.begin_invocation(OWNER).unwrap();a.put(StateScope::Parse,NamespaceVisibility::Private,NS,"key",value()).unwrap();a.commit().unwrap();
    let mut b=first.begin_invocation(OTHER).unwrap();assert!(b.get(StateScope::Parse,NamespaceVisibility::Private,NS,"key").unwrap().is_none());
    b.put(StateScope::Parse,NamespaceVisibility::Private,NS,"key",value()).unwrap();assert!(b.has_only_isolated_private_writes());b.commit().unwrap();
    assert_eq!(first.read_write_set().unwrap().writes.len(),2);
    let mut stale=first.begin_invocation(OWNER).unwrap();stale.put(StateScope::Parse,NamespaceVisibility::Private,NS,"late",value()).unwrap();
    first.cancel().unwrap();assert!(stale.commit().is_err());
    let current=parse(&store,2);let mut a=current.begin_invocation(OWNER).unwrap();
    assert!(a.get(StateScope::Parse,NamespaceVisibility::Private,NS,"key").unwrap().is_none());a.rollback();current.commit().unwrap();
    assert!(store.release_document("file:///workspace","file:///workspace/isolation.sk",2).unwrap());
    let reopened=parse(&store,2);let mut b=reopened.begin_invocation(OTHER).unwrap();
    assert!(b.get(StateScope::Parse,NamespaceVisibility::Private,NS,"key").unwrap().is_none());b.rollback();reopened.cancel().unwrap();
}
