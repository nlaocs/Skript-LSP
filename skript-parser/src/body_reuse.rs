//! Portable, already-adopted Structure bodies. This module grants no reuse
//! authority: the environment must certify source, dependencies and ownership.
//! In particular a cloned opaque effect is never a portable resource.
use crate::*;

/// Absolute logical native counters at a top-level body boundary (not deltas).
/// Reserved memo slots remain
/// charged even when a certified body no longer retains its transient memo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructureBodyWork {
    pub candidates: usize,
    pub memo_entries: usize,
    pub next_resolution_id: u64,
    pub next_preparation_attempt: u64,
}

/// Current native input. All headers and declarations have already executed.
pub struct StructureBodyReuseRequest<'a> {
    pub source: &'a MappedSource,
    pub tree: &'a RawTree,
    pub candidate: &'a StructureCandidate,
    pub context: &'a ExpressionParseContext,
    pub body_mode: StructureBodyMode,
    pub work: StructureBodyWork,
    pub parent_attempt: PreparationAttemptId,
    pub ambiguous: bool,
    pub config: ExpressionParserConfig,
}

/// A certified body result, not a guest-supplied AST replacement.
#[derive(Debug, Clone)]
pub struct StructureBodyReplay {
    pub before: StructureBodyWork,
    pub body: PortableStructureBody,
    pub post_context: ExpressionParseContext,
    pub work: StructureBodyWork,
    /// Exact enter-time proposals, one per adopted Section in native preorder.
    /// Missing, duplicated or synthetic owners cannot certify a body replay.
    pub preparations: Vec<ReusedSectionPreparation>,
}

/// A portable, provisional Section header. Native replay still validates its
/// current owner and registers the event through the current session.
#[derive(Debug, Clone)]
pub struct ReusedSectionPreparation {
    pub candidate: SectionCandidate,
    pub attempt_id: PreparationAttemptId,
    pub parent_attempt_id: Option<PreparationAttemptId>,
}

/// The default environment leaves the ordinary parser completely unchanged.
#[derive(Debug, Clone)]
pub enum StructureBodyPlan {
    Parse,
    Observe,
    Reuse(StructureBodyReplay),
}

/// Which consumed resource the environment is certifying for removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumedBodyResource {
    Expression,
    Pattern,
}

/// Data-only body with no deferred effects, native Function handle, opaque
/// attachment, or unsuccessful/ambiguous nested candidate. The environment
/// must additionally prove normal (non-recovery) native execution: a Condition
/// no longer carries the matcher's original recovery trace at this boundary.
#[derive(Debug, Clone)]
pub struct PortableStructureBody {
    body: StructureBody,
    metadata: Vec<(String, String)>,
}

impl PortableStructureBody {
    /// Freeze only a completely accepted body. The callback must establish that
    /// a known resource has already been consumed in the CURRENT transaction;
    /// a type-name match alone is insufficient. Failure leaves `body` untouched.
    pub fn freeze(
        body: &StructureBody,
        consumed: &mut impl FnMut(ConsumedBodyResource, &ExpressionEffects) -> bool,
    ) -> Option<Self> {
        let StructureBody::Trigger(_) = body else { return None; };
        let mut result = Self { body: body.clone(), metadata: Vec::new() };
        let StructureBody::Trigger(nodes) = &mut result.body else { unreachable!() };
        let mut freeze = Freeze { consumed, metadata: &mut result.metadata };
        freeze.nodes(nodes)?;
        Some(result)
    }

    pub fn into_body(self) -> StructureBody { self.body }
    pub fn body(&self) -> &StructureBody { &self.body }
    /// Includes summaries and Section contexts, not just visible AST metadata.
    pub fn metadata(&self) -> &[(String, String)] { &self.metadata }
    /// Every adopted Section must have exactly one real enter-time proposal,
    /// in native preorder. This checks identity/completeness only; the caller
    /// must separately validate current source, parents, attempts and authority.
    pub fn matches_preparations(&self, preparations:&[ReusedSectionPreparation]) -> bool {
        let StructureBody::Trigger(nodes)=&self.body else {return false;};
        fn next_section<'a>(pending:&mut Vec<&'a SectionBodyNode>)->Option<&'a SectionCandidate> {
            while let Some(node)=pending.pop() {
                if let SectionBodyNode::Section(matches)=node {
                    let candidate=matches.selected.as_ref()?;
                    pending.extend(candidate.body.iter().rev());
                    return Some(candidate);
                }
            }
            None
        }
        let mut pending=nodes.iter().rev().collect();
        let mut owners=std::collections::BTreeSet::new();
        for preparation in preparations {
            let Some(final_candidate)=next_section(&mut pending) else {return false;};
            let candidate=&preparation.candidate;
            if candidate.raw_node_id!=final_candidate.raw_node_id
                || candidate.matched.kind!=MatchSyntaxKind::Section
                || candidate.matched.definition_id!=final_candidate.matched.definition_id
                || candidate.matched.registration_id!=final_candidate.matched.registration_id
                || candidate.matched.pattern_index!=final_candidate.matched.pattern_index
                || candidate.body_mode!=final_candidate.body_mode
                || !owners.insert(candidate.raw_node_id) {return false;}
        }
        next_section(&mut pending).is_none()
    }
    /// Apply the same resource check to a provisional editing header.
    pub fn freeze_preparation(
        candidate: &SectionCandidate,
        consumed: &mut impl FnMut(ConsumedBodyResource, &ExpressionEffects) -> bool,
    ) -> Option<(SectionCandidate,Vec<(String,String)>)> {
        let mut candidate=candidate.clone();
        let mut metadata=Vec::new();
        Freeze {consumed,metadata:&mut metadata}.section(&mut candidate)?;
        Some((candidate,metadata))
    }
}

struct Freeze<'a, F> {
    consumed: &'a mut F,
    metadata: &'a mut Vec<(String, String)>,
}

impl<F: FnMut(ConsumedBodyResource, &ExpressionEffects) -> bool> Freeze<'_, F> {
    fn metadata(&mut self, map: &std::collections::BTreeMap<String, String>) {
        self.metadata.extend(map.iter().map(|(k,v)| (k.clone(),v.clone())));
    }
    fn resource(&mut self, kind: ConsumedBodyResource, value: &mut Option<ExpressionEffects>) -> Option<()> {
        if let Some(effect) = value.as_ref() { if !(self.consumed)(kind, effect) { return None; } }
        *value = None;
        Some(())
    }
    fn matched(&mut self, value: &mut CandidateMatch) -> Option<()> {
        self.resource(ConsumedBodyResource::Pattern, &mut value.extension_state)?;
        if !value.matched.recovered_failures.is_empty() { return None; }
        Some(())
    }
    fn expression(&mut self, node: &mut ExpressionNode) -> Option<()> {
        if node.function.is_some() || matches!(node.kind,ExpressionNodeKind::Function {..}|ExpressionNodeKind::NativeFunction {..}) { return None; }
        self.resource(ConsumedBodyResource::Expression, &mut node.effects)?;
        self.metadata(&node.metadata);
        for child in &mut node.children { self.expression(child)?; }
        self.captures(&mut node.routed_captures)
    }
    fn condition(&mut self, node: &mut ConditionNode) -> Option<()> {
        self.metadata(&node.metadata);
        for expression in &mut node.expressions { self.expression(expression)?; }
        for child in &mut node.children { self.condition(child)?; }
        Some(())
    }
    fn captures(&mut self, captures: &mut [ParsedCapture]) -> Option<()> {
        for capture in captures {
            let result = &mut capture.result;
            if result.status!=ParsedCaptureStatus::Success || !result.attachments.is_empty() || !result.diagnostics.is_empty() { return None; }
            if let Some(summary) = &result.summary { self.metadata(&summary.metadata); }
            match result.value.as_mut()? {
                ParsedCaptureValue::Expression(value) => self.expression(value)?,
                ParsedCaptureValue::Condition(value) => self.condition(value)?,
                ParsedCaptureValue::Effect(value) => self.effect(value)?,
                ParsedCaptureValue::Section(value) => self.section(value)?,
                // Event-header and open opaque routes remain on the cold path.
                ParsedCaptureValue::Event(_) => return None,
                ParsedCaptureValue::Raw(_) => {},
            }
        }
        Some(())
    }
    fn effect(&mut self, value: &mut EffectCandidate) -> Option<()> {
        self.matched(&mut value.matched)?;
        self.metadata(&value.metadata);
        self.captures(&mut value.parsed_captures)
    }
    fn section(&mut self, value: &mut SectionCandidate) -> Option<()> {
        self.matched(&mut value.matched)?;
        self.metadata(&value.metadata);
        self.metadata(&value.body_context.values);
        for frame in &value.body_context.section_stack {
            self.metadata(&frame.metadata);
            for capture in &frame.captures {
                self.metadata(&capture.metadata);
            }
        }
        self.captures(&mut value.parsed_captures)?;
        self.nodes(&mut value.body)
    }
    fn nodes(&mut self, nodes: &mut [SectionBodyNode]) -> Option<()> {
        for node in nodes {
            match node {
                SectionBodyNode::Trivia(_) => {},
                SectionBodyNode::Unclaimed(_) => return None,
                SectionBodyNode::Effect(matches) => {
                    if !matches!(matches.native_function,NativeFunctionMatch::NotCall) || matches.unknown.is_some() || !matches.alternatives.is_empty() { return None; }
                    self.effect(matches.selected.as_mut()?)?;
                },
                SectionBodyNode::Condition { matches, .. } => {
                    if matches.unknown.is_some() || !matches.alternatives.is_empty() { return None; }
                    self.condition(&mut matches.selected.as_mut()?.node)?;
                },
                SectionBodyNode::Section(matches) => {
                    if matches.unknown.is_some() || !matches.alternatives.is_empty() || !matches.diagnostics.is_empty() { return None; }
                    self.section(matches.selected.as_mut()?)?;
                },
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_body_rejects_recovery_and_nontrigger_without_consulting_resources() {
        let mut calls=0;
        let mut consumed=|_,_: &ExpressionEffects| { calls+=1; true };
        assert!(PortableStructureBody::freeze(&StructureBody::None,&mut consumed).is_none());
        let body=StructureBody::Trigger(vec![SectionBodyNode::Unclaimed(RawNodeId::new(1))]);
        assert!(PortableStructureBody::freeze(&body,&mut consumed).is_none());
        assert_eq!(calls,0);
        assert!(matches!(body,StructureBody::Trigger(_)));
    }
    #[test]
    fn empty_complete_body_is_data_only_and_roundtrips() {
        let body=StructureBody::Trigger(Vec::new());
        let result=PortableStructureBody::freeze(&body,&mut |_,_| false).unwrap();
        assert!(result.metadata().is_empty());
        assert_eq!(result.into_body(),body);
    }
    fn expression() -> ExpressionNode {
        let range=TextRange::new(0,1);
        ExpressionNode {effects:Some(ExpressionEffects::new(7u32)),kind:ExpressionNodeKind::Literal {parser_id:"fixture".into()},function:None,
            span:MatchSpan {local_range:range,mapped:MappedSource::identity("1").map_range(range).unwrap()},return_type:None,possible_return_types:Vec::new(),possible_return_types_state:syntaxes::PossibleReturnTypesState::Complete,
            multiplicity:None,captures:Vec::new(),tags:Vec::new(),mark:0,children:Vec::new(),routed_captures:Vec::new(),public_data:Vec::new(),metadata:std::collections::BTreeMap::from([("fixture".into(),"preserved".into())])}
    }
    #[test]
    fn nested_opaque_resources_require_current_consumption_and_leave_input_intact() {
        let original=expression();
        let mut parent=expression();parent.effects=None;parent.children.push(original.clone());
        let mut metadata=Vec::new();
        let mut reject=|_,_: &ExpressionEffects| false;
        assert!(Freeze {consumed:&mut reject,metadata:&mut metadata}.expression(&mut parent.clone()).is_none());
        let mut consumed=|kind,e:&ExpressionEffects| kind==ConsumedBodyResource::Expression && e.downcast_ref::<u32>()==Some(&7);
        let mut portable=parent.clone();
        Freeze {consumed:&mut consumed,metadata:&mut metadata}.expression(&mut portable).unwrap();
        assert!(portable.children[0].effects.is_none());
        assert!(parent.children[0].effects.is_some());
        assert_eq!(portable.children[0].metadata,original.metadata);
    }
    #[test]
    fn pattern_resources_are_not_confused_with_expression_effects() {
        let mut candidate=CandidateMatch {extension_state:Some(ExpressionEffects::new(7u32)),kind:MatchSyntaxKind::Effect,
            definition_id:"fixture".into(),registration_id:"fixture#0".into(),priority:0,registration_order:0,literal_anchor:false,pattern_index:0,pattern:"x".into(),
            matched:PatternMatch {span:expression().span,captures:Vec::new(),tags:Vec::new(),mark:0,marks:Vec::new(),recovered_failures:Vec::new()}};
        let mut metadata=Vec::new();
        assert!(Freeze {consumed:&mut |kind,_: &ExpressionEffects|kind==ConsumedBodyResource::Expression,metadata:&mut metadata}.matched(&mut candidate.clone()).is_none());
        Freeze {consumed:&mut |kind,e: &ExpressionEffects|kind==ConsumedBodyResource::Pattern && e.downcast_ref::<u32>()==Some(&7),metadata:&mut metadata}.matched(&mut candidate).unwrap();
        assert!(candidate.extension_state.is_none());
    }
}
