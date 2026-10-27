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
fn local_private_observers_keep_receipts_without_claiming_node_authority() {
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
        let mut prefix=Vec::new();
        if placement==1 {prefix.push(write(&mut registry,&mut other,"ancestor-transform"));}
        let local=observe(&mut registry,&mut other,"addon",SemanticEffects::Local);
        let core=write(&mut registry,&mut assignment,"core");
        let mut calls=if placement==0 {
            vec![local,core,write(&mut registry,&mut assignment,"conflict")]
        } else {vec![local,core]};
        prefix.append(&mut calls);let calls=prefix;
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

#[test]
fn private_observers_preserve_bound_core_ancestor_and_subtree_facts() {
    let mut registry=ReceiptRegistry::default();let mut header=node(0);let mut child=node(1);
    header.children=vec![1];
    let core_header=write(&mut registry,&mut header,"core");
    let observer=observe(&mut registry,&mut header,"observer",SemanticEffects::Local);
    let receipt=&registry.receipts[&observer.semantic_receipt.unwrap()];
    assert_eq!(receipt.issuer,"observer");assert_eq!(receipt.writers,BTreeSet::from(["core".into()]));
    let core_child=write(&mut registry,&mut child,"core");
    let tree=AstTree {roots:vec![0],nodes:vec![header,child]};
    let calls=[core_header,observer,core_child];
    let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&calls);let view=proof.view(&tree);
    for node in &tree.nodes {assert!(view.provider_valid(node,"core"));assert!(!view.provider_valid(node,"observer"));}
}

#[test]
fn unattached_local_receipt_recovers_all_prior_participants_without_adopting_calls() {
    let mut registry=ReceiptRegistry::default();let mut dropped=node(0);let mut current=node(1);
    let earlier=observe(&mut registry,&mut dropped,"earlier-observer",SemanticEffects::Local);
    let latest=observe(&mut registry,&mut dropped,"latest-observer",SemanticEffects::Local);
    assert!(registry.receipts[&latest.semantic_receipt.unwrap()].parents.contains(&earlier.semantic_receipt.unwrap()));
    let core=write(&mut registry,&mut current,"core");
    // Only the latest cumulative call is directly retained; its old participant
    // is a dependency, not a newly adopted call or source of authority.
    let tree=AstTree {roots:vec![1],nodes:vec![current]};
    let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&[latest,core]);
    assert!(proof.writers.values().all(|set|set.contains("earlier-observer")&&set.contains("latest-observer")));
    assert!(!proof.view(&tree).provider_valid(&tree.nodes[0],"core"));
}

#[test]
fn a_private_observer_does_not_remove_its_earlier_real_modification() {
    let mut registry=ReceiptRegistry::default();let mut root=node(0);
    let core=write(&mut registry,&mut root,"core");let modification=write(&mut registry,&mut root,"addon");
    let observer=observe(&mut registry,&mut root,"addon",SemanticEffects::Local);
    assert_eq!(registry.receipts[&observer.semantic_receipt.unwrap()].writers,BTreeSet::from(["core".into(),"addon".into()]));
    let tree=AstTree {roots:vec![0],nodes:vec![root]};
    assert!(!SemanticProofRegistry::capture(&tree,Some(&registry),&[core,modification,observer]).view(&tree).provider_valid(&tree.nodes[0],"core"));
}

#[test]
fn a_transform_with_private_state_remains_a_semantic_modifier() {
    let mut registry=ReceiptRegistry::default();let mut root=node(0);let core=write(&mut registry,&mut root,"core");
    let mut payload=payload();if let HookPayload::Matching(p)=&mut payload {p.metadata=root.metadata.clone();}
    let mut writers=registry.begin(&payload);
    let token=registry.accept(&mut writers,&mut payload,"transform",HookMode::Transform,SemanticEffects::Local).unwrap();
    if let HookPayload::Matching(p)=payload {root.metadata=p.metadata;}
    let transform=HookCall {origin:HookCallOrigin::Executed,component_id:"transform".into(),subscription_id:"local-transform".into(),semantic_receipt:Some(token)};
    let tree=AstTree {roots:vec![0],nodes:vec![root]};
    assert!(!SemanticProofRegistry::capture(&tree,Some(&registry),&[core,transform]).view(&tree).provider_valid(&tree.nodes[0],"core"));
}

#[test]
fn malformed_unattached_receipt_ancestry_fails_closed() {
    for case in 0..3 {
        let mut registry=ReceiptRegistry::default();let mut dropped=node(0);let mut current=node(1);
        let observer=observe(&mut registry,&mut dropped,"observer",SemanticEffects::Local);let token=observer.semantic_receipt.unwrap();
        let receipt=registry.receipts.get_mut(&token).unwrap();
        match case {0=>{receipt.parents.insert(token);},1=>{receipt.parents.insert(0);},_=>receipt.valid=false}
        let core=write(&mut registry,&mut current,"core");let tree=AstTree {roots:vec![1],nodes:vec![current]};
        assert!(!SemanticProofRegistry::capture(&tree,Some(&registry),&[observer,core]).view(&tree).native_valid(&tree.nodes[0]),"case {case}");
    }
}
