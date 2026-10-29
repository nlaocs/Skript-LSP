//! Ordinary, API-contract type bounds. These are NOT native-origin, identity,
//! purity or exact-runtime-class certificates. No event/type inventory is pinned.
use super::domain::{Class, Graph};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const BASE: &str = "ch.njol.skript.classes.ClassInfo";
const EXPRESSION: &str = "ch.njol.skript.lang.Expression";
const SLOT: &str = "ch.njol.skript.util.slot.Slot";
const ITEM: &str = "org.bukkit.inventory.ItemStack";
pub(super) const KEY: &str = "skript.semantic.declared-storage-input";
pub(super) const BASIS: &str = "skript.standard-storage-contract.v1";

/// An audited standard adapter can construct an expression internally. This
/// identifies that delegate, not a public value type or a purity certificate.
#[derive(Clone, Copy)]
pub(crate) struct DelegatedInput {
    pub adapter: &'static str,
    pub forwarder: &'static str,
    pub receiver: &'static str,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct Inference {
    pub version: u32,
    pub basis: String,
    pub types: Vec<String>,
    pub complete: bool,
    pub maybe_unset: bool,
    pub preserves_local_bindings: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all="camelCase")]
pub(super) struct Declaration {
    pub name: String,
    pub kind: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
}
pub(super) struct Model {
    records: Vec<Value>,
    graph: Graph,
    declarations: BTreeMap<String, Declaration>,
}
// This adapter models the common beforeChange/clone/local serializeAs contract,
// not all language semantics of the accepted runtime range.
pub(super) fn supports(version: &str) -> bool {
    crate::runtime::parse_skript_version(version).is_some_and(|v| v.0==2 && (12..=15).contains(&v.1))
}
impl Model {
    pub(super) fn from_records(records:Vec<Value>)->Option<Self> {
        let mut declarations=Vec::new();let mut classes=Vec::new();
        for r in &records {
            let t=&r["typeRuntime"];let name=r["originalClass"].as_str()?;
            if t["version"]!=3||t["ancestryComplete"]!=true {return None;}
            let ancestors=t["ancestors"].as_array()?;
            if ancestors.len()>1024 {return None;}
            let ancestors=ancestors.iter().map(|a|a.as_str().filter(|s|!s.is_empty()&&s.len()<=1024).map(str::to_owned)).collect::<Option<Vec<_>>>()?;
            if ancestors.iter().any(|a|a==name)||ancestors.iter().collect::<BTreeSet<_>>().len()!=ancestors.len() {return None;}
            let kind=t["classKind"].as_str()?;
            // Array covariance and primitive boxing are not represented by
            // this class/interface dispatch model.
            if matches!(kind,"Array"|"Primitive") {return None;}
            declarations.push(Declaration {name:name.into(),kind:kind.into(),super_class:None,interfaces:ancestors.clone()});
            classes.push(Class {name:name.into(),kind:kind.into(),super_class:None,interfaces:ancestors,lineage_complete:true});
        }
        let graph=Graph::new(classes)?;
        let mut model=Self::new(records,declarations)?;model.graph=graph;Some(model)
    }
    pub(super) fn new(records: Vec<Value>, declarations: Vec<Declaration>) -> Option<Self> {
        if records.is_empty() || records.len()>1024 || declarations.is_empty() || declarations.len()>8192 {return None;}
        let graph=Graph::new(declarations.iter().map(|d|Class {name:d.name.clone(),kind:d.kind.clone(),super_class:d.super_class.clone(),interfaces:d.interfaces.clone(),lineage_complete:false}).collect())?;
        let mut classes=BTreeSet::new();
        for (i,r) in records.iter().enumerate() {
            let c=r["originalClass"].as_str()?; let t=&r["typeRuntime"];
            if c.is_empty() || c.len()>1024 || !classes.insert(c.to_owned()) || r["typeParseOrder"].as_u64()!=Some(i as u64)
                || t["version"]!=3 || t["classState"]!="present" || t["classGetterDeclaringClass"]!=BASE
                || t["classValue"]!=c || !t["classUnknownReason"].is_null() || graph.assignable(c,c)!=Some(true) {return None;}
        }
        let count=declarations.len();
        let declarations=declarations.into_iter().map(|d|(d.name.clone(),d)).collect::<BTreeMap<_,_>>();
        (declarations.len()==count).then_some(Self {records,graph,declarations})
    }
    /// Validate the entire inherited closure, not just the concrete class. The
    /// caller supplies source-fenced declared-method metadata; None is unknown.
    #[cfg(test)]
    pub(super) fn default_before_change(&self, class:&str, mut declares:impl FnMut(&str)->Option<bool>) -> bool {
        let mut pending=vec![class]; let mut seen=BTreeSet::new(); let mut found=false;
        while let Some(c)=pending.pop() {
            if !seen.insert(c) {continue;} if seen.len()>128 {return false;}
            let Some(row)=self.declarations.get(c) else {return false;};
            match declares(c) {Some(true) if c==EXPRESSION=>found=true,Some(false)=>{},_=>return false}
            if let Some(parent)=&row.super_class {pending.push(parent);}
            pending.extend(row.interfaces.iter().map(String::as_str));
        }
        found
    }
    /// Exact runtime-class registrations have priority over the ordered scan.
    /// A later exact registration must remain a candidate, even beyond the first
    /// ordered entry that covers every value of the public input bound.
    fn dispatch(&self, bound:&str)->Option<Vec<&Value>> {
        if self.graph.assignable(bound,bound)!=Some(true) || !self.declarations.contains_key(bound) {return None;}
        let mut indices=BTreeSet::new();
        for (i,r) in self.records.iter().enumerate() {
            let c=r["originalClass"].as_str()?;
            if self.graph.concrete(c)? && self.graph.assignable(c,bound)? {indices.insert(i);}
        }
        let mut covered=false;
        for (i,r) in self.records.iter().enumerate() {
            let c=r["originalClass"].as_str()?;
            if self.graph.overlap(c,bound)? {indices.insert(i);}
            if self.graph.assignable(bound,c)? {covered=true;break;}
        }
        covered.then(||indices.into_iter().map(|i|&self.records[i]).collect())
    }
    fn clone_bounds(&self,bound:&str)->Option<(BTreeSet<String>,bool)> {
        let mut out=BTreeSet::new(); let mut preserves=true;
        for r in self.dispatch(bound)? {
            let t=&r["typeRuntime"];
            if t["cloneMethodState"]!="known" || !t["cloneMethodUnknownReason"].is_null() {return None;}
            if t["cloneMethodDeclaringClass"]==BASE && t["clonerGetterDeclaringClass"]==BASE && t["clonerState"]=="absent"
                && t["clonerClass"].is_null() && t["clonerUnknownReason"].is_null() {
                out.insert(bound.into());
            } else {
                // ClassInfo<C>.clone returns C under its ordinary Java type
                // contract, not necessarily the original subtype. No purity.
                out.insert(r["originalClass"].as_str()?.into()); preserves=false;
            }
        }
        (!out.is_empty()&&out.len()<=64).then_some((out,preserves))
    }
    fn stored_bounds(&self,bound:&str)->Option<(BTreeSet<String>,bool)> {
        let mut out=BTreeSet::new(); let mut preserves=true;
        for r in self.dispatch(bound)? {
            let t=&r["typeRuntime"];
            if t["serializeAsGetterDeclaringClass"]!=BASE || !t["serializeAsUnknownReason"].is_null() {return None;}
            match t["serializeAsState"].as_str()? {
                "absent" if t["serializeAsClass"].is_null() && r["serializeAs"].is_null()=>{out.insert(bound.into());},
                "present"=>{
                    let target=t["serializeAsClass"].as_str()?;
                    if target.is_empty() || target.len()>1024 || !self.declarations.contains_key(target)
                        || r["serializeAs"].as_str()!=Some(target) {return None;}
                    // This dispatch branch's value is also an instance of the
                    // selected registration class. Converters.convert returns
                    // the same value if either bound proves it is already T.
                    if self.graph.assignable(bound,target)? || self.graph.assignable(r["originalClass"].as_str()?,target)? {out.insert(bound.into());}
                    else {out.insert(target.into());preserves=false;}
                },
                _=>return None,
            }
        }
        (!out.is_empty()&&out.len()<=64).then_some((out,preserves))
    }
    pub(super) fn infer(&self, inputs:&[String])->Option<Inference> {
        if inputs.is_empty()||inputs.len()>64||inputs.iter().any(|c|c.is_empty()||c=="java.lang.Object") {return None;}
        let mut intermediate=BTreeSet::new(); let mut preserves=true;
        for bound in inputs {
            // Open interfaces can overlap Slot. Include the conversion branch;
            // lack of a known implementor is not a disjointness proof.
            if self.graph.overlap(bound,SLOT)? {intermediate.insert(ITEM.into());preserves=false;}
            if !self.graph.assignable(bound,SLOT)? {
                let (cloned,safe)=self.clone_bounds(bound)?;intermediate.extend(cloned);preserves &= safe;
            }
        }
        if intermediate.len()>64 {return None;}
        let mut output=BTreeSet::new();
        // ClassInfo dispatch is repeated AFTER cloning, since a cloner may
        // return another subtype or a wider registered class.
        for bound in intermediate {
            let (stored,safe)=self.stored_bounds(&bound)?;output.extend(stored);preserves &= safe;
        }
        if output.is_empty()||output.len()>64||output.contains("java.lang.Object") {return None;}
        Some(Inference {version:1,basis:BASIS.into(),types:output.into_iter().collect(),complete:true,maybe_unset:true,preserves_local_bindings:preserves})
    }
}

fn default_hook(closure:&[String],mut declares:impl FnMut(&str)->Option<bool>)->bool {
    !closure.is_empty()&&closure.len()<=128&&closure.iter().collect::<BTreeSet<_>>().len()==closure.len()
        && closure.iter().any(|c|c==EXPRESSION)
        && closure.iter().all(|c|declares(c)==Some(c==EXPRESSION))
}

fn forwarding_hook(closure:&[String],forwarder:&str,mut declares:impl FnMut(&str)->Option<bool>)->bool {
    !closure.is_empty()&&closure.len()<=128&&closure.iter().collect::<BTreeSet<_>>().len()==closure.len()
        && closure.iter().any(|c|c==EXPRESSION) && closure.iter().any(|c|c==forwarder)
        && closure.iter().all(|c|declares(c)==Some(c==EXPRESSION||c==forwarder))
}

mod source;
pub(crate) use source::{annotate, for_summary, prepare_step};

#[cfg(test)]
mod tests;
