//! A body may import only its private, already-adopted closed receipt graph.
//! Source/dependency admission is independent; IDs alone never authorize reuse.
use super::*;

#[derive(Debug, Clone)]
pub(in crate::host) struct BodyReceiptBundle {
    sequence: Arc<AtomicU64>,
    receipts: BTreeMap<u64, Receipt>,
}

impl ReceiptRegistry {
    /// Bounded test-only attribution for an export refusal. Never imports or
    /// changes validity. Tokens and payload metadata stay out of the report.
    #[cfg(test)]
    pub(in crate::host) fn body_shadow_export(&self, calls:&[HookCall], metadata:&[(String,String)]) -> serde_json::Value {
        if calls.len()>4096 {return serde_json::json!({"exported":false,"overflow":true});}
        let mut admitted=BTreeSet::new();let mut missing=0;let mut invalid=0;
        for call in calls {
            if let Some(token)=call.semantic_receipt {
                match self.receipts.get(&token) {
                    Some(receipt)=>{invalid+=usize::from(!receipt.valid || receipt.issuer!=call.component_id);admitted.insert(token);},
                    None=>missing+=1,
                }
            }
        }
        let mut pending=admitted.iter().copied().collect::<Vec<_>>();let mut overflow=false;let mut edges=0;
        'graph: while let Some(token)=pending.pop() {
            if let Some(receipt)=self.receipts.get(&token) {
                for child in &receipt.admitted_nested {
                    if edges>=4096 || (!admitted.contains(child) && admitted.len()>=4096) {overflow=true;break 'graph;}
                    edges+=1;if admitted.insert(*child) {pending.push(*child);}
                }
            } else {missing+=1;}
        }
        let mut external_parents=0;
        if !overflow {
            'parents: for receipt in admitted.iter().filter_map(|token|self.receipts.get(token)) {
                for parent in &receipt.parents {
                    if edges>=4096 {overflow=true;break 'parents;}
                    edges+=1;external_parents+=usize::from(!admitted.contains(parent));
                }
            }
        }
        if overflow {return serde_json::json!({"exported":false,"overflow":true,"inspectedReceipts":admitted.len()});}
        struct Size(usize);
        impl fmt::Write for Size {
            fn write_str(&mut self,value:&str)->fmt::Result {
                self.0=self.0.saturating_add(value.len());
                if self.0>256*1024 {Err(fmt::Error)} else {Ok(())}
            }
        }
        let mut size=Size(0);
        overflow |= write!(&mut size,"{metadata:?}").is_err();
        if !overflow {
            for token in &admitted {
                if write!(&mut size,"{:?}",self.receipts.get(token)).is_err() {overflow=true;break;}
            }
        }
        if overflow {return serde_json::json!({"exported":false,"overflow":true,"inspectedReceipts":admitted.len()});}
        let external_metadata=metadata.iter().filter(|(key,_)|key==RECEIPT)
            .filter(|(_,value)|value.parse::<u64>().ok().is_none_or(|token|!admitted.contains(&token))).count();
        let exported=!overflow && self.export_body(calls,metadata).is_some();
        serde_json::json!({"exported":exported,"closedReceipts":admitted.len(),"missing":missing,"invalid":invalid,
            "externalParentEdges":external_parents,"externalMetadataReferences":external_metadata,
            "exhausted":self.exhausted,"overflow":overflow})
    }

    /// Canonicalize only protected host IDs; opaque provider context stays exact.
    /// Writer/parent/nested topology remains part of the current header identity.
    pub(in crate::host) fn body_header_identity(&self, metadata: &mut BTreeMap<String,String>) -> Option<String> {
        if self.exhausted { return None; }
        let mut roots=BTreeSet::new();
        if let Some(value)=metadata.get(RECEIPT) { roots.insert(value.parse::<u64>().ok()?); }
        let mut admitted=BTreeSet::new();
        let mut pending=roots.iter().copied().collect::<Vec<_>>();
        while let Some(token)=pending.pop() {
            if admitted.contains(&token) {continue;}
            if admitted.len()>=4096 {return None;}
            let receipt=self.receipts.get(&token)?;
            if !receipt.valid {return None;}
            admitted.insert(token);
            pending.extend(receipt.parents.iter().chain(&receipt.admitted_nested).copied());
        }
        let rank=admitted.iter().enumerate().map(|(i,t)|(*t,i)).collect::<BTreeMap<_,_>>();
        let graph=admitted.iter().map(|token| {
            let r=&self.receipts[token];
            (&r.issuer,&r.writers,r.unbound,r.parents.iter().map(|t|rank[t]).collect::<Vec<_>>(),r.admitted_nested.iter().map(|t|rank[t]).collect::<Vec<_>>())
        }).collect::<Vec<_>>();
        let roots=roots.iter().map(|t|rank[t]).collect::<Vec<_>>();
        let identity=debug_digest("body-header-authority-v1", &(roots,graph));
        if let Some(value)=metadata.get_mut(RECEIPT) { *value="opaque host receipt".into(); }
        Some(format!("{identity:x?}"))
    }

    pub(in crate::host) fn export_body(
        &self,
        calls: &[HookCall],
        metadata: &[(String,String)],
    ) -> Option<BodyReceiptBundle> {
        if self.exhausted || metadata.iter().any(|(key,_)| matches!(key.as_str(),value_facts::DRAFT|value_facts::INPUT|value_facts::COMPOSE)) { return None; }
        let mut admitted = BTreeSet::new();
        for call in calls {
            let Some(token) = call.semantic_receipt else { continue; };
            let receipt = self.receipts.get(&token)?;
            if !receipt.valid || receipt.issuer != call.component_id || self.value_facts.has_receipt(token) { return None; }
            admitted.insert(token);
        }
        let mut pending = admitted.iter().copied().collect::<Vec<_>>();
        while let Some(token) = pending.pop() {
            let receipt = self.receipts.get(&token)?;
            if !receipt.valid || self.value_facts.has_receipt(token) { return None; }
            for child in &receipt.admitted_nested {
                if admitted.insert(*child) { pending.push(*child); }
            }
        }
        // A referenced external parent/header cannot silently widen the cached
        // authority. It remains current and forces this particular body cold.
        for token in &admitted {
            let receipt = self.receipts.get(token)?;
            if !receipt.parents.is_subset(&admitted) { return None; }
        }
        for (key,value) in metadata {
            if key == RECEIPT && !admitted.contains(&value.parse::<u64>().ok()?) {
                return None;
            }
        }
        Some(BodyReceiptBundle {
            sequence: self.sequence.clone(),
            receipts: admitted.into_iter().map(|token| (token,self.receipts[&token].clone())).collect(),
        })
    }

    pub(in crate::host) fn can_import_body(&self, bundle: &BodyReceiptBundle) -> bool {
        !self.exhausted && Arc::ptr_eq(&self.sequence,&bundle.sequence)
            && bundle.receipts.keys().all(|token| !self.receipts.contains_key(token))
            && bundle.receipts.values().all(|receipt| receipt.valid
                && receipt.parents.iter().chain(&receipt.admitted_nested).all(|token| bundle.receipts.contains_key(token)))
    }

    pub(in crate::host) fn import_body(&mut self, bundle: &BodyReceiptBundle) -> bool {
        if !self.can_import_body(bundle) { return false; }
        self.receipts.extend(bundle.receipts.clone());
        true
    }
}

#[test]
fn shadow_receipt_export_bounds_graph_edges_and_metadata_before_cloning() {
    let mut registry=ReceiptRegistry::default();
    registry.receipts.insert(1,Receipt {issuer:"public.fixture".into(),writers:BTreeSet::new(),parents:BTreeSet::new(),
        admitted_nested:(2..5002).collect(),valid:true,unbound:false});
    let calls=vec![HookCall {origin:HookCallOrigin::Executed,component_id:"public.fixture".into(),subscription_id:"fixture".into(),semantic_receipt:Some(1)}];
    let result=registry.body_shadow_export(&calls,&[]);
    assert_eq!(result["overflow"],true);assert_eq!(result["exported"],false);
    assert!(result["inspectedReceipts"].as_u64().unwrap()<=4096);
    registry.receipts.get_mut(&1).unwrap().admitted_nested.clear();
    let result=registry.body_shadow_export(&calls,&[("public.fixture".into(),"a".repeat(256*1024+1))]);
    assert_eq!(result["overflow"],true);assert_eq!(result["exported"],false);
    assert_eq!(registry.receipts.len(),1,"probe must not mutate the registry");
}
