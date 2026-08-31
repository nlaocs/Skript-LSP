use super::{
    accept, annotate, child, child_span, child_types, mark_unresolved, matches, register_handler,
    reject_with,
};
use crate::catalog::{self, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, DynamicMultiplicity, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.prop-cond-contains";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Accepted,
    Rejected,
    Unresolved,
}

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, ".PropCondContains");
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    if !matches(&payload, HANDLER_ID) {
        return None;
    }
    if matches!(payload.candidate.pattern_index, 4 | 5) {
        annotate(&mut payload, "semantic-mode", "inventory-contains");
        return Some(accept(payload));
    }
    let Some(haystack) = child(&payload, 0) else {
        mark_unresolved(&mut payload, "contains-haystack");
        return Some(accept(payload));
    };
    let Some(needles) = child(&payload, 1) else {
        mark_unresolved(&mut payload, "contains-needles");
        return Some(accept(payload));
    };
    let haystack_types = child_types(haystack);
    let needle_types = child_types(needles);
    if haystack_types.is_empty() || needle_types.is_empty() {
        mark_unresolved(&mut payload, "contains-return-type");
        return Some(accept(payload));
    }

    let allow_containment =
        payload.candidate.mark != 1 || haystack.multiplicity == Some(DynamicMultiplicity::Single);
    if allow_containment {
        match containment_verdict(&haystack_types, &needle_types) {
            Verdict::Accepted => {
                annotate(&mut payload, "semantic-mode", "property-contains");
                return Some(accept(payload));
            }
            Verdict::Unresolved => {
                mark_unresolved(&mut payload, "contains-property-contract");
                return Some(accept(payload));
            }
            Verdict::Rejected => {}
        }
    }
    match direct_verdict(&haystack_types, &needle_types) {
        Verdict::Accepted => {
            annotate(&mut payload, "semantic-mode", "direct-contains");
            Some(accept(payload))
        }
        Verdict::Unresolved => {
            mark_unresolved(&mut payload, "contains-comparator-contract");
            Some(accept(payload))
        }
        Verdict::Rejected => Some(reject_with(
            "the source and contained values cannot be compared",
            "core.prop-cond-contains.incompatible-types",
            child_span(&payload, 1),
        )),
    }
}

fn containment_verdict(haystacks: &[&str], needles: &[&str]) -> Verdict {
    let mut unresolved = false;
    for haystack in haystacks {
        if *haystack == "java.lang.Object" {
            unresolved = true;
            continue;
        }
        let handlers = match catalog::property_handlers_for_type("contains", haystack) {
            Ok(handlers) => handlers,
            Err(_) => {
                unresolved = true;
                continue;
            }
        };
        let element_types = handlers
            .iter()
            .filter(|handler| handler.handler_kind == "contains")
            .flat_map(|handler| handler.element_types.iter().map(String::as_str))
            .collect::<Vec<_>>();
        if element_types.is_empty() {
            continue;
        }
        if element_types.contains(&"java.lang.Object") {
            return Verdict::Accepted;
        }
        for needle in needles {
            for element in &element_types {
                match catalog::can_convert(needle, element) {
                    Ok(TypeRelation::Compatible) => return Verdict::Accepted,
                    Ok(TypeRelation::Unknown) | Err(_) => unresolved = true,
                    Ok(TypeRelation::Incompatible) => {}
                }
            }
        }
    }
    if unresolved {
        Verdict::Unresolved
    } else {
        Verdict::Rejected
    }
}

fn direct_verdict(haystacks: &[&str], needles: &[&str]) -> Verdict {
    let mut unresolved = false;
    for haystack in haystacks {
        for needle in needles {
            if *haystack == "java.lang.Object" || *needle == "java.lang.Object" {
                unresolved = true;
                continue;
            }
            match catalog::comparator_for_types(haystack, needle) {
                Ok(contract) => match contract.relation {
                    TypeRelation::Compatible => return Verdict::Accepted,
                    TypeRelation::Unknown => unresolved = true,
                    TypeRelation::Incompatible => {}
                },
                Err(_) => unresolved = true,
            }
        }
    }
    if unresolved {
        Verdict::Unresolved
    } else {
        Verdict::Rejected
    }
}
