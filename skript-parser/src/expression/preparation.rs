//! Reuse only pattern-derived data, never source or semantic parse results.
use super::*;
use syntax_pattern_parser::syntax::ParseResult;

/// Bounded preparation shared by parses using one environment.
///
/// Only the latest complete pattern matrix is retained. Every candidate/pattern
/// boundary, source and parsed AST must match before its pure search tables are
/// reused. Types, overrides, contexts and all parse results remain session-local.
#[derive(Default)]
pub struct ExpressionPreparationCache {
    current: Option<Arc<PreparedExpressionPatterns>>,
}

struct PatternKey {
    source: String,
    parsed: ParseResult,
}

pub(super) struct PreparedExpressionPatterns {
    matrix: Vec<Vec<PatternKey>>,
    prefilters: HashMap<String, Arc<PatternPrefilter>>,
    pub(super) initials: Arc<PatternInitialIndex>,
}

impl ExpressionPreparationCache {
    pub(super) fn prepare(
        &mut self,
        candidates: &[PatternCandidate<'_>],
    ) -> Arc<PreparedExpressionPatterns> {
        if let Some(current) = &self.current
            && current.matches(candidates)
        {
            return Arc::clone(current);
        }
        let filters = pattern_prefilter_index(candidates);
        let initials = Arc::new(pattern_initial_index(candidates, &filters));
        let prepared = Arc::new(PreparedExpressionPatterns {
            matrix: candidates
                .iter()
                .map(|candidate| {
                    candidate
                        .patterns
                        .iter()
                        .map(|pattern| PatternKey {
                            source: pattern.source.to_owned(),
                            parsed: pattern.parsed.clone(),
                        })
                        .collect()
                })
                .collect(),
            prefilters: filters
                .into_iter()
                .map(|(source, filter)| (source.to_owned(), filter))
                .collect(),
            initials,
        });
        self.current = Some(Arc::clone(&prepared));
        prepared
    }
}

impl PreparedExpressionPatterns {
    fn matches(&self, candidates: &[PatternCandidate<'_>]) -> bool {
        self.matrix.len() == candidates.len()
            && self
                .matrix
                .iter()
                .zip(candidates)
                .all(|(known, candidate)| {
                    known.len() == candidate.patterns.len()
                        && known.iter().zip(&candidate.patterns).all(|(key, pattern)| {
                            key.source == pattern.source && &key.parsed == pattern.parsed
                        })
                })
    }

    pub(super) fn local_prefilters<'a>(
        &self,
        candidates: &[PatternCandidate<'a>],
    ) -> HashMap<&'a str, Arc<PatternPrefilter>> {
        let mut filters = HashMap::new();
        for pattern in candidates.iter().flat_map(|candidate| &candidate.patterns) {
            filters
                .entry(pattern.source)
                .or_insert_with(|| Arc::clone(&self.prefilters[pattern.source]));
        }
        filters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn fixture_catalog() -> &'static Catalog {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
            ssg::load(path).unwrap().into_catalog()
        })
    }

    fn parsed(source: &str) -> ParseResult {
        syntax_pattern_parser::syntax::parse(source, fixture_catalog().plural_rules()).unwrap()
    }

    fn candidate<'a>(source: &'a str, parsed: &'a ParseResult) -> PatternCandidate<'a> {
        PatternCandidate {
            kind: crate::MatchSyntaxKind::Expression,
            definition_id: source.into(),
            registration_id: source.into(),
            priority: 0,
            registration_order: 0,
            resolved_order: None,
            patterns: vec![MatchPattern {
                pattern_index: 0,
                source,
                parsed,
            }],
        }
    }

    #[test]
    fn same_pattern_matrix_reuses_only_pure_tables_when_identity_changes() {
        let parsed = parsed("alpha");
        let mut candidates = vec![candidate("alpha", &parsed)];
        let mut cache = ExpressionPreparationCache::default();
        let first = cache.prepare(&candidates);
        candidates[0].registration_id = "replacement".into();
        candidates[0].definition_id = "other-definition".into();
        candidates[0].priority = -10;
        candidates[0].registration_order = 99;
        candidates[0].resolved_order = Some(0);
        candidates[0].patterns[0].pattern_index = 8;
        let next = cache.prepare(&candidates);
        assert!(Arc::ptr_eq(&first, &next));
        // Positions still address the current candidate; identity is not cached.
        let position = next.initials.by_initial[&'a'][0];
        assert_eq!(candidates[position.0].registration_id, "replacement");
        assert_eq!(candidates[position.0].patterns[position.1].pattern_index, 8);
    }

    #[test]
    fn candidate_boundaries_order_insertions_and_removals_invalidate_positions() {
        let alpha = parsed("alpha");
        let beta = parsed("beta");
        let a = candidate("alpha", &alpha);
        let b = candidate("beta", &beta);
        let mut cache = ExpressionPreparationCache::default();
        let separate = cache.prepare(&[a.clone(), b.clone()]);
        assert_eq!(separate.initials.by_initial[&'b'], [(1, 0)]);
        let mut combined = a.clone();
        combined.patterns.extend(b.patterns.clone());
        let grouped = cache.prepare(&[combined]);
        assert!(!Arc::ptr_eq(&separate, &grouped));
        assert_eq!(grouped.initials.by_initial[&'b'], [(0, 1)]);
        let reordered = cache.prepare(&[b.clone(), a.clone()]);
        assert_eq!(reordered.initials.by_initial[&'a'], [(1, 0)]);
        let removed = cache.prepare(&[a.clone()]);
        assert!(!removed.initials.by_initial.contains_key(&'b'));
        let inserted = cache.prepare(&[a, b]);
        assert_eq!(inserted.initials.by_initial[&'b'], [(1, 0)]);
        // Old sessions retain their own immutable tables after replacement.
        assert_eq!(separate.initials.by_initial[&'b'], [(1, 0)]);
    }

    #[test]
    fn identical_source_with_different_parsed_ast_must_not_hit() {
        let alpha = parsed("alpha");
        let beta = parsed("beta");
        let mut cache = ExpressionPreparationCache::default();
        let first = cache.prepare(&[candidate("same-source", &alpha)]);
        let second = cache.prepare(&[candidate("same-source", &beta)]);
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(pattern_prefilter_matches(
            &first.prefilters["same-source"],
            "alpha"
        ));
        assert!(!pattern_prefilter_matches(
            &second.prefilters["same-source"],
            "alpha"
        ));
        assert!(pattern_prefilter_matches(
            &second.prefilters["same-source"],
            "beta"
        ));
        let restored = cache.prepare(&[candidate("same-source", &alpha)]);
        assert!(!Arc::ptr_eq(&first, &restored));
        assert!(pattern_prefilter_matches(
            &restored.prefilters["same-source"],
            "alpha"
        ));
    }

    #[test]
    fn lazy_other_kind_filters_and_local_replacements_do_not_pollute_next_session() {
        let alpha = parsed("alpha");
        let beta = parsed("beta");
        let candidates = vec![candidate("alpha", &alpha)];
        let mut cache = ExpressionPreparationCache::default();
        let prepared = cache.prepare(&candidates);
        let mut local = prepared.local_prefilters(&candidates);
        let other = Arc::new(PatternPrefilter::new(&MatchPattern {
            pattern_index: 0,
            source: "alpha",
            parsed: &beta,
        }));
        local.insert("alpha", Arc::clone(&other));
        local.insert("foreign-kind", other);
        let next = cache.prepare(&candidates);
        assert!(Arc::ptr_eq(&prepared, &next));
        let next_local = next.local_prefilters(&candidates);
        assert!(!next_local.contains_key("foreign-kind"));
        assert!(pattern_prefilter_matches(&next_local["alpha"], "alpha"));
        assert!(!pattern_prefilter_matches(&local["alpha"], "alpha"));
    }

    #[test]
    fn prepared_and_uncached_filters_agree_for_choices_optional_unicode_and_recursion() {
        for source in [
            "[the] alpha",
            "(alpha|beta)",
            "caf\u{e9}",
            "%objects% plus %objects%",
            "<.+> tail",
        ] {
            let parsed = parsed(source);
            let candidates = vec![candidate(source, &parsed)];
            let uncached = pattern_prefilter_index(&candidates);
            let mut cache = ExpressionPreparationCache::default();
            cache.prepare(&candidates);
            let warmed = cache.prepare(&candidates).local_prefilters(&candidates);
            for input in [
                "",
                "alpha",
                "the alpha",
                "beta",
                "CAF\u{c9}",
                "1 plus 2",
                "x tail",
                "other",
            ] {
                assert_eq!(
                    pattern_prefilter_matches(&uncached[source], input),
                    pattern_prefilter_matches(&warmed[source], input),
                    "{source}: {input}"
                );
            }
        }
    }

    #[test]
    #[ignore = "explicit pure preparation microbenchmark; not a latency assertion"]
    fn measure_preparation_cold_matrix_hit_and_local_map() {
        let candidates = catalog_pattern_candidates(fixture_catalog(), SyntaxKind::Expression);
        let started = std::time::Instant::now();
        let uncached = pattern_prefilter_index(&candidates);
        std::hint::black_box(pattern_initial_index(&candidates, &uncached));
        let uncached_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut cache = ExpressionPreparationCache::default();
        let started = std::time::Instant::now();
        std::hint::black_box(cache.prepare(&candidates));
        let cold_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut hit_us = Vec::new();
        let mut local_us = Vec::new();
        for _ in 0..100 {
            let started = std::time::Instant::now();
            let prepared = cache.prepare(&candidates);
            hit_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            let started = std::time::Instant::now();
            std::hint::black_box(prepared.local_prefilters(&candidates));
            local_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        }
        hit_us.sort_by(f64::total_cmp);
        local_us.sort_by(f64::total_cmp);
        println!(
            "preparation_benchmark uncached_ms={uncached_ms} cold_ms={cold_ms} hit_median_us={} hit_p95_us={} local_median_us={} local_p95_us={}",
            hit_us[50], hit_us[95], local_us[50], local_us[95]
        );
    }

    #[test]
    fn duplicate_source_keeps_first_ast_in_both_cold_and_warm_tables() {
        let alpha = parsed("alpha");
        let beta = parsed("beta");
        let a = candidate("same", &alpha);
        let b = candidate("same", &beta);
        let mut cache = ExpressionPreparationCache::default();
        for candidates in [[a.clone(), b.clone()], [b, a]] {
            let uncached = pattern_prefilter_index(&candidates);
            cache.prepare(&candidates);
            let warm = cache.prepare(&candidates).local_prefilters(&candidates);
            for input in ["alpha", "beta"] {
                assert_eq!(
                    pattern_prefilter_matches(&uncached["same"], input),
                    pattern_prefilter_matches(&warm["same"], input)
                );
            }
        }
    }

    #[derive(Default)]
    struct Environment {
        cache: ExpressionPreparationCache,
        use_cache: bool,
        overriding: bool,
    }
    impl PatternMatchEnvironment for Environment {
        fn resolve_type(
            &mut self,
            _: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            Ok(TypeExpressionOutcome::default())
        }
        fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            Ok(PatternHookControl::Continue)
        }
        fn may_override_pattern(&self, _: crate::MatchSyntaxKind, _: &str, _: usize) -> bool {
            self.overriding
        }
    }
    impl ExpressionParseEnvironment for Environment {
        fn expression_preparation_cache(&mut self) -> Option<&mut ExpressionPreparationCache> {
            self.use_cache.then_some(&mut self.cache)
        }
        fn parse_expression_leaf(
            &mut self,
            _: ExpressionLeafRequest<'_>,
        ) -> Result<ExpressionLeafParse, String> {
            Ok(ExpressionLeafParse::default())
        }
        fn state_revision(&self) -> Result<u64, String> {
            Ok(0)
        }
    }

    #[test]
    fn changed_override_decision_is_observed_after_a_preparation_hit() {
        fn retained(environment: &mut Environment) -> usize {
            let parsed = parsed("required");
            let source = MappedSource::identity("other");
            let mut candidates = vec![candidate("required", &parsed)];
            let mut session = ExpressionSession::new(
                fixture_catalog(),
                None,
                &source,
                environment,
                ExpressionParseContext::default(),
                ExpressionParserConfig::default(),
            );
            session
                .retain_viable_patterns(TextRange::new(0, 5), &mut candidates)
                .unwrap();
            candidates.len()
        }
        let mut cached = Environment {
            use_cache: true,
            ..Environment::default()
        };
        let mut uncached = Environment::default();
        let mut original = None;
        for (overriding, expected) in [(false, 0), (true, 1), (false, 0)] {
            cached.overriding = overriding;
            uncached.overriding = overriding;
            assert_eq!(retained(&mut cached), expected);
            assert_eq!(retained(&mut uncached), expected);
            let prepared = cached.cache.current.as_ref().unwrap();
            if let Some(first) = &original {
                assert!(Arc::ptr_eq(first, prepared));
            } else {
                original = Some(Arc::clone(prepared));
            }
        }
    }
}
