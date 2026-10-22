//! Source edits consume the same authenticated bindings as references. A known
//! declaration is insufficient when another participant has incomplete coverage.
use super::*;

pub(in crate::lsp) struct RenamePlan {
    pub range: Range,
    pub placeholder: String,
    pub edits: Vec<Location>,
}

/// Build once per navigation query; never rescan all occurrences for each edge.
pub(super) struct FunctionResolver<'a> {
    symbols: HashMap<(&'a str, &'a str), Vec<(&'a Document, &'a FunctionSymbol)>>,
}
impl<'a> FunctionResolver<'a> {
    pub(super) fn new(current: &'a Document, project: &[&'a Document]) -> Self {
        let mut symbols = HashMap::<_, Vec<_>>::new();
        for document in
            std::iter::once(current).chain(project.iter().copied().filter(|d| d.uri != current.uri))
        {
            for symbol in &document.functions {
                symbols
                    .entry((
                        symbol.definition.definition_id.as_str(),
                        symbol.definition.registration_id.as_str(),
                    ))
                    .or_default()
                    .push((document, symbol));
            }
        }
        Self { symbols }
    }
    pub(super) fn resolve(
        &self,
        caller: &Document,
        occurrence: &Occurrence,
    ) -> Option<(&'a Document, &'a FunctionSymbol)> {
        let Reference::Function {
            definition,
            registration,
            ..
        } = &occurrence.reference
        else {
            return None;
        };
        let mut matches = self
            .symbols
            .get(&(definition.as_str(), registration.as_str()))?
            .iter()
            .copied()
            .filter(|(owner, symbol)| caller.reference_binds(occurrence, owner, symbol));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
}

impl Document {
    fn name_edit_is_current(&self) -> bool {
        self.name_edit_certificate
            .as_ref()
            .is_some_and(|proof| proof.is_current())
    }

    pub fn source_matches(&self, source: &str) -> bool {
        self.source.as_ref() == source
    }

    pub(super) fn reference_binds(
        &self,
        occurrence: &Occurrence,
        owner: &Document,
        symbol: &FunctionSymbol,
    ) -> bool {
        let Reference::Function {
            definition,
            registration,
            witness,
        } = &occurrence.reference
        else {
            return false;
        };
        *definition == symbol.definition.definition_id
            && *registration == symbol.definition.registration_id
            && (!symbol.local || self.uri == owner.uri)
            && witness
                .as_ref()
                .is_some_and(|w| w.matches(&symbol.definition, &owner.source_witness))
    }
    pub(super) fn function_at<'a>(
        &'a self,
        position: Position,
        project: &[&'a Document],
    ) -> Option<(&'a Document, &'a FunctionSymbol)> {
        if let Some(occurrence) = self.at(position)
            && matches!(occurrence.reference, Reference::Function { .. })
        {
            return FunctionResolver::new(self, project).resolve(self, occurrence);
        }
        let mut matches = self.functions.iter().filter(|symbol| {
            symbol
                .name_range
                .is_some_and(|r| r.start <= position && position < r.end)
        });
        let first = matches.next()?;
        matches.next().is_none().then_some((self, first))
    }

    pub fn rename_plan(
        &self,
        position: Position,
        project: &[&Document],
        new_name: Option<&str>,
    ) -> Result<Option<RenamePlan>, String> {
        let Some((owner, symbol)) = self.function_at(position, project) else {
            return self.rename_local(position, new_name);
        };
        let documents = std::iter::once(self)
            .chain(project.iter().copied().filter(|d| d.uri != self.uri))
            .collect::<Vec<_>>();
        if documents
            .iter()
            .any(|d| !d.project_complete || !d.rename_complete)
        {
            return Err("Rename requires complete current parsing of every project participant; recovery, unresolved input and source macros are not editable".into());
        }
        if documents.iter().any(|d| !d.name_edit_is_current()) {
            return Err("Current providers have not certified stable static name edits, or their runtime certificate was revoked".into());
        }
        let old_name = &symbol.definition.name;
        if !ascii_identifier(old_name) {
            return Err(
                "The declaration name is outside the certified ASCII identifier edit grammar"
                    .into(),
            );
        }
        if symbol
            .definition
            .metadata
            .get("function.name.edit-grammar")
            .map(String::as_str)
            != Some("ascii-identifier-v1")
        {
            return Err(
                "The declaration producer has not certified a supported name-edit grammar".into(),
            );
        }
        if let Some(new_name) = new_name {
            if !ascii_identifier(new_name) || !owner.function_policy.accepts_name(new_name) {
                return Err(
                    "The new function name is not valid under this document's declaration policy"
                        .into(),
                );
            }
            if documents.iter().any(|d| d.functions.iter().any(|candidate|
                !std::ptr::eq(candidate, symbol) && candidate.definition.name.eq_ignore_ascii_case(new_name)))
                || self.catalog.functions().any(|candidate| candidate.name.eq_ignore_ascii_case(new_name))
                || new_name != old_name && documents.iter().any(|d| d.dependencies.iter().any(|dependency|
                    matches!(dependency, Dependency::FunctionName(name) if name.eq_ignore_ascii_case(new_name))))
            { return Err("The new name collides with a declaration, registered function or existing function lookup".into()) }
        }
        let range = match self.at(position) {
            Some(Occurrence {
                range,
                reference: Reference::Function { .. },
                ..
            }) => self.lines.range(&self.source, *range)?,
            _ => symbol
                .name_range
                .ok_or("The declaration has no authenticated editable name range")?,
        };
        let edits = self.references_at(position, project, true);
        if edits.is_empty() {
            return Err("No authenticated reference ranges are available".into());
        }
        for edit in &edits {
            let document = documents
                .iter()
                .find(|d| d.uri == edit.uri)
                .ok_or("A reference owner is no longer current")?;
            let start = document
                .lines
                .offset(&document.source, edit.range.start)
                .ok_or("Invalid reference start")?;
            let end = document
                .lines
                .offset(&document.source, edit.range.end)
                .ok_or("Invalid reference end")?;
            if document.source.get(start..end) != Some(old_name.as_str()) {
                return Err(
                    "A reference spelling does not have an exact editable declaration binding"
                        .into(),
                );
            }
        }
        if documents.iter().any(|d| !d.name_edit_is_current()) {
            return Err("Provider semantics changed while preparing the rename".into());
        }
        Ok(Some(RenamePlan {
            range,
            placeholder: old_name.clone(),
            edits,
        }))
    }

    fn rename_local(
        &self,
        position: Position,
        new_name: Option<&str>,
    ) -> Result<Option<RenamePlan>, String> {
        let Some(Occurrence {
            reference: Reference::Variable { identity, .. },
            ..
        }) = self.at(position)
        else {
            return Ok(None);
        };
        if !identity.key.local {
            return Ok(None);
        }
        if !self.project_complete
            || !self.rename_complete
            || self.unknown_variable_scopes.contains(&identity.key.scope)
        {
            return Err(
                "Local rename requires complete static reference coverage in its execution scope"
                    .into(),
            );
        }
        if !self.name_edit_is_current() {
            return Err("Current providers have not certified stable static name edits, or their runtime certificate was revoked".into());
        }
        if identity.list || identity.edit_range.is_none() || !ascii_identifier(&identity.key.name) {
            return Err("This variable has no certified scalar identifier edit range; parameters and list families are not yet renameable".into());
        }
        if new_name.is_some_and(|name| !ascii_identifier(name)) {
            return Err(
                "This provider's rename contract accepts only ASCII identifier names".into(),
            );
        }
        let old_name = &identity.key.name;
        let mut edits = Vec::new();
        for occurrence in &self.occurrences {
            let Reference::Variable {
                identity: candidate,
                ..
            } = &occurrence.reference
            else {
                continue;
            };
            if candidate.key.scope != identity.key.scope
                || candidate.key.context != identity.key.context
                || !candidate.key.local
            {
                continue;
            }
            if candidate.alias.is_none() {
                return Err("A potential case alias in this scope cannot be excluded under the unknown runtime case policy".into());
            }
            if candidate.key.name == *old_name {
                let range = candidate.edit_range.ok_or("A bound occurrence has no certified editable name; parameter and named-call bindings require additional source evidence")?;
                if self.source.get(range.start..range.end) != Some(old_name.as_str()) {
                    return Err("A variable name range does not match its static identity".into());
                }
                edits.push(Location::new(
                    self.uri.clone(),
                    self.lines.range(&self.source, range)?,
                ));
            } else if candidate.key.name.eq_ignore_ascii_case(old_name)
                || candidate
                    .key
                    .name
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}::", old_name.to_ascii_lowercase()))
                || new_name.is_some_and(|name| {
                    candidate.key.name.eq_ignore_ascii_case(name)
                        || candidate
                            .key
                            .name
                            .to_ascii_lowercase()
                            .starts_with(&format!("{}::", name.to_ascii_lowercase()))
                })
            {
                return Err("The old or new variable name has a competing static name, list family or potential case alias in this scope".into());
            }
        }
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        edits.dedup();
        if edits.is_empty() {
            return Err("No complete variable edit set is available".into());
        }
        // Source aliases may never make one range edit a different symbol.
        for edit in &edits {
            for occurrence in &self.occurrences {
                if let Reference::Variable {
                    identity: candidate,
                    ..
                } = &occurrence.reference
                    && candidate.key != identity.key
                    && candidate
                        .edit_range
                        .and_then(|range| self.lines.range(&self.source, range).ok())
                        == Some(edit.range)
                {
                    return Err("An edit range is shared by more than one variable identity".into());
                }
            }
        }
        if !self.name_edit_is_current() {
            return Err("Provider semantics changed while preparing the rename".into());
        }
        Ok(Some(RenamePlan {
            range: self
                .lines
                .range(&self.source, identity.edit_range.unwrap())?,
            placeholder: old_name.clone(),
            edits,
        }))
    }
}

fn ascii_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
