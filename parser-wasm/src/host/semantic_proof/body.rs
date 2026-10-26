//! A body may import only its private, already-adopted closed receipt graph.
//! Source/dependency admission is independent; IDs alone never authorize reuse.
use super::*;

#[derive(Debug, Clone)]
pub(in crate::host) struct BodyReceiptBundle {
    sequence: Arc<AtomicU64>,
    receipts: BTreeMap<u64, Receipt>,
}

impl ReceiptRegistry {
    pub(in crate::host) fn export_body(
        &self,
        calls: &[HookCall],
        metadata: &[(String,String)],
    ) -> Option<BodyReceiptBundle> {
        if self.exhausted { return None; }
        let mut admitted = BTreeSet::new();
        for call in calls {
            let Some(token) = call.semantic_receipt else { continue; };
            let receipt = self.receipts.get(&token)?;
            if !receipt.valid || receipt.issuer != call.component_id { return None; }
            admitted.insert(token);
        }
        let mut pending = admitted.iter().copied().collect::<Vec<_>>();
        while let Some(token) = pending.pop() {
            let receipt = self.receipts.get(&token)?;
            if !receipt.valid { return None; }
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
