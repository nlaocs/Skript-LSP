//! Local State reach never widens semantic authorship or native authority.
use super::*;
use super::tests::{node, payload, write};
fn observe(registry: &mut ReceiptRegistry, node: &mut AstNode, issuer: &str, scope: SemanticEffects) -> HookCall {
    let mut payload=payload();
    if let HookPayload::Matching(p)=&mut payload {p.metadata=node.metadata.clone();}
    let mut writers=registry.begin(&payload);
    let receipt=registry.accept(&mut writers,&mut payload,issuer,HookMode::Observe,scope);
    assert!(receipt.is_some(),"local State writes still retain receipt and issuer");
    if let HookPayload::Matching(p)=payload {node.metadata=p.metadata;}
    HookCall {origin:HookCallOrigin::Executed,component_id:issuer.into(),subscription_id:"state".into(),semantic_receipt:receipt}
}
#[test]
fn local_private_writers_keep_their_node_authority_without_tainting_other_roots() {
    let mut registry=ReceiptRegistry::default();let mut import=node(0);let mut assignment=node(1);
    let calls=vec![observe(&mut registry,&mut import,"addon-a",SemanticEffects::Local),
        observe(&mut registry,&mut import,"addon-b",SemanticEffects::Local),write(&mut registry,&mut assignment,"core")];
    let tree=AstTree {roots:vec![0,1],nodes:vec![import,assignment]};
    let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&calls);let view=proof.view(&tree);
    assert!(view.provider_valid(&tree.nodes[1],"core"));assert_eq!(view.unique_issuer(&tree.nodes[0]),None);
    // Editing metadata ownership or a copied issuer label cannot grant a type proof.
    assert!(!view.provider_valid(&tree.nodes[0],"addon-a"));
    let mut changed=tree.clone();changed.nodes[1].span.virtual_range.end+=1;
    assert!(!proof.view(&changed).provider_valid(&changed.nodes[1],"core"));
    let fresh=ReceiptRegistry::default();
    assert!(!SemanticProofRegistry::capture(&tree,Some(&fresh),&calls).view(&tree).provider_valid(&tree.nodes[1],"core"));
}
#[test]
fn local_scope_does_not_erase_transform_conflict_ancestor_or_unattached_dependencies() {
    for placement in 0..3 {
        let mut registry=ReceiptRegistry::default();let mut other=node(0);let mut assignment=node(1);
        if placement==1 {other.children=vec![1];}
        let local=observe(&mut registry,&mut other,"addon",SemanticEffects::Local);
        let core=write(&mut registry,&mut assignment,"core");
        let calls=if placement==0 {
            vec![local,core,write(&mut registry,&mut assignment,"conflict")]
        } else {vec![local,core]};
        let tree=if placement==2 {AstTree {roots:vec![1],nodes:vec![assignment]}}
            else {AstTree {roots:if placement==1 {vec![0]} else {vec![0,1]},nodes:vec![other,assignment]}};
        let assignment=tree.nodes.iter().find(|n|n.id==1).unwrap();
        assert!(!SemanticProofRegistry::capture(&tree,Some(&registry),&calls).view(&tree).provider_valid(assignment,"core"),"placement {placement}");
    }
}
#[test]
fn one_unbound_addon_still_invalidates_otherwise_local_composition() {
    let mut registry=ReceiptRegistry::default();let mut import=node(0);let mut assignment=node(1);
    let calls=vec![observe(&mut registry,&mut import,"local",SemanticEffects::Local),
        observe(&mut registry,&mut import,"shared",SemanticEffects::Unbound),write(&mut registry,&mut assignment,"core")];
    let tree=AstTree {roots:vec![0,1],nodes:vec![import,assignment]};
    assert!(!SemanticProofRegistry::capture(&tree,Some(&registry),&calls).view(&tree).provider_valid(&tree.nodes[1],"core"));
}
