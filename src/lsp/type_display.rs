//! Presentation only. Type evidence, completeness and presence remain distinct.
use super::{
    hover_text::type_name,
    semantic::{Certainty, InferredType},
};
use syntaxes::Catalog;

pub(super) fn compact(ty: &InferredType, catalog: &Catalog) -> String {
    let InferredType::Values {
        classes,
        multiple,
        maybe_unset,
        support,
    } = ty
    else {
        return "?".into();
    };
    if classes.is_empty() || !support.candidates_complete || support.certainty == Certainty::Unknown
    {
        return "?".into();
    }
    let mut label = classes
        .iter()
        .map(|class| type_name(catalog, class))
        .collect::<Vec<_>>()
        .join(" | ");
    if classes.len() > 1 && (*multiple || *maybe_unset) {
        label = format!("({label})");
    }
    if *multiple {
        label.push_str("[]");
    }
    if *maybe_unset {
        label.push('?');
    }
    if support.certainty == Certainty::Heuristic {
        label.insert(0, '~');
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::semantic::{Evidence, TypeSupport};
    use std::collections::BTreeSet;
    #[test]
    fn notation_keeps_presence_evidence_union_and_completeness_distinct() {
        let catalog = crate::lsp::test_support::multi_addon_catalog();
        for (heuristic, unset, expected) in [
            (false, false, "number"),
            (false, true, "number?"),
            (true, false, "~number"),
            (true, true, "~number?"),
        ] {
            let mut support = TypeSupport::from(Evidence::Assignment);
            support.certainty = if heuristic {
                Certainty::Heuristic
            } else {
                Certainty::Inferred
            };
            let ty = InferredType::Values {
                classes: BTreeSet::from(["java.lang.Number".into()]),
                multiple: false,
                maybe_unset: unset,
                support,
            };
            assert_eq!(compact(&ty, &catalog), expected);
        }
        let mut union = InferredType::Values {
            classes: BTreeSet::from(["java.lang.Number".into(), "java.lang.String".into()]),
            multiple: false,
            maybe_unset: true,
            support: TypeSupport::inferred(),
        };
        assert_eq!(compact(&union, &catalog), "(number | string)?");
        if let InferredType::Values { support, .. } = &mut union {
            support.candidates_complete = false;
        }
        assert_eq!(compact(&union, &catalog), "?");
        assert_eq!(compact(&InferredType::Unknown, &catalog), "?");
    }
}
