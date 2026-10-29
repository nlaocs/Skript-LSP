use super::*;

#[derive(Default)]
struct Environment { cancelled: bool, failed: bool }
impl PatternMatchEnvironment for Environment {
    fn resolve_type(&mut self, _: TypeExpressionRequest<'_>) -> Result<TypeExpressionOutcome,String> {
        Ok(TypeExpressionOutcome::default())
    }
    fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl,String> {
        Ok(PatternHookControl::Continue)
    }
    fn cancellation_requested(&mut self) -> bool { self.cancelled }
}
impl ExpressionParseEnvironment for Environment {
    fn parse_expression_leaf(&mut self, _: ExpressionLeafRequest<'_>) -> Result<ExpressionLeafParse,String> {
        Ok(ExpressionLeafParse::default())
    }
    fn state_revision(&self) -> Result<u64,String> {
        if self.failed { Err("owned test lookup failure".into()) } else { Ok(0) }
    }
}

#[test]
fn capture_root_scopes_restore_after_memo_hits_no_match_failure_and_cancel() {
    let snapshot=ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")).unwrap();
    let catalog=snapshot.catalog();
    let registration = |pattern: &str| catalog.syntaxes().iter().find(|syntax|
        matches!(syntax,syntaxes::Syntax::Expression(value) if value.common.patterns.iter().any(|p|p.source==pattern)))
        .unwrap().registration_id().as_str().to_owned();
    let first=BTreeSet::from([registration("dummy direct registry expression")]);
    let second=BTreeSet::from([registration("dummy dynamically registered expression")]);
    assert_ne!(first,second);
    let binding = |roots: Option<&BTreeSet<String>>| RegisteredCaptureBinding {
        capture_index:0,parser_id:HOST_EXPRESSION_PARSER_ID.into(),required:true,
        options:roots.map(|roots|BTreeMap::from([("host.expression.root-registrations".into(),
            serde_json::to_string(roots).unwrap())])).unwrap_or_default(),
    };
    let source=MappedSource::identity("dummy direct registry expression");
    let range=TextRange::new(0,source.virtual_source().len());
    let mut environment=Environment::default();
    let mut session=ExpressionSession::new(catalog,None,&source,&mut environment,
        ExpressionParseContext::default(),ExpressionParserConfig {root_registrations:Some(second.clone()),..Default::default()});
    let parse = |session: &mut ExpressionSession<'_,Environment>, roots: Option<&BTreeSet<String>>|
        session.parse_capture_expression_prefixes(&binding(roots),range,&[],true,true,0,4);
    for _ in 0..2 {
        assert!(!parse(&mut session,Some(&first)).unwrap().candidates.is_empty());
        assert!(parse(&mut session,Some(&second)).unwrap().candidates.is_empty());
        assert!(!parse(&mut session,None).unwrap().candidates.is_empty());
        assert_eq!(session.config.root_registrations,Some(second.clone()));
        assert_eq!(session.root_selection_depth,0);
        assert!(session.parse_prefixes_detailed(range,&[range.end],&[],true,true,0,0).unwrap().candidates.is_empty(),
            "restored enclosing family must not borrow the unrestricted child's memo");
    }
    session.environment.failed=true;
    assert!(parse(&mut session,Some(&first)).is_err());
    assert_eq!(session.config.root_registrations,Some(second.clone()));
    assert_eq!(session.root_selection_depth,0);
    session.environment.failed=false;
    session.environment.cancelled=true;
    assert!(matches!(parse(&mut session,Some(&first)),Err(ExpressionParseError::Cancelled(_))));
    assert_eq!(session.config.root_registrations,Some(second.clone()));
    assert_eq!(session.root_selection_depth,0);
    session.environment.cancelled=false;
    assert!(!parse(&mut session,Some(&first)).unwrap().candidates.is_empty());
}
