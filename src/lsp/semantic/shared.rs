//! Shared-storage facts and completion candidates are distinct channels.
//! Provider contracts describe effects; no syntax is recognized here.
use super::*;
pub(super) const BINDINGS:&str="skript.semantic.binding-effects";
pub(super) const SCOPE:&str="skript.semantic.shared-scope";
#[derive(Clone,Debug)]
pub(super) struct Candidate {pub ty:InferredType}
impl Candidate {
    pub fn new(ty:&InferredType)->Option<Self> {
        let InferredType::Values {classes,multiple:false,..}=ty else{return None;};
        if classes.is_empty()||classes.len()>16||classes.iter().map(String::len).sum::<usize>()>8192||classes.iter().any(|c|c=="java.lang.Object"||c.is_empty()||c.len()>1024||c.chars().any(char::is_control)) {return None;}
        let mut ty=ty.clone();
        if let InferredType::Values {support,maybe_unset,..}=&mut ty {support.certainty=Certainty::Heuristic;support.candidates_complete=false;*maybe_unset=true;}
        Some(Self {ty})
    }
    pub fn join(left:Option<&Self>,right:Option<&Self>)->Option<Self> {
        match (left,right) {(Some(a),Some(b))=>Self::new(&a.ty.join(&b.ty)),(Some(a),None)|(None,Some(a))=>Self::new(&a.ty),_=>None}
    }
}
#[derive(Deserialize,PartialEq)]#[serde(rename_all="kebab-case")]
enum Local {Preserve,Invalidate}
#[derive(Deserialize,PartialEq)]#[serde(rename_all="kebab-case")]
enum Shared {Preserve,Invalidate,Suspend,Reenter}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]
struct Effects {version:u32,local:Local,shared:Shared}
fn effects(node:&AstNode)->Option<Effects> {
    let effect:Effects=serde_json::from_str(unique_metadata(node,BINDINGS)?).ok()?;
    (effect.version==1).then_some(effect)
}
pub(super) fn local_preserved(node:&AstNode)->bool {effects(node).is_some_and(|e|e.local==Local::Preserve)}
pub(super) fn declared(node:&AstNode)->bool {effects(node).is_some()}
pub(super) fn local_invalidated(node:&AstNode)->bool {effects(node).is_some_and(|e|e.local==Local::Invalidate)}
pub(super) fn shared_preserved(node:&AstNode)->bool {effects(node).is_some_and(|e|e.shared==Shared::Preserve)}
pub(super) fn isolated(node:&AstNode)->bool {
    #[derive(Deserialize)]#[serde(deny_unknown_fields)]
    struct Scope {version:u32,model:String,interference:String}
    unique_metadata(node,SCOPE).and_then(|s|serde_json::from_str::<Scope>(s).ok())
        .is_some_and(|s|s.version==1&&s.model=="serialized-segment"&&s.interference=="excluded-by-provider-contract")
}
pub(super) fn expire(facts:&mut Facts,range:Option<TextRange>) {invalidate_where(facts,range,Invalidation::Shared,|key,_|!key.local);}
pub(super) fn after_operation(node:&AstNode,facts:&mut Facts,range:Option<TextRange>) {
    if local_invalidated(node) {invalidate_where(facts,range,Invalidation::Provider,|key,_|key.local);}
    if !shared_preserved(node) {expire(facts,range);}
}
pub(super) fn candidate_bound(fact:&Fact)->Option<(&BTreeSet<String>,bool)> {
    let InferredType::Values {classes,multiple:false,maybe_unset,..}=&fact.candidate.as_ref()?.ty else{return None;};
    (!classes.is_empty()&&classes.len()<=16).then_some((classes,*maybe_unset))
}
pub(super) fn global_write(identity:&Identity,ty:InferredType,planned:Option<&transfer::Output>,facts:&mut Facts) {
    if facts.len()>=4096&&!facts.contains_key(&identity.key){return;}
    let candidate=planned.and_then(|o|transfer::inferred(&o.value)).as_ref().and_then(Candidate::new).or_else(||Candidate::new(&ty));
    if matches!(ty,InferredType::Unknown)&&candidate.is_none(){facts.remove(&identity.key);return;}
    facts.insert(identity.key.clone(),Fact {ty,definitions:Vec::new(),alias:identity.alias.clone(),list:false,history:None,candidate});
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ty(name:&str)->InferredType {InferredType::Values {classes:BTreeSet::from([name.into()]),multiple:false,maybe_unset:false,support:TypeSupport::inferred()}}
    fn key(local:bool)->VariableKey {VariableKey {local,scope:if local {1}else{0},context:0,name:"value".into()}}
    fn fact(name:&str)->Fact {Fact {ty:ty(name),definitions:Vec::new(),alias:None,list:false,history:None,candidate:None}}
    #[test]
    fn shared_barrier_preserves_locals_but_retains_no_global_proof() {
        let mut facts=Facts::from([(key(true),fact("java.lang.String")),(key(false),fact("java.lang.Long"))]);
        expire(&mut facts,None);
        assert!(facts[&key(true)].ty.proven_types().is_some());
        assert!(matches!(facts[&key(false)].ty,InferredType::Unknown));
        let candidate=&facts[&key(false)].candidate.as_ref().unwrap().ty;
        assert!(candidate.proven_types().is_none());
        assert!(transfer::bound(candidate).is_none());
        assert!(candidate.exclusion_types().is_none());
        assert!(matches!(candidate,InferredType::Values {maybe_unset:true,..}));
        let InferredType::Values {support,..}=candidate else{panic!()};
        assert_eq!(support.certainty,Certainty::Heuristic);assert!(!support.candidates_complete);
    }
    #[test]
    fn unavailable_source_withdraws_local_shared_and_completion_only_candidates() {
        let mut candidate_key=key(false);candidate_key.name="candidate-only".into();
        let mut candidate_only=fact("java.lang.Long");
        candidate_only.candidate=Candidate::new(&candidate_only.ty);
        candidate_only.ty=InferredType::Unknown;
        let mut facts=Facts::from([(key(true),fact("java.lang.String")),(key(false),fact("java.lang.Long")),(candidate_key.clone(),candidate_only)]);
        facts.get_mut(&key(true)).unwrap().definitions.push(TextRange::new(0,4));
        facts.get_mut(&key(false)).unwrap().definitions.push(TextRange::new(5,9));
        let mut suspended=facts.clone();expire(&mut suspended,None);
        assert!(suspended[&key(true)].ty.proven_types().is_some());
        assert!(candidate_bound(&suspended[&key(false)]).is_some());
        assert!(candidate_bound(&suspended[&candidate_key]).is_some());
        invalidate(&mut facts,None,Invalidation::Source);
        for fact in facts.values() {
            assert!(matches!(fact.ty,InferredType::Unknown));
            assert!(fact.definitions.is_empty());assert!(fact.candidate.is_none());
            assert!(transfer::bound(&fact.ty).is_none());
        }
        assert!(!facts.contains_key(&candidate_key),"candidate-only knowledge is withdrawn, not historical proof");
        assert!(facts[&key(true)].history.is_some());
        assert!(facts[&key(false)].history.is_some());
    }
    #[test]
    fn candidates_join_across_branches_but_never_initialize_a_fact() {
        let mut left=Facts::new();let id=Identity {key:key(false),alias:None,list:false,edit_range:None};
        let output=transfer::Output {value:transfer::TypeBound {types:vec!["java.lang.String".into()],complete:true,multiple:false,maybe_unset:false},
            source:transfer::SourceUpdate::Unknown,source_writeback:None,effects:transfer::BindingEffects::Invalidate};
        global_write(&id,InferredType::Unknown,Some(&output),&mut left);
        let one=join_facts(&left,&Facts::new());assert!(candidate_bound(&one[&id.key]).is_some());assert!(one[&id.key].ty.proven_types().is_none());
        let mut right=left.clone();right.get_mut(&id.key).unwrap().candidate=Candidate::new(&ty("java.lang.Long"));
        let both=join_facts(&left,&right);assert_eq!(candidate_bound(&both[&id.key]).unwrap().0.len(),2);
        assert!(both[&id.key].ty.proven_types().is_none());
    }
}
