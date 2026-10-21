//! Parameter bindings are provider declarations, separate from reference history.
use super::*;
use crate::lsp::public_data::ParameterBindings;

pub(super) struct Parameter {
    pub source: String,
    pub name: String,
    pub class_name: String,
    pub single: bool,
    pub list: bool,
    pub range: TextRange,
}

pub(super) fn project(
    bindings: ParameterBindings,
    source: &str,
    header: TextRange,
    virtual_start: u64,
) -> Option<Vec<Parameter>> {
    let mut names = HashSet::new();
    let mut accepted = Vec::new();
    source.get(header.start..header.end)?;
    for binding in bindings.parameters {
        let name = binding.identity.name;
        if binding.identity.scope != "local"
            || binding.identity.resolution != "static"
            || name.is_empty()
            || !names.insert(name.clone())
            || binding.class_name.is_empty()
            || binding.start >= binding.end
        {
            return None;
        }
        let relative = |offset: u64| {
            if bindings.version == 1 {
                offset.checked_sub(virtual_start)
            } else {
                Some(offset)
            }
        };
        let start = header
            .start
            .checked_add(usize::try_from(relative(binding.start)?).ok()?)?;
        let end = header
            .start
            .checked_add(usize::try_from(relative(binding.end)?).ok()?)?;
        if end > header.end || source.get(start..end).is_none() {
            return None;
        }
        // Runtime identities are already initialized. VariableString source
        // doubles literal percent signs rather than treating them as expressions.
        if name.contains(['{', '}', '\r', '\n']) {
            continue;
        }
        accepted.push(Parameter {
            source: format!("{{_{}}}", name.replace('%', "%%")),
            name,
            class_name: binding.class_name,
            single: binding.single,
            list: binding.identity.list,
            range: TextRange::new(start, end),
        });
    }
    Some(accepted)
}

#[cfg(test)]
#[path = "parameter_tests.rs"]
mod tests;
