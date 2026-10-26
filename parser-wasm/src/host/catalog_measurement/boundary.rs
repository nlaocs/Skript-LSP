//! Test-only sampled VM transitions. VM includes canonical realloc/codec/cleanup;
//! it is deliberately not labelled pure parser time or pure WIT copy time.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub(super) struct Recorder(Arc<Inner>);
struct Inner { active: AtomicBool, state: Mutex<State> }
struct State {
    stride: u64, seen: BTreeMap<String,u64>, samples: Vec<Sample>,
    outcomes: BTreeMap<String,Outcome>, current: Option<Current>, skipped: u64,
}
#[derive(Default,serde::Serialize)]
struct Outcome { calls:u64, input_bytes:u64, return_input_bytes:u64, no_echo:u64 }
#[derive(Default,serde::Serialize)]
struct Ledger { native_ns:u64, vm_ns:u64, import_ns:u64, body_union_ns:u64, observer_ns:u64 }
#[derive(serde::Serialize)]
struct Sample {
    registration:String, sequence:u64, input_bytes:usize, remaining_bytes:u64,
    wall_ns:u64, ledger:Ledger, imports:u64, imports_without_body:u64,
    invalid_transitions:u64, incomplete:bool,
}
struct Current { sample:Sample, start:Instant, last:u64, stack:Vec<Phase> }
enum Phase { Native, Vm, Import { depth:u32, seen:bool } }
enum Event { VmIn, VmOut, HostIn, HostOut, BodyIn, BodyOut }

impl Current {
    fn advance(&mut self, now:u64) {
        let elapsed=now.saturating_sub(self.last); self.last=now;
        match self.stack.last() {
            Some(Phase::Native)=>self.sample.ledger.native_ns+=elapsed,
            Some(Phase::Vm)=>self.sample.ledger.vm_ns+=elapsed,
            Some(Phase::Import{depth,..})=>{
                self.sample.ledger.import_ns+=elapsed;
                if *depth>0 { self.sample.ledger.body_union_ns+=elapsed; }
            },
            None=>self.sample.invalid_transitions+=1,
        }
    }
    fn event(&mut self, event:Event) {
        let valid=match event {
            Event::VmIn=>{ self.stack.push(Phase::Vm); true },
            Event::VmOut=>if matches!(self.stack.last(),Some(Phase::Vm)) {self.stack.pop();true}else{false},
            Event::HostIn=>if matches!(self.stack.last(),Some(Phase::Vm)) {
                self.stack.push(Phase::Import{depth:0,seen:false});self.sample.imports+=1;true
            }else{false},
            Event::HostOut=>if matches!(self.stack.last(),Some(Phase::Import{depth:0,..})) {
                if matches!(self.stack.pop(),Some(Phase::Import{seen:false,..})) {self.sample.imports_without_body+=1;} true
            }else{false},
            Event::BodyIn=>if let Some(Phase::Import{depth,seen})=self.stack.last_mut(){*depth+=1;*seen=true;true}else{false},
            Event::BodyOut=>if let Some(Phase::Import{depth,..})=self.stack.last_mut(){if *depth>0{*depth-=1;true}else{false}}else{false},
        };
        if !valid {self.sample.invalid_transitions+=1;}
    }
}
impl Recorder {
    pub(super) fn new(stride:u64)->Self {
        assert!(stride>0);
        Self(Arc::new(Inner{active:AtomicBool::new(false),state:Mutex::new(State{
            stride,seen:BTreeMap::new(),samples:Vec::new(),outcomes:BTreeMap::new(),current:None,skipped:0,
        })}))
    }
    pub(super) fn begin(&self,invocation:&HookInvocation)->Option<Guard> {
        let HookPayload::Expression(payload)=&invocation.payload else{return None};
        let active=payload.active_type.as_ref()?;
        let mut state=self.0.state.lock().unwrap();
        let sequence={let seen=state.seen.entry(active.registration_id.clone()).or_default();*seen+=1;*seen};
        let sampled=(sequence==1 || sequence%state.stride==0) && state.samples.len()<512;
        if sampled {
            assert!(state.current.is_none(),"binding calls must not overlap in one Store");
            state.current=Some(Current{sample:Sample{
                registration:active.registration_id.clone(),sequence,input_bytes:payload.input.len(),
                remaining_bytes:payload.remaining.end.saturating_sub(payload.remaining.start),
                wall_ns:0,ledger:Ledger::default(),imports:0,imports_without_body:0,invalid_transitions:0,incomplete:false,
            },start:Instant::now(),last:0,stack:vec![Phase::Native]});
            self.0.active.store(true,Ordering::Relaxed);
        } else {state.skipped+=1;}
        Some(Guard{recorder:self.clone(),sampled,input_bytes:payload.input.len(),registration:active.registration_id.clone()})
    }
    fn event(&self,event:Event) {
        if !self.0.active.load(Ordering::Relaxed){return;}
        // Lock acquisition and event bookkeeping are charged to observer time.
        let event_at=Instant::now();
        let mut state=self.0.state.lock().unwrap();
        if let Some(current)=&mut state.current {
            let at=event_at.duration_since(current.start).as_nanos() as u64;
            current.advance(at);current.event(event);
            let ended=current.start.elapsed().as_nanos() as u64;
            current.sample.ledger.observer_ns+=ended.saturating_sub(at);current.last=ended;
        }
    }
    pub(super) fn vm_event(&self,event:wasmtime::CallHook) {
        self.event(match event {
            wasmtime::CallHook::CallingWasm=>Event::VmIn,
            wasmtime::CallHook::ReturningFromWasm=>Event::VmOut,
            wasmtime::CallHook::CallingHost=>Event::HostIn,
            wasmtime::CallHook::ReturningFromHost=>Event::HostOut,
        });
    }
    pub(super) fn body_enter(&self){self.event(Event::BodyIn);}
    pub(super) fn body_exit(&self){self.event(Event::BodyOut);}
    pub(super) fn snapshot(&self)->serde_json::Value {
        let state=self.0.state.lock().unwrap();assert!(state.current.is_none());
        serde_json::json!({"stridePerRegistration":state.stride,"sampleLimit":512,"skipped":state.skipped,
            "samples":state.samples,"outcomes":state.outcomes,
            "limits":"Systematic bounded samples, not population extrapolation. VM includes canonical allocation/codec/cleanup. Native outside VM is combined lowering/lifting/native setup, not direction-separated. Import residual includes argument/return marshaling and any unmeasured Host body. Logical string lengths are not physical copies. No source text retained."})
    }
}
pub(in crate::host) struct Guard {recorder:Recorder,sampled:bool,input_bytes:usize,registration:String}
impl Guard {
    pub(in crate::host) fn finish(self,output:Option<&crate::bindings::nlaocs::skript_parser_addon::types::HookOutput>) {
        self.finish_inner(output);
    }
    fn finish_inner(&self,output:Option<&crate::bindings::nlaocs::skript_parser_addon::types::HookOutput>) {
        self.recorder.0.active.store(false,Ordering::Relaxed);
        let ended=Instant::now();let mut state=self.recorder.0.state.lock().unwrap();
        if self.sampled {
            let mut current=state.current.take().unwrap();
            current.sample.wall_ns=ended.duration_since(current.start).as_nanos() as u64;
            current.advance(current.sample.wall_ns);
            current.sample.incomplete=current.stack.len()!=1 || !matches!(current.stack[0],Phase::Native);
            state.samples.push(current.sample);
        }
        let echo=output.and_then(|out|out.replacement.as_ref());
        let tag=outcome_tag(output);
        let row=state.outcomes.entry(format!("{}/{tag}",self.registration)).or_default();
        row.calls+=1;row.input_bytes+=self.input_bytes as u64;
        if let Some(echo)=echo {row.return_input_bytes+=input_bytes(echo) as u64;}else{row.no_echo+=1;}
    }
}
fn outcome_tag(output:Option<&crate::bindings::nlaocs::skript_parser_addon::types::HookOutput>)->String {
    output.map_or_else(||"trap-or-error".into(),|out|{
            let decision=match out.decision {HookDecision::ContinueProcessing=>"continue",HookDecision::NotApplicable=>"not-applicable",
                HookDecision::Reject(_)=>"reject",HookDecision::Handled=>"handled"};
            let outcome=match out.replacement.as_ref() {
                Some(HookPayload::Expression(p))=>match p.type_parser_outcome {
                    Some(crate::bindings::nlaocs::skript_parser_addon::types::TypeParserOutcome::Handled)=>"handled",
                    Some(crate::bindings::nlaocs::skript_parser_addon::types::TypeParserOutcome::NoMatch)=>"no-match",
                    None=>"no-outcome",
                },
                Some(_)=>"other-payload",None=>"no-echo",
            };format!("{decision}/{outcome}")
        })
}

#[test]
fn boundary_labels_do_not_retain_reject_reason_or_diagnostics() {
    use crate::bindings::nlaocs::skript_parser_addon::types::{HookOutput,Rejection};
    let output=HookOutput{decision:HookDecision::Reject(Rejection{reason:"private-source-sentinel".into(),diagnostics:Vec::new()}),
        replacement:None,effects:empty_effects()};
    assert_eq!(outcome_tag(Some(&output)),"reject/no-echo");
    assert_eq!(outcome_tag(None),"trap-or-error");
}

#[test]
fn boundary_ledger_accounts_nested_host_body_union_and_multiple_vm_entries() {
    let mut current=Current{sample:Sample{registration:String::new(),sequence:1,input_bytes:0,remaining_bytes:0,
        wall_ns:0,ledger:Ledger::default(),imports:0,imports_without_body:0,invalid_transitions:0,incomplete:false},
        start:Instant::now(),last:0,stack:vec![Phase::Native]};
    for (at,event) in [(10,Event::VmIn),(30,Event::HostIn),(35,Event::BodyIn),(40,Event::BodyIn),
        (45,Event::BodyOut),(50,Event::BodyOut),(55,Event::HostOut),(70,Event::VmOut),
        (72,Event::VmIn),(74,Event::VmOut)] {current.advance(at);current.event(event);}
    current.advance(80);
    assert_eq!((current.sample.ledger.native_ns,current.sample.ledger.vm_ns,current.sample.ledger.import_ns,
        current.sample.ledger.body_union_ns),(18,37,25,15));
    assert_eq!(current.sample.invalid_transitions,0);assert_eq!(current.stack.len(),1);
    current.event(Event::HostOut);assert_eq!(current.sample.invalid_transitions,1);
}
