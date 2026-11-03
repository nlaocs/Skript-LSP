//! Predicate results and their short-lived relation to a checked binding.
//! No condition spelling, event name or addon-specific wrapper is interpreted here.
use super::*;
pub(super) const GUARD:&str="skript.semantic.type-guard";
pub(super) const VALUES:&str="skript.semantic.binding-values";
pub(super) const EVIDENCE:&str="skript.semantic.guard-evidence";
const LIMIT:usize=4096;

#[derive(Clone,Deserialize)]
#[serde(rename_all="kebab-case")]
enum Presence {Keep,Present,Absent}
fn keep()->Presence {Presence::Keep}
#[derive(Clone,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Constraint {
    #[serde(default="keep")] presence:Presence,
    #[serde(default)] upper_bounds:Vec<String>,
}
#[derive(Clone,Copy,Deserialize,PartialEq,Eq)]
#[serde(rename_all="kebab-case")]
enum TruthDomain {Boolean,Kleenean}
#[derive(Clone,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Predicate {version:u32,subject_capture:u64,when_true:Constraint,when_false:Constraint,
    #[serde(default)] truth_domain:Option<TruthDomain>,
    #[serde(default)] evidence:Vec<Requirement>}
#[derive(Clone,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Requirement {capture:u64,token:String}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceToken {version:u32,token:String}
impl Predicate {
    fn read(node:&AstNode)->Option<Self> {
        let json=unique_metadata(node,GUARD)?;if json.len()>8192{return None;}
        let value:Self=serde_json::from_str(json).ok()?;
        let valid=|c:&Constraint|c.upper_bounds.len()<=64
            && c.upper_bounds.iter().map(String::len).sum::<usize>()<=8192
            && c.upper_bounds.iter().all(|s|!s.is_empty()&&s.len()<=1024&&!s.chars().any(char::is_control))
            && c.upper_bounds.iter().collect::<BTreeSet<_>>().len()==c.upper_bounds.len()
            && (!matches!(c.presence,Presence::Absent)||c.upper_bounds.is_empty());
        (matches!((value.version,value.truth_domain),(1,None)|(2,Some(_)))&&valid(&value.when_true)&&valid(&value.when_false)
            &&value.evidence.len()<=8&&value.evidence.iter().all(|e|!e.token.is_empty()&&e.token.len()<=1024&&!e.token.chars().any(char::is_control))
            &&value.evidence.iter().map(|e|e.capture).collect::<BTreeSet<_>>().len()==value.evidence.len()).then_some(value)
    }
}
pub(super) fn preserves_values(node:&AstNode)->bool {
    #[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Promise {version:u32,kind:String}
    unique_metadata(node,VALUES).and_then(|s|serde_json::from_str::<Promise>(s).ok())
        .is_some_and(|p|p.version==1&&p.kind=="preserve")
}
#[derive(Clone)]
pub(super) struct ReadLink {identity:Identity,ty:InferredType,epoch:u64}
#[derive(Clone)]
struct Checked {identity:Identity,predicate:Predicate,read:Option<ReadLink>,epoch:u64}
#[derive(Default)]
pub(super) struct State {
    epoch:u64,
    disabled:bool,
    reads:HashMap<u64,ReadLink>,
    checked:HashMap<u64,Checked>,
}
impl State {
    pub fn changed(&mut self) {
        if let Some(next)=self.epoch.checked_add(1){self.epoch=next;}else{self.disabled=true;}
    }
    pub fn reset(&mut self,id:u64){self.reads.remove(&id);self.checked.remove(&id);}
    pub(super) fn boolean_result(&self,id:u64)->bool {
        self.checked.get(&id).filter(|checked|checked.epoch==self.epoch)
            .is_some_and(|checked|checked.predicate.truth_domain==Some(TruthDomain::Boolean))
    }
    pub fn read(&mut self,id:u64,identity:Identity,ty:InferredType) {
        if self.disabled||transfer::bound(&ty).is_none(){return;}
        if self.reads.len()>=LIMIT{self.reads.clear();self.checked.clear();self.changed();}
        self.reads.insert(id,ReadLink {identity,ty,epoch:self.epoch});
    }
    pub fn group(&mut self,id:u64,child:u64){if self.reads.len()>=LIMIT{return;}if let Some(link)=self.reads.get(&child).filter(|r|r.epoch==self.epoch).cloned(){self.reads.insert(id,link);}}
}

// Complete closure is required before treating two unrelated concrete classes
// as disjoint. Interfaces, arrays and unavailable ancestry stay open.
fn relation(catalog:&Catalog,a:&str,b:&str)->(bool,bool,bool) {
    if a==b{return (true,true,false);}
    let complete=catalog.class_lineage(a).is_some()&&catalog.class_lineage(b).is_some();
    if !complete{return (false,false,false);}
    let ab=catalog.is_class_assignable(a,b);let ba=catalog.is_class_assignable(b,a);
    let disjoint=!ab&&!ba&&[a,b].iter().all(|name|catalog.class(name).is_some_and(|c|
        c.kind==syntaxes::ClassKind::Class&&(c.super_class.is_some()||c.name.as_str()=="java.lang.Object")));
    (ab,ba,disjoint)
}
fn constrain(ty:&InferredType,constraint:&Constraint,catalog:&Catalog)->Option<InferredType> {
    let InferredType::Values {classes,multiple:false,maybe_unset,support}=ty else{return None;};
    if !support.is_proof(){return None;}
    let present=match constraint.presence {Presence::Keep=>!*maybe_unset,Presence::Present=>true,Presence::Absent=>{
        if !maybe_unset{return None;}
        let mut support=support.clone();support.evidence.extend([Evidence::TypeGuard,Evidence::DeclaredTransfer]);
        return Some(InferredType::Values {classes:BTreeSet::new(),multiple:false,maybe_unset:true,support});
    }};
    let mut next=classes.clone();
    if !constraint.upper_bounds.is_empty() {
        next.clear();
        for source in classes {for upper in &constraint.upper_bounds {
            let (sub,supertype,disjoint)=relation(catalog,source,upper);
            if sub {next.insert(source.clone());}else if supertype {next.insert(upper.clone());}
            else if !disjoint {next.insert(source.clone());}
        }}
    }
    // An impossible present intersection is not an unset value or a new fact.
    if next.is_empty()&&present{return None;}
    let mut support=support.clone();support.evidence.extend([Evidence::TypeGuard,Evidence::DeclaredTransfer]);
    Some(InferredType::Values {classes:next,multiple:false,maybe_unset:!present,support})
}
impl Engine<'_,'_> {
    pub(super) fn remember_guard(&mut self,node:&AstNode,scope:u64) {
        if node.kind!=SyntaxKind::Condition||!preserves_values(node)||self.guards.disabled{return;}
        let Some(predicate)=Predicate::read(node)else{return;};
        let Some(owner)=metadata_owner(node,GUARD)else{return;};
        if metadata_owner(node,VALUES)!=Some(owner){return;}
        // A static operand is semantic evidence, not an editing/display field.
        // The ordinary accepted-provider filter also authenticates this token.
        for requirement in &predicate.evidence {
            let Some(evidence)=capture_node(node,requirement.capture,&self.nodes).and_then(|id|self.nodes.get(&id).copied())else{return;};
            if metadata_owner(evidence,EVIDENCE)!=Some(owner){return;}
            let Some(token)=unique_metadata(evidence,EVIDENCE).filter(|v|v.len()<=2048)
                .and_then(|v|serde_json::from_str::<EvidenceToken>(v).ok())else{return;};
            if token.version!=1||token.token!=requirement.token{return;}
        }
        let Some(subject)=capture_node(node,predicate.subject_capture,&self.nodes)else{return;};
        let read=self.guards.reads.get(&subject).filter(|r|r.epoch==self.guards.epoch).cloned();
        let identity=if let Some(read)=&read {Some(read.identity.clone())}else{
            let mut target=subject;let mut result=None;
            for _ in 0..8 {
                let Some(n)=self.nodes.get(&target).copied()else{break;};
                if let Some(identity)=self.identity(n,scope){result=Some(identity);break;}
                if n.summary.as_ref().is_some_and(|s|s.kind=="grouped-expression")&&n.children.len()==1 {target=n.children[0];}else{break;}
            }result
        };
        let Some(identity)=identity.filter(|i|!i.list)else{return;};
        if self.guards.checked.len()>=LIMIT{self.guards.checked.clear();}
        self.guards.checked.insert(node.id,Checked {identity,predicate,read,epoch:self.guards.epoch});
    }
    pub(super) fn guard_edge(&self,condition:u64,truth:bool,facts:&mut Facts) {
        let Some(checked)=self.guards.checked.get(&condition).filter(|c|c.epoch==self.guards.epoch)else{return;};
        let constraint=if truth{&checked.predicate.when_true}else{&checked.predicate.when_false};
        if let Some(read)=checked.read.as_ref().filter(|r|r.epoch==self.guards.epoch) {
            if (checked.identity.key.local||self.shared_isolated)
                && let Some(fact)=facts.get_mut(&checked.identity.key)
                && transfer::bound(&fact.ty).is_some()&&fact.ty==read.ty
                && let Some(ty)=constrain(&fact.ty,constraint,&self.document.catalog) {fact.ty=ty;return;}
        }
        // Shared checks never establish a later unrelated shared read. Only add
        // incomplete suggestions; do not exclude prior candidates or prove types.
        if !checked.identity.key.local&&!constraint.upper_bounds.is_empty() {
            let ty=InferredType::Values {classes:constraint.upper_bounds.iter().cloned().collect(),multiple:false,
                maybe_unset:true,support:TypeSupport {certainty:Certainty::Heuristic,candidates_complete:false,evidence:BTreeSet::new()}};
            if let Some(candidate)=shared::Candidate::new(&ty) {
                if facts.len()>=LIMIT&&!facts.contains_key(&checked.identity.key){return;}
                let fact=facts.entry(checked.identity.key.clone()).or_insert_with(||Fact {ty:InferredType::Unknown,
                    definitions:Vec::new(),alias:checked.identity.alias.clone(),list:false,history:None,candidate:None});
                fact.candidate=shared::Candidate::join(fact.candidate.as_ref(),Some(&candidate));
            }
        }
    }
}
