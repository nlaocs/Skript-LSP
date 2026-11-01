use super::*;
use skript_parser::{ConditionExpressionBindingOrigin, ConditionSemanticDecision, ConditionSemanticRequest,
    ExpressionEffects, RegisteredCaptureBinding, RegisteredSyntaxIdentity};
use syntaxes::{Catalog, CatalogParts, Pattern, Syntax};

fn catalog(fallback: bool) -> Catalog {
    let saved=ssg::load(fixture()).unwrap();let original=saved.catalog();
    let mut condition=original.conditions().find(|c|c.common.patterns.iter()
        .any(|p|p.source=="dummy fixture condition")).unwrap().clone();
    let pattern="check <.+>";
    condition.common.patterns=vec![Pattern {source:pattern.into(),
        parsed:syntax_pattern_parser::syntax::parse(pattern,original.plural_rules()).unwrap()}];
    let mut syntaxes=original.syntaxes().iter().filter(|s|matches!(s,Syntax::Type(t)
        if matches!(t.code_name.as_str(),"string"|"object"))).cloned().collect::<Vec<_>>();
    // Native matching publishes one structurally matched pattern per registration.
    // Competing providers therefore need distinct registration identities.
    for index in 0..if fallback {2} else {1} {
        let mut registered=condition.clone();
        registered.common.registration_order=index;
        registered.common.definition_id=syntaxes::DefinitionId(format!("native-capture:definition:{index}"));
        registered.common.registration_id=syntaxes::RegistrationId(format!("native-capture:registration:{index}"));
        syntaxes.push(Syntax::Condition(registered));
    }
    Catalog::new(CatalogParts {syntaxes,converters:vec![],comparators:vec![],event_values:vec![],
        properties:vec![],operators:vec![],operations:BTreeMap::new(),differences:vec![],classes:vec![],
        aliases:original.aliases().clone(),plural_rules:original.plural_rules().clone(),language:BTreeMap::new()})
}

struct Env {
    parser: &'static str, required: bool, reject_first: bool, restricted: bool,
    cancel_after_child: bool, cancelled: bool, marker: usize, applications: usize,
    savepoints: Vec<usize>, observations: Vec<(usize,usize)>,
}
impl Default for Env {
    fn default()->Self {Self {parser:"host.expression",required:true,reject_first:false,restricted:false,
        cancel_after_child:false,cancelled:false,marker:0,applications:0,savepoints:vec![],observations:vec![]}}
}
impl PatternMatchEnvironment for Env {
    fn allows_regex_pattern(&mut self,_:skript_parser::MatchSyntaxKind,_:&str,_:usize)->Result<bool,String>{Ok(true)}
    fn resolve_type(&mut self,_:TypeExpressionRequest<'_>)->Result<TypeExpressionOutcome,String>{Ok(TypeExpressionOutcome::default())}
    fn dispatch_hook(&mut self,_:PatternHookEvent<'_>)->Result<PatternHookControl,String>{Ok(PatternHookControl::Continue)}
    fn cancellation_requested(&mut self)->bool {self.cancelled}
}
impl ExpressionParseEnvironment for Env {
    fn parse_expression_leaf(&mut self,request:ExpressionLeafRequest<'_>)->Result<ExpressionLeafParse,String>{
        let mut result=LiteralEnvironment.parse_expression_leaf(request)?;
        for value in &mut result.candidates {value.effects=Some(ExpressionEffects::new(1usize));}
        Ok(result)
    }
    fn registered_capture_bindings(&self,syntax:RegisteredSyntaxIdentity<'_>)->Result<Vec<RegisteredCaptureBinding>,String>{
        if syntax.kind!=syntaxes::SyntaxKind::Condition || syntax.registration_id!="native-capture:registration:0"{return Ok(vec![]);}
        let mut options=BTreeMap::from([("expression.expected-types".into(),"java.lang.String".into())]);
        if self.restricted {options.insert("host.expression.root-registrations".into(),"[]".into());}
        Ok(vec![RegisteredCaptureBinding {capture_index:0,parser_id:self.parser.into(),required:self.required,options}])
    }
    fn apply_expression_effects(&mut self,effects:&ExpressionEffects)->Result<(),String>{
        self.applications+=1;
        self.marker=*effects.downcast_ref::<usize>().ok_or("missing test effect")?;
        if self.cancel_after_child {self.cancelled=true;}Ok(())
    }
    fn begin_semantic_candidate(&mut self)->Result<(),String>{self.savepoints.push(self.marker);Ok(())}
    fn finish_semantic_candidate(&mut self,accepted:bool)->Result<(),String>{
        let before=self.savepoints.pop().ok_or("missing semantic savepoint")?;
        if !accepted {self.marker=before;}Ok(())
    }
    fn resolve_condition_candidate(&mut self,request:ConditionSemanticRequest<'_>)->Result<ConditionSemanticDecision,String>{
        let ConditionNodeKind::Registered {ref registration_id,..}=request.candidate.node.kind else {panic!()};
        let registration_index=usize::from(registration_id=="native-capture:registration:1");
        self.observations.push((registration_index,self.marker));
        Ok(if self.reject_first && registration_index==0 {ConditionSemanticDecision::Reject {
            reason:"ordinary competing condition rejects this candidate".into(),diagnostics:vec![]}
        } else {ConditionSemanticDecision::UseCandidate})
    }
    fn state_revision(&self)->Result<u64,String>{Ok(0)}
}
fn run(env:&mut Env,text:&str,fallback:bool)->Result<skript_parser::ConditionMatches,skript_parser::ConditionParseError>{
    let source=MappedSource::identity(text);
    let start=text.find("check").unwrap();
    parse_condition(&catalog(fallback),ConditionParseRequest {source:&source,range:TextRange::new(start,text.len()),
        context:ExpressionParseContext::default()},env,ConditionParserConfig::default())
}

#[test]
fn native_condition_capture_selects_one_current_child_and_preserves_unicode_positions(){
    let text="# 🦀\ncheck \"hello\"";let mut env=Env::default();let parsed=run(&mut env,text,false).unwrap();
    let node=parsed.selected.unwrap().node;
    assert_eq!(node.expressions.len(),1);assert_eq!(node.expression_capture_index(0),Some(0));
    assert_eq!(node.expression_bindings[0].origin,ConditionExpressionBindingOrigin::NativeRegex);
    let start=text.find('"').unwrap();assert_eq!(node.expressions[0].span.mapped.virtual_range,TextRange::new(start,text.len()));
    assert_eq!(node.parsed_capture_refs().count(),1);assert_eq!(env.observations,[(0,1)]);
    assert_eq!(env.marker,1);assert_eq!(env.applications,1,"selected child applies once");
    assert!(env.savepoints.is_empty());
}

#[test]
fn native_condition_required_failure_and_parent_rejection_roll_back_before_alternatives(){
    for (text,reject,expected) in [("check not-an-expression",false,vec![(1,0)]),
        ("check \"hello\"",true,vec![(0,1),(1,0)])] {
        let mut env=Env {reject_first:reject,..Env::default()};let parsed=run(&mut env,text,true).unwrap();let node=parsed.selected.unwrap_or_else(||panic!("{text}: observations={:?}, unknown={:?}",env.observations,parsed.unknown)).node;
        assert!(matches!(node.kind,ConditionNodeKind::Registered {ref registration_id,..} if registration_id=="native-capture:registration:1"));
        assert!(node.expressions.is_empty());assert_eq!(env.observations,expected);
        assert_eq!(env.applications,usize::from(reject),"rejected parent never reapplies its child");
        assert_eq!(env.marker,0);assert!(env.savepoints.is_empty());
    }
}

#[test]
fn native_condition_optional_or_unsupported_routes_do_not_fabricate_a_child(){
    for (parser,required,selected) in [("host.condition",true,false),("host.condition",false,true),
        ("host.expression",false,true)] {
        let mut env=Env {parser,required,..Env::default()};let result=run(&mut env,"check invalid",false).unwrap();
        assert_eq!(result.selected.is_some(),selected);
        if let Some(candidate)=result.selected {assert!(candidate.node.expressions.is_empty());}
        assert_eq!(env.marker,0);assert!(env.savepoints.is_empty());
    }
    let mut env=Env {restricted:true,..Env::default()};
    let result=run(&mut env,"check \"hello\"",false).unwrap();
    assert!(result.selected.is_none(),"foreign literal is not a registered root");
    assert!(result.unknown.unwrap().failure.is_some());assert_eq!(env.marker,0);
}

#[test]
fn native_condition_cancel_after_child_selection_is_not_no_match_and_releases_savepoints(){
    let mut env=Env {cancel_after_child:true,..Env::default()};
    let error=run(&mut env,"check \"hello\"",true).unwrap_err();
    assert!(matches!(error,skript_parser::ConditionParseError::Expression(skript_parser::ExpressionParseError::Cancelled(_))),"{error:?}");
    assert!(env.observations.is_empty());assert_eq!(env.marker,0);assert!(env.savepoints.is_empty());
}
