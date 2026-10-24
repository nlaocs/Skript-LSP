//! Provider-owned conditional change rules. No syntax or Java class names are
//! interpreted here. Unresolved dispatch is not a definite binding write.
use super::{InferredType, StorageResult};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct Rule {
    target_types: Vec<String>,
    delta_types: Vec<String>,
    result: StorageResult,
}

pub(super) fn resolve<'a>(rules: &'a [Rule],target:&InferredType,delta:&InferredType)->Option<&'a StorageResult> {
    if rules.is_empty() || rules.len()>64 { return None; }
    let (targets,false)=target.proven_present()? else { return None; };
    let (deltas,false)=delta.proven_present()? else { return None; };
    let valid=|types:&[String]| !types.is_empty() && types.len()<=64 && types.iter().all(|s|!s.is_empty())
        && types.iter().collect::<std::collections::BTreeSet<_>>().len()==types.len();
    if rules.iter().any(|rule|!valid(&rule.target_types)||!valid(&rule.delta_types)
        || !rule.result.preserves_local_bindings || rule.result.ty().is_none()) {return None;}
    // A narrower overlapping rule can conflict on one member of a union even
    // though it does not cover the whole union. Never ignore that overlap.
    let mut matching=rules.iter().filter(|rule|targets.iter().any(|t|rule.target_types.contains(t))
        && deltas.iter().any(|t|rule.delta_types.contains(t)));
    let rule=matching.next()?;
    (matching.next().is_none() && targets.iter().all(|t|rule.target_types.contains(t))
        && deltas.iter().all(|t|rule.delta_types.contains(t))).then_some(&rule.result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::semantic::{TypeSupport,Certainty};
    use std::collections::BTreeSet;
    fn ty(names:&[&str])->InferredType { InferredType::Values {classes:names.iter().map(|s|s.to_string()).collect(),multiple:false,maybe_unset:false,support:TypeSupport::inferred()} }
    fn rule()->Rule { serde_json::from_value(serde_json::json!({"targetTypes":["fixture.A","fixture.B"],"deltaTypes":["fixture.D"],"result":{"version":1,"scope":"local-scalar","types":["fixture.R"],"complete":true,"multiple":false,"maybeUnset":false,"preservesLocalBindings":true}})).unwrap() }
    #[test]
    fn conditional_rules_need_current_complete_present_scalar_and_unique_coverage() {
        let rules=vec![rule()];let target=ty(&["fixture.A","fixture.B"]);let delta=ty(&["fixture.D"]);
        assert!(resolve(&rules,&target,&delta).is_some());
        for variant in ["nullable","plural","partial","heuristic","unknown","other"] {
            for side in 0..2 {
                let mut bad=if side==0 {target.clone()} else {delta.clone()};
                if let InferredType::Values {maybe_unset,multiple,support,classes,..}=&mut bad {
                    match variant {"nullable"=>*maybe_unset=true,"plural"=>*multiple=true,"partial"=>support.candidates_complete=false,
                        "heuristic"=>support.certainty=Certainty::Heuristic,"other"=>{classes.insert("fixture.Other".into());},_=>{}}
                }
                if variant=="unknown" {bad=InferredType::Unknown;}
                let answer=if side==0 {resolve(&rules,&bad,&delta)}else{resolve(&rules,&target,&bad)};
                assert!(answer.is_none(),"{variant}, input {side}");
            }
        }
        let mut narrow=rule();narrow.target_types=vec!["fixture.A".into()];
        assert!(resolve(&[rule(),narrow],&target,&delta).is_none(),"overlap on part of a union is still ambiguous");
        assert!(resolve(&[rule(),rule()],&target,&delta).is_none(),"overlap is not an ordering preference");
        assert!(resolve(&rules,&InferredType::Values {classes:BTreeSet::new(),multiple:false,maybe_unset:true,support:TypeSupport::inferred()},&delta).is_none(),"unset/default path is separate");
    }
}
