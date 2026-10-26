//! Opt-in local allocator request accounting; not an end-to-end heap profile.
use super::*;
use std::{alloc::{GlobalAlloc,Layout,System},cell::Cell,hint::black_box,time::Instant};
mod legacy;

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
struct Counts { alloc:u64,zeroed:u64,realloc:u64,dealloc:u64,requested_bytes:u64 }
thread_local! {
    static ACTIVE:Cell<bool>=const{Cell::new(false)};
    static COUNTS:Cell<Counts>=const{Cell::new(Counts{alloc:0,zeroed:0,realloc:0,dealloc:0,requested_bytes:0})};
}
struct CountingAllocator;
#[global_allocator]
static ALLOCATOR:CountingAllocator=CountingAllocator;
fn count(operation:u8,bytes:usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _=COUNTS.try_with(|cell|{let mut c=cell.get();match operation {
            0=>c.alloc=c.alloc.saturating_add(1),1=>c.zeroed=c.zeroed.saturating_add(1),
            2=>c.realloc=c.realloc.saturating_add(1),_=>c.dealloc=c.dealloc.saturating_add(1),
        }if operation!=3{c.requested_bytes=c.requested_bytes.saturating_add(bytes as u64);}cell.set(c);});
    }
}
// SAFETY: every operation forwards its unchanged pointer/layout contract to System.
// Counters use const TLS, never allocate, and tolerate TLS destruction without panic.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self,layout:Layout)->*mut u8 {let p=unsafe{System.alloc(layout)};count(0,layout.size());p}
    unsafe fn alloc_zeroed(&self,layout:Layout)->*mut u8 {let p=unsafe{System.alloc_zeroed(layout)};count(1,layout.size());p}
    unsafe fn realloc(&self,p:*mut u8,layout:Layout,n:usize)->*mut u8 {let p=unsafe{System.realloc(p,layout,n)};count(2,n);p}
    unsafe fn dealloc(&self,p:*mut u8,layout:Layout){count(3,0);unsafe{System.dealloc(p,layout)}}
}
struct Tracking;
impl Drop for Tracking {fn drop(&mut self){let _=ACTIVE.try_with(|v|v.set(false));}}
fn allocations(f:impl FnOnce())->Counts {
    assert!(!ACTIVE.with(Cell::get));COUNTS.with(|v|v.set(Counts::default()));ACTIVE.with(|v|v.set(true));
    let guard=Tracking;f();drop(guard);COUNTS.with(Cell::get)
}
fn counts_json(c:Counts)->serde_json::Value {serde_json::json!({"alloc":c.alloc,"allocZeroed":c.zeroed,
    "realloc":c.realloc,"dealloc":c.dealloc,"requestedBytes":c.requested_bytes})}
fn observed(states:impl IntoIterator<Item=StaticPatternMatch>)->Vec<(usize,Vec<String>,i32)> {
    states.into_iter().map(|s|(s.cursor,s.tags,s.mark)).collect()
}
fn signature(value:Option<StaticPatternMatch>)->Option<(usize,Vec<String>,i32)> {value.map(|v|(v.cursor,v.tags,v.mark))}

fn element(value:PatternElement)->SpannedPatternElement {
    SpannedPatternElement::new(value,syntax_pattern_parser::syntax::Span::new(0,0))
}
fn compare_sequence(elements:&[SpannedPatternElement],input:&str,n:usize) {
    let states=(0..n).map(|i|StaticPatternMatch{cursor:0,tags:vec![format!("seed-{i}")],mark:i as i32}).collect::<Vec<_>>();
    let old=legacy::legacy_match_static_sequence(elements,input,states.clone());
    let mut inline=StaticPatternStates::default();inline.extend(states.into_iter());
    assert_eq!(inline.len(),n);assert_eq!(inline.is_empty(),n==0);
    let new=match_static_sequence(elements,input,inline);
    assert_eq!(observed(old),observed(new.into_iter()),"input {input:?}, {n} incoming states");
    assert_eq!(signature(legacy::legacy_match_static_pattern(elements,input)),signature(match_static_pattern(elements,input)));
}

#[test]
fn inline_states_preserve_empty_branch_tags_unicode_offsets_and_limit_order() {
    use PatternElement::*;
    assert_eq!(MAX_STATIC_PATTERN_STATES,256);
    let mut patterns=vec![vec![],vec![element(Empty)],vec![element(Literal("a".into()))],
        vec![element(ParseTag(String::new())),element(ParseMark(-1))],
        vec![element(Regex(".*".into()))],
        vec![element(Choice(vec![vec![element(ParseTag("1".into())),element(Literal("a".into()))],
            vec![element(ParseTag("2".into())),element(Literal("ab".into()))]]))],
        vec![element(Option(vec![])),element(Group(vec![]))],
        vec![element(Choice((0..300).map(|i|vec![element(ParseTag(i.to_string()))]).collect()))],
        (0..10).map(|i|element(Option(vec![element(ParseTag((1<<i).to_string()))]))).collect(),
    ];
    for text in ["ß","ẞ","İ","i\u{307}","🧪","東京"," a "] {patterns.push(vec![element(Literal(text.into()))]);}
    let rules=PluralRules::from_json(r#"{"algorithm":"singular-aware","pluralOverrideSupported":false,"rules":[{"ruleOrder":0,"singular":"","plural":"s","completeWord":false,"origin":"built-in","addon":{"name":"Fixture","version":"1"}}]}"#).unwrap();
    patterns.push(parse_pattern("%texts%",&rules).unwrap().elements);
    let mut deep=vec![element(Literal("a".into()))];for _ in 0..128 {deep=vec![element(Group(deep))];}patterns.push(deep);
    for elements in patterns {for input in ["","a","ab","a "," A","ß","ẞ","İ","i\u{307}","🧪","東京"] {
        for n in [0,1,3,255,256,257] {compare_sequence(&elements,input,n);}
    }}
}

#[test]
fn generated_nested_sequences_match_frozen_reference() {
    fn random(seed:&mut u64)->u64 {*seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);*seed}
    fn generated(seed:&mut u64,depth:usize)->Vec<SpannedPatternElement> {
        let len=(random(seed)%4) as usize;
        (0..len).map(|_|element(match random(seed)%if depth==0{5}else{8} {
            0=>PatternElement::Literal(["a","b"," ","ß","🧪"][(random(seed)%5) as usize].into()),
            1=>PatternElement::Empty,2=>PatternElement::ParseTag((random(seed)%16).to_string()),
            3=>PatternElement::ParseMark(random(seed) as i32),4=>PatternElement::Regex(".*".into()),
            5=>PatternElement::Option(generated(seed,depth-1)),6=>PatternElement::Group(generated(seed,depth-1)),
            _=>PatternElement::Choice(vec![generated(seed,depth-1),generated(seed,depth-1)]),
        })).collect()
    }
    let mut seed=17;
    for _ in 0..512 {let elements=generated(&mut seed,3);for input in ["","a","b","ab"," a","aa ","ß","ẞ","🧪"] {
        for n in [1,3] {compare_sequence(&elements,input,n);}
    }}
}

#[test]
fn deterministic_literal_paths_avoid_state_vector_allocations() {
    let elements=[element(PatternElement::Literal("a".into())),element(PatternElement::Empty)];
    let old=allocations(||drop(black_box(legacy::legacy_match_static_pattern(&elements,black_box("a")))));
    let new=allocations(||drop(black_box(match_static_pattern(&elements,black_box("a")))));
    assert!(old.alloc>=2);assert_eq!(new.alloc,0);assert_eq!(new.realloc,0);
    let old=allocations(||drop(black_box(legacy::legacy_match_static_pattern(&elements,black_box("b")))));
    let new=allocations(||drop(black_box(match_static_pattern(&elements,black_box("b")))));
    assert!(old.alloc>=1);assert_eq!(new.alloc,0);
}

#[test]
fn allocation_observer_counts_only_scoped_thread_and_closes_after_unwind() {
    let counts=allocations(||{drop(black_box(vec![7_u64;black_box(17)]));});
    assert_eq!(counts.alloc,1);assert_eq!(counts.requested_bytes,136);assert_eq!(counts.dealloc,1);
    let result=std::panic::catch_unwind(||{allocations(||panic!("close counter"));});assert!(result.is_err());
    assert!(!ACTIVE.with(Cell::get));
}

#[test]
#[ignore="bounded public snapshot matcher cost; needs SKRIPT_MATCHER_COST_PLAN"]
fn measure_registered_pattern_allocations() {
    use sha2::{Digest,Sha256};
    let plan:serde_json::Value=serde_json::from_slice(&std::fs::read(std::env::var_os("SKRIPT_MATCHER_COST_PLAN").unwrap()).unwrap()).unwrap();
    assert_eq!(plan["publicFixtures"],true);let started=Instant::now();
    let snapshot=std::path::Path::new(plan["snapshot"].as_str().unwrap());
    let files=plan["snapshotFiles"].as_array().unwrap();assert_eq!(files.len(),21);
    for f in files {let name=std::path::Path::new(f["name"].as_str().unwrap());assert!(name.is_relative()&&name.components().all(|p|matches!(p,std::path::Component::Normal(_))));
        assert_eq!(format!("{:x}",Sha256::digest(std::fs::read(snapshot.join(name)).unwrap())),f["sha256"]);}
    let rules=PluralRules::from_json(&std::fs::read_to_string(snapshot.join("PluralRules.json")).unwrap()).unwrap();
    let types:serde_json::Value=serde_json::from_slice(&std::fs::read(snapshot.join("Types.json")).unwrap()).unwrap();
    let mut groups=Vec::new();
    for t in types.as_array().unwrap(){if let Some(patterns)=t["registeredParserPatterns"].as_array(){
        let mut entries=patterns.iter().filter_map(|p|parse_pattern(p["pattern"].as_str().unwrap(),&rules).ok()
            .map(|ast|(p["registrationIndex"].as_u64().unwrap(),p["patternIndex"].as_u64().unwrap(),ast))).collect::<Vec<_>>();
        entries.sort_by_key(|e|(e.0,e.1));if !entries.is_empty(){groups.push(entries);}
    }}
    let inputs=plan["inputs"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>();
    assert!(!groups.is_empty()&&inputs.len()<=64&&inputs.iter().all(|s|s.len()<=256));
    let repeats=plan["repeats"].as_u64().unwrap();let blocks=plan["blocks"].as_u64().unwrap();assert!((1..=128).contains(&repeats)&&(1..=4).contains(&blocks));
    let run=|old:bool,n:u64|{for _ in 0..n {for patterns in &groups {for input in &inputs {
        let result=patterns.iter().enumerate().find_map(|(i,entry)|{
            let result=if old{legacy::legacy_match_static_pattern(&entry.2.elements,black_box(input))}else{match_static_pattern(&entry.2.elements,black_box(input))};
            result.map(|value|(i,value))});drop(black_box(result));
    }}}};
    let matrix=|old:bool|groups.iter().map(|patterns|inputs.iter().map(|input|patterns.iter().enumerate().find_map(|(i,entry)|{
        let value=if old{legacy::legacy_match_static_pattern(&entry.2.elements,input)}else{match_static_pattern(&entry.2.elements,input)};
        signature(value).map(|v|(i,v))})).collect::<Vec<_>>()).collect::<Vec<_>>();
    let baseline=matrix(true);assert_eq!(baseline,matrix(false));
    let digest=format!("{:x}",Sha256::digest(format!("{baseline:?}")));
    let mut rows=Vec::new();
    for block in 0..blocks {for old in if block%2==0{[true,false]}else{[false,true]}{
        assert!(started.elapsed().as_secs()<30);run(old,1);
        let counts=allocations(||run(old,1));
        let timer=Instant::now();run(old,repeats);let elapsed_ns=timer.elapsed().as_nanos() as u64;
        rows.push(serde_json::json!({"block":block,"legacy":old,"allocationPasses":1,"counts":counts_json(counts),"timedPasses":repeats,"elapsedNs":elapsed_ns}));
    }}
    assert_eq!(baseline,matrix(false));
    let output=serde_json::json!({"rows":rows,"groups":groups.len(),"patterns":groups.iter().map(Vec::len).sum::<usize>(),"inputCount":inputs.len(),"observationSha256":digest,
        "limits":"Public snapshot patterns and explicit local controls, not actual call-frequency distribution. Counts include matching and returned-state/tag drops; setup/formatting excluded. Requested sizes are allocator requests, not OS heap/copy bytes. Timings have counting disabled but retain inactive TLS-check allocator wrapper; end-to-end production allocator comparison is separate."});
    use std::io::Write;let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(plan["output"].as_str().unwrap()).unwrap();
    file.write_all(&serde_json::to_vec_pretty(&output).unwrap()).unwrap();eprintln!("matcher_cost {output}");
}
