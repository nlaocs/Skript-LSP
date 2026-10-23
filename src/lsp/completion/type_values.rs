//! Finite parser-backed values from the same immutable index as native literals.
//! A suggestion is not proof of runtime parser success or addon guard approval.
use super::*;
use skript_parser::expression_types_are_incompatible;
use syntaxes::{Multiplicity, TypeLiteralSource};

const MAX_RECORDS: usize = 16_384;
const MAX_BYTES: usize = 2 * 1024 * 1024;

impl Document {
    pub(super) fn push_type_values(
        &self,
        line: &Line,
        offset: usize,
        word_start: usize,
        word_end: usize,
        references: bool,
        pool: &mut ranking::Pool<'_>,
    ) -> Result<bool, String> {
        let Some(index) = self.index.typing.as_ref().filter(|i| i.current()) else {
            return Ok(false);
        };
        let (start_offsets, mut limited) = self.type_literal_starts(line, word_start);
        let mut budget = continuation::Budget::new();
        let mut records = 0;
        let mut bytes = 0;
        let mut seen = HashSet::new();
        for start in start_offsets {
            if offset - start > 256 {
                limited = true;
                continue;
            }
            let query = &self.source[start..offset];
            let mut options = index.catalog.type_literal_prefix_matches(query).peekable();
            if options.peek().is_none() {
                continue;
            }
            let (ends, ends_limited) = self.type_value_ends(line, start, word_end, &index.catalog);
            limited |= ends_limited;
            for end in ends {
                let Some((expected, suffix_pending)) = self.literal_value_targets_at(
                    line, TextRange::new(start, end), references, &mut budget,
                ) else {
                    continue;
                };
                // Compute compatibility once per registration, not once per alias.
                let compatible = index.catalog.types().filter(|ty| {
                    ty.has_parser && !expression_types_are_incompatible(
                        &index.catalog, std::slice::from_ref(&ty.original_class), true,
                        Some(Multiplicity::Single), &expected,
                    )
                }).map(|ty| ty.registration_id.as_str()).collect::<HashSet<_>>();
                for (text, value) in index.catalog.type_literal_prefix_matches(query) {
                    records += 1;
                    bytes += text.len() + value.canonical_value.len();
                    if records > MAX_RECORDS || bytes > MAX_BYTES {
                        return Ok(true);
                    }
                    let ty = value.type_info;
                    let registration = ty.registration_id.as_str();
                    if !compatible.contains(registration)
                        || !starts(text, query)
                        || !supported(text, value.source)
                        || !text.ends_with(&self.source[offset..end])
                        || !seen.insert((registration, text, start, end))
                        || !pool.may_accept(text, start)
                    {
                        continue;
                    }
                    let identity = format!("type-value:{registration}:{text}");
                    let source = match value.source {
                        TypeLiteralSource::ParserPattern => "parser data",
                        TypeLiteralSource::Supplier => "registered value",
                        TypeLiteralSource::EnumConstant => "enum value",
                        TypeLiteralSource::Alias => "registered alias",
                    };
                    let mut item = self.item(
                        text, text,
                        &format!("Value / {} / {}", ty.noun.singular, ty.addon.name),
                        &identity, CompletionItemKind::VALUE,
                        TextRange::new(start, end), false,
                    )?;
                    item.data = Some(serde_json::json!({
                        "syntaxIdentity": identity,
                        "typeValue": {
                            "registrationId": registration,
                            "definitionId": ty.definition_id.as_str(),
                            "className": ty.original_class.as_str(),
                            "codeName": ty.code_name.as_str(),
                            "source": source,
                            "canonicalValue": value.canonical_value,
                            "pluralSpelling": value.plural,
                            "contextVerified": false,
                            "syntaxSuffixPending": suffix_pending,
                        }
                    }));
                    item.documentation = Some(super::super::hover_text::markup(
                        super::super::hover_text::render_cached(
                            "Value",
                            &[
                                ("Type", ty.noun.singular.clone()),
                                ("Source", format!("{source} ({})", ty.addon.name)),
                                ("Canonical value", value.canonical_value.into()),
                                ("Syntax", if suffix_pending {
                                    "Structural prefix; required suffix not entered"
                                } else {
                                    "Complete structural path; semantic guards unverified"
                                }.into()),
                                ("Status", "Finite parser data; not excluded by expected-type rules. Runtime parser and addon guards unverified.".into()),
                            ], None, &self.index.documentation_format,
                        ), &self.index.documentation_format,
                    ));
                    pool.push(item);
                }
            }
            limited |= budget.exhausted();
        }
        Ok(limited || budget.exhausted())
    }

    // A multiword literal may already extend to the right of the cursor.
    // Consider only exact finite spellings, never arbitrary following grammar.
    // Pool::push still preserves the complete existing suffix unchanged.
    fn type_value_ends(
        &self,
        line: &Line,
        start: usize,
        word_end: usize,
        catalog: &Catalog,
    ) -> (Vec<usize>, bool) {
        let mut ends = vec![word_end];
        for (relative, c) in self.source[start..line.range.end]
            .char_indices().chain(std::iter::once((line.range.end - start, ' ')))
        {
            if relative > 256 { break }
            let end = start + relative;
            if end <= word_end || word(c) || self.source[..end].ends_with(char::is_whitespace)
                || catalog.type_literal_matches(&self.source[start..end]).next().is_none() {
                continue;
            }
            if ends.len() == 6 { return (ends, true) }
            ends.push(end);
        }
        (ends, false)
    }

    /// Shared bounded candidate spans for registered type names and values.
    pub(super) fn type_literal_starts(&self, line: &Line, word_start: usize) -> (Vec<usize>, bool) {
        let mut starts = vec![word_start];
        let mut scan_start = word_start.saturating_sub(256).max(line.code_start);
        while !self.source.is_char_boundary(scan_start) {
            scan_start += 1;
        }
        let mut earlier = self.source[scan_start..word_start].char_indices().rev().filter_map(|(i, c)| {
            let absolute = scan_start + i;
            (!c.is_whitespace() && (absolute == line.code_start
                || self.source[..absolute].ends_with(char::is_whitespace))).then_some(absolute)
        });
        starts.extend(earlier.by_ref().take(5));
        let limited = scan_start > line.code_start || earlier.next().is_some();
        (starts, limited)
    }
}

fn supported(text: &str, source: TypeLiteralSource) -> bool {
    !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
        // ParserPattern records may contain grammar/regex notation. Only plain
        // finite strings are insertion proposals; never insert the pattern itself.
        && (source != TypeLiteralSource::ParserPattern
            || text.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'')))
}

#[cfg(test)]
#[path = "type_value_tests.rs"]
mod tests;
