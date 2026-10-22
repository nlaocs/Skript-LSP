//! One bounded, stable selection policy shared by every completion producer.
use super::*;

type Rank = (bool, u8, std::cmp::Reverse<usize>, usize);
struct Entry {
    item: CompletionItem,
    rank: Rank,
    dedup: Option<(String, String)>,
}
pub(super) struct Pool<'a> {
    source: &'a str,
    lines: &'a LineIndex,
    offset: usize,
    demote_unverified: bool,
    entries: Vec<Entry>,
    seen: HashSet<(String, String)>,
    limited: bool,
}
impl<'a> Pool<'a> {
    pub fn new(
        source: &'a str,
        lines: &'a LineIndex,
        offset: usize,
        demote_unverified: bool,
    ) -> Self {
        Self {
            source,
            lines,
            offset,
            demote_unverified,
            entries: Vec::new(),
            seen: HashSet::new(),
            limited: false,
        }
    }
    fn rank(&self, label: &str, unverified: bool, start: usize) -> Rank {
        let folded = label.to_lowercase();
        let query = self.source[start..self.offset].to_lowercase();
        let quality = if folded == query {
            0
        } else if folded.starts_with(&query) {
            1
        } else {
            2
        };
        (
            self.demote_unverified && unverified,
            quality,
            std::cmp::Reverse(if quality < 2 {
                query.chars().count()
            } else {
                0
            }),
            label.chars().count(),
        )
    }
    /// All retained entries already passed suffix validation. A worse label can
    /// never enter the global top-k, regardless of its kind or output type.
    pub fn may_accept(&mut self, label: &str, start: usize) -> bool {
        let rank = self.rank(label, false, start);
        if self.entries.len() == MAX_ITEMS && self.entries.last().unwrap().rank <= rank {
            self.limited = true;
            false
        } else {
            true
        }
    }
    pub fn push(&mut self, mut item: CompletionItem) {
        let Some(CompletionTextEdit::Edit(edit)) = &mut item.text_edit else {
            return;
        };
        let Some(end) = self.lines.offset(self.source, edit.range.end) else {
            return;
        };
        if end > self.offset {
            let suffix = &self.source[self.offset..end];
            let suffix = if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
                escape(suffix)
            } else {
                suffix.to_owned()
            };
            let Some(prefix) = edit.new_text.strip_suffix(&suffix) else {
                return;
            };
            edit.new_text = prefix.to_owned();
            let Ok(position) = self.lines.position(self.source, self.offset) else {
                return;
            };
            edit.range.end = position;
        }
        let Some(start) = self
            .lines
            .offset(self.source, edit.range.start)
            .filter(|start| *start <= self.offset)
        else {
            return;
        };
        // Keep distinct registrations, variants, resolve data and adapter edits.
        let dedup = item
            .data
            .as_ref()
            .and_then(|data| data["syntaxIdentity"].as_str())
            .map(|identity| {
                (
                    identity.to_owned(),
                    serde_json::to_string(&(
                        &item.text_edit,
                        &item.insert_text_format,
                        &item.label,
                        &item.data,
                    ))
                    .expect("completion fields are serializable"),
                )
            });
        if dedup.as_ref().is_some_and(|key| self.seen.contains(key)) {
            return;
        }
        let unverified = item
            .data
            .as_ref()
            .is_some_and(|data| data["continuation"] == true);
        let rank = self.rank(&item.label, unverified, start);
        if self.entries.len() == MAX_ITEMS && self.entries.last().unwrap().rank <= rank {
            self.limited = true;
            return;
        }
        if let Some(key) = &dedup {
            self.seen.insert(key.clone());
        }
        let at = self.entries.partition_point(|entry| entry.rank <= rank);
        self.entries.insert(at, Entry { item, rank, dedup });
        if self.entries.len() > MAX_ITEMS {
            let removed = self.entries.pop().unwrap();
            if let Some(key) = removed.dedup {
                self.seen.remove(&key);
            }
            self.limited = true;
        }
    }
    pub fn finish(self) -> (Vec<CompletionItem>, bool) {
        (
            self.entries.into_iter().map(|entry| entry.item).collect(),
            self.limited,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(label: &str, kind: CompletionItemKind, source: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            kind: Some(kind),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: lsp_types::Range::new(
                    Position::new(0, 0),
                    Position::new(0, source.len() as u32),
                ),
                new_text: label.into(),
            })),
            ..Default::default()
        }
    }
    #[test]
    fn one_global_rank_applies_before_cap_without_candidate_kind_quotas() {
        let source = "p";
        let lines = LineIndex::new(source);
        let mut pool = Pool::new(source, &lines, 1, false);
        for n in 0..MAX_ITEMS + 50 {
            pool.push(item(
                &format!("prefix long proposal {n:03}"),
                CompletionItemKind::SNIPPET,
                source,
            ));
        }
        for kind in [
            CompletionItemKind::VARIABLE,
            CompletionItemKind::FUNCTION,
            CompletionItemKind::SNIPPET,
        ] {
            pool.push(item("player", kind, source));
        }
        let (items, limited) = pool.finish();
        assert!(limited);
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(
            items.iter().take(3).map(|i| i.kind).collect::<Vec<_>>(),
            [
                Some(CompletionItemKind::VARIABLE),
                Some(CompletionItemKind::FUNCTION),
                Some(CompletionItemKind::SNIPPET)
            ]
        );
        assert!(items.iter().take(3).all(|i| i.label == "player"));
        assert_eq!(items[3].label, "prefix long proposal 000");
    }
    #[test]
    fn invalid_suffixes_and_duplicate_edits_cannot_crowd_out_valid_candidates() {
        let source = "player";
        let lines = LineIndex::new(source);
        let mut pool = Pool::new(source, &lines, 1, false);
        for _ in 0..150 {
            pool.push(item("pi", CompletionItemKind::SNIPPET, source));
        }
        let mut player = item("player", CompletionItemKind::SNIPPET, source);
        player.data = Some(serde_json::json!({"syntaxIdentity":"public-player"}));
        for _ in 0..150 {
            pool.push(player.clone());
        }
        let (items, limited) = pool.finish();
        assert!(!limited);
        assert_eq!(items.len(), 1);
        let Some(CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
            panic!()
        };
        assert_eq!(edit.range.end, Position::new(0, 1));
        assert_eq!(edit.new_text, "p");
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    fn candidate(source: &str, label: &str, start: usize, id: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: lsp_types::Range::new(
                    Position::new(0, start as u32),
                    Position::new(0, source.len() as u32),
                ),
                new_text: label.into(),
            })),
            data: Some(serde_json::json!({"syntaxIdentity":id})),
            ..Default::default()
        }
    }
    #[test]
    fn actual_edit_queries_and_matched_extent_define_a_common_rank() {
        let source = "send s";
        let lines = LineIndex::new(source);
        let mut pool = Pool::new(source, &lines, source.len(), false);
        pool.push(candidate(source, "sin(value: number)", 5, "function"));
        assert!(pool.may_accept("send subtitle %object%", 0));
        pool.push(candidate(source, "send subtitle %object%", 0, "statement"));
        let (items, limited) = pool.finish();
        assert!(!limited);
        assert_eq!(items[0].label, "send subtitle %object%");
    }
    #[test]
    fn continuation_is_demoted_only_when_expected_context_is_known() {
        for expected in [false, true] {
            let source = "p";
            let lines = LineIndex::new(source);
            let mut pool = Pool::new(source, &lines, 1, expected);
            let mut continuation = candidate(source, "proxy", 0, "alternative");
            continuation.data.as_mut().unwrap()["continuation"] = serde_json::json!(true);
            pool.push(continuation);
            pool.push(candidate(source, "player", 0, "literal"));
            let (items, limited) = pool.finish();
            assert!(!limited);
            assert_eq!(items[0].label, if expected { "player" } else { "proxy" });
        }
    }
    #[test]
    fn a_duplicate_at_the_full_pool_boundary_does_not_report_truncation() {
        let source = "p";
        let lines = LineIndex::new(source);
        let mut pool = Pool::new(source, &lines, 1, false);
        let mut last = None;
        for i in 0..MAX_ITEMS {
            let item = candidate(source, &format!("proposal{i:04}"), 0, &i.to_string());
            last = Some(item.clone());
            pool.push(item);
        }
        pool.push(last.unwrap());
        let (items, limited) = pool.finish();
        assert_eq!(items.len(), MAX_ITEMS);
        assert!(!limited);
    }
}
