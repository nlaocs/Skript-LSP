//! One bounded, stable selection policy shared by every completion producer.
use super::*;

// A soft browsing cost, not a quota: narrow/exact matches bypass this cost.
const FINITE_BROWSE_COST: usize = 1;

type Rank = (bool, u8, std::cmp::Reverse<usize>, usize, bool, usize);
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
    fn rank(
        &self, text: &str, unverified: bool, start: usize, finite_value: bool, label_len: usize,
    ) -> Rank {
        let folded = text.to_lowercase();
        let query = self.source[start..self.offset].to_lowercase();
        let quality = if folded == query {
            0
        } else if folded.starts_with(&query) {
            1
        } else {
            2
        };
        // A one-character query gives little evidence for browsing thousands of
        // finite values. With a longer token, prefer completing its nearest word
        // first (stone before stored...), then break comparable matches by role.
        // Weak matches use the displayed proposal size, so a short fixed prefix
        // does not turn a long argument template into a cheap browsing choice.
        let token_len = query.chars().rev().take_while(|c| word(*c)).count();
        let remaining = if quality == 1 && token_len > 1 {
            folded[query.len()..].chars().take_while(|c| word(*c)).count()
        } else {
            0
        };
        (
            self.demote_unverified && unverified,
            quality,
            std::cmp::Reverse(if quality < 2 { query.chars().count() } else { 0 }),
            if quality == 1 && token_len <= 1 {
                label_len + if finite_value { FINITE_BROWSE_COST } else { 0 }
            } else {
                remaining
            },
            finite_value,
            text.chars().count(),
        )
    }
    /// All retained entries already passed suffix validation. A worse label can
    /// never enter the global top-k. Use the best role as an admissible bound.
    pub fn may_accept(&mut self, label: &str, start: usize) -> bool {
        self.may_accept_matching(label, label, start)
    }
    pub fn may_accept_matching(&mut self, text: &str, label: &str, start: usize) -> bool {
        let rank = self.rank(text, false, start, false, label.chars().count());
        if self.entries.len() == MAX_ITEMS && self.entries.last().unwrap().rank <= rank {
            self.limited = true;
            false
        } else {
            true
        }
    }
    pub fn push(&mut self, item: CompletionItem) {
        self.insert(item, None);
    }
    /// Producers supply the same search text used for eligibility, independently
    /// of presentation labels, parameter signatures and variable delimiters.
    pub fn push_matching(&mut self, item: CompletionItem, text: &str) {
        self.insert(item, Some(text));
    }
    fn insert(&mut self, mut item: CompletionItem, text: Option<&str>) {
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
            .is_some_and(|data| data["continuation"] == true || data["addonProposal"].is_object());
        let finite_value = item.data.as_ref()
            .is_some_and(|data| data["typeValue"].is_object());
        let rank = self.rank(
            text.unwrap_or(&item.label), unverified, start, finite_value,
            item.label.chars().count(),
        );
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

#[cfg(test)]
mod value_ranking_tests {
    use super::*;
    fn candidate(source: &str, label: &str, value: bool) -> CompletionItem {
        CompletionItem {
            label: label.into(), kind: Some(CompletionItemKind::VALUE),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: lsp_types::Range::new(Position::new(0, 0), Position::new(0, source.encode_utf16().count() as u32)),
                new_text: label.into(),
            })),
            data: Some(if value { serde_json::json!({"typeValue": {"registrationId":label}}) }
                else { serde_json::json!({"syntaxKind":"Expression"}) }),
            ..Default::default()
        }
    }
    #[test]
    fn weak_queries_balance_browsing_before_cap_in_either_producer_order() {
        for source in ["", "p"] {
            for values_first in [false, true] {
                let lines=LineIndex::new(source);let mut pool=Pool::new(source,&lines,source.len(),false);
                let expression=candidate(source,"player",false);
                if !values_first { pool.push(expression.clone()); }
                for n in 0..MAX_ITEMS+50 { pool.push(candidate(source,&format!("p{n:04}"),true)); }
                if values_first { pool.push(expression); }
                let (items,limited)=pool.finish();assert!(limited);assert_eq!(items.len(),MAX_ITEMS);
                assert_eq!(items[0].label,"player");assert!(items[0].data.as_ref().unwrap()["typeValue"].is_null());
                assert!(items.iter().any(|i|i.data.as_ref().unwrap()["typeValue"].is_object()));
            }
        }
    }
    #[test]
    fn strong_literal_and_exact_matches_remain_reachable() {
        for (query,value,expression,wanted) in [
            ("sto","stone","stored entity count","stone"),
            ("re","red","result","red"),
            ("p","player","player","expression"),
            ("player","player","player","expression"),
            ("red","red","redirection","red"),
        ] {
            let lines=LineIndex::new(query);let mut pool=Pool::new(query,&lines,query.len(),false);
            pool.push(candidate(query,value,true));pool.push(candidate(query,expression,false));
            let (items,_)=pool.finish();
            if wanted=="expression" { assert!(items[0].data.as_ref().unwrap()["typeValue"].is_null()); }
            else { assert_eq!(items[0].label,wanted); }
        }
    }
    #[test]
    fn producer_search_text_controls_rank_and_admissible_preflight() {
        for (query,label,search) in [("_p","{_public}","_public"),("play","player(a: long.signature.Type)","player"),("é","élève(a: long)","élève")] {
            let lines=LineIndex::new(query);let mut pool=Pool::new(query,&lines,query.len(),false);
            for n in 0..MAX_ITEMS { pool.push(candidate(query,&format!("{search} long proposal {n:04}"),true)); }
            assert!(pool.may_accept_matching(search,label,0));pool.push_matching(candidate(query,label,false),search);
            let (items,_)=pool.finish();assert_eq!(items[0].label,label);
        }
        let source="dark g";let lines=LineIndex::new(source);let mut pool=Pool::new(source,&lines,source.len(),false);
        let mut short=candidate(source,"grey",true);
        if let Some(CompletionTextEdit::Edit(edit))=&mut short.text_edit { edit.range.start=Position::new(0,5); }
        pool.push(short);pool.push(candidate(source,"dark grey",true));
        assert_eq!(pool.finish().0[0].label,"dark grey");
    }
}
