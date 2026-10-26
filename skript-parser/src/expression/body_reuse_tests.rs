use super::*;

#[test]
fn certified_body_work_keeps_document_budgets_and_releases_resources() {
    let fixture=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
    let catalog=ssg::load(fixture).unwrap().into_catalog();
    let source=MappedSource::identity("x");
    let mut environment=NoopExpressionEnvironment;
    let mut session=ExpressionSession::new(&catalog,None,&source,&mut environment,
        ExpressionParseContext::default(),ExpressionParserConfig {max_candidates:3,max_memo_entries:3,..ExpressionParserConfig::default()});
    let key=MemoKey {range:TextRange::new(0,1),candidate_ends:vec![1],expected_type_id:0,
        allow_literals:true,allow_expressions:true,time:0,allow_lists:true,
        context:ExpressionParseContext::default(),state_revision:0};
    session.memo.insert(key,PrefixParse::default());
    let before=session.body_work();
    let after=crate::StructureBodyWork {candidates:2,memo_entries:3,next_resolution_id:10,next_preparation_attempt:3};
    assert!(session.body_work_is_valid(before,after));
    assert!(!session.body_work_is_valid(before,crate::StructureBodyWork {memo_entries:4,..after}));
    session.apply_body_work(before,after);
    assert_eq!(session.body_work(),after);
    session.release_body_resources(TextRange::new(0,1));
    assert!(session.memo.is_empty());
    assert_eq!(session.body_work(),after,"releasing resources must not forgive logical quota");
    assert!(session.count_candidate().is_ok());
    assert!(matches!(session.count_candidate(),Err(ExpressionParseError::CandidateLimit {limit:3})));
    assert!(!session.body_work_is_valid(before,after),"a stale prefix cannot borrow old work");
}

#[test]
fn default_environment_does_not_enable_body_reuse() {
    assert!(!NoopExpressionEnvironment.structure_body_reuse_enabled());
}
