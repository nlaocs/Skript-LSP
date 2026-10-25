//! Bounded test-only actual intervals, with no payloads or dynamic labels.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

const MAX_THREADS: usize = 16;
const OUTPUT_BYTES: usize = 8 * 1024 * 1024;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
type Buffer = Arc<Mutex<ThreadBuffer>>;
struct Cached { session: Weak<Session>, buffer: Option<Weak<Mutex<ThreadBuffer>>> }
thread_local! { static BUFFERS: RefCell<HashMap<u64, Cached>> = RefCell::new(HashMap::new()); }

#[derive(Clone)]
pub(super) struct Trace(Arc<Session>);
struct Session {
    id: u64, origin: Instant, limit: usize, reserve: usize, minimum_ns: u64,
    coarse_attempts: AtomicUsize, detail_attempts: AtomicUsize, thread_drops: AtomicUsize,
    buffers: Mutex<Vec<Buffer>>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind { Scope, Catalog }
impl Kind { fn name(self) -> &'static str { match self { Self::Scope => "scope", Self::Catalog => "catalog" } } }
struct Frame {
    id: u64, parent: Option<u64>, name: &'static str, kind: Kind,
    start_ns: u64, children_ns: u64, bytes: Option<u64>,
}
struct Event { frame: Frame, duration_ns: u64, exclusive_ns: u64 }
#[derive(Default, serde::Serialize)]
struct Total { calls: u64, inclusive_ns: u64, exclusive_ns: u64, bytes: u64 }
struct ThreadBuffer {
    tid: usize, next_id: u64, stack: Vec<Frame>, events: Vec<Event>,
    totals: BTreeMap<(Kind, &'static str), Total>, short: u64, capacity_drops: u64,
}
pub(super) struct Guard {
    trace: Trace, buffer: Buffer, id: u64,
    // A timer cannot migrate to another thread and mislabel its interval.
    _same_thread: PhantomData<Rc<()>>,
}
fn nanos(origin: Instant, time: Instant) -> u64 {
    u64::try_from(time.saturating_duration_since(origin).as_nanos()).unwrap_or(u64::MAX)
}
fn coarse(name: &str, kind: Kind) -> bool {
    kind == Kind::Scope && (name.starts_with("document.") || name.starts_with("syntax."))
}

impl Trace {
    pub(super) fn new(origin: Instant, limit: usize, minimum_ns: u64) -> Self {
        assert!((4..=20_000).contains(&limit));
        Self(Arc::new(Session { id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed), origin,
            limit, reserve: 256.min(limit/4), minimum_ns,
            coarse_attempts: AtomicUsize::new(0), detail_attempts: AtomicUsize::new(0),
            thread_drops: AtomicUsize::new(0), buffers: Mutex::new(Vec::new()) }))
    }
    fn buffer(&self) -> Option<Buffer> {
        BUFFERS.with(|cell| {
            let mut cache = cell.borrow_mut();
            if let Some(cached) = cache.get(&self.0.id) {
                return cached.buffer.as_ref().and_then(Weak::upgrade);
            }
            cache.retain(|_, cached| cached.session.strong_count()>0);
            // Registration only; hot events use one thread-local buffer lock.
            let mut buffers = self.0.buffers.lock().unwrap();
            let buffer = (buffers.len()<MAX_THREADS).then(|| {
                let buffer = Arc::new(Mutex::new(ThreadBuffer { tid: buffers.len()+1,
                    next_id: 0, stack: Vec::new(), events: Vec::new(), totals: BTreeMap::new(),
                    short: 0, capacity_drops: 0 }));
                buffers.push(Arc::clone(&buffer)); buffer
            });
            cache.insert(self.0.id, Cached { session: Arc::downgrade(&self.0),
                buffer: buffer.as_ref().map(Arc::downgrade) });
            buffer
        })
    }
    pub(super) fn begin(&self, name: &'static str, kind: Kind, started: Instant,
        bytes: Option<u64>) -> Option<Guard> {
        let Some(buffer) = self.buffer() else {
            self.0.thread_drops.fetch_add(1,Ordering::Relaxed); return None;
        };
        let mut thread = buffer.lock().unwrap(); thread.next_id+=1; let id=thread.next_id;
        let parent=thread.stack.last().map(|frame|frame.id);
        thread.stack.push(Frame { id,parent,name,kind,start_ns:nanos(self.0.origin,started),children_ns:0,bytes });
        drop(thread); Some(Guard { trace:self.clone(),buffer,id,_same_thread:PhantomData })
    }
    fn output(&self, wall_ns: u64, max_bytes: usize) -> serde_json::Value {
        let mut rows=Vec::new(); let mut totals=Vec::new(); let mut metadata=Vec::new();
        let mut short=0; let mut drops=0;
        for buffer in self.0.buffers.lock().unwrap().iter() {
            let thread=buffer.lock().unwrap(); assert!(thread.stack.is_empty(),"trace still active");
            metadata.push(serde_json::json!({"ph":"M","name":"thread_name","pid":1,"tid":thread.tid,
                "args":{"name":format!("parser-thread-{}",thread.tid)}}));
            short+=thread.short; drops+=thread.capacity_drops;
            for ((kind,name),total) in &thread.totals {
                totals.push(serde_json::json!({"tid":thread.tid,"category":kind.name(),"name":name,"sample":total}));
            }
            for event in &thread.events {
                let f=&event.frame;
                rows.push((coarse(f.name,f.kind),f.start_ns,thread.tid,f.id,
                    serde_json::json!({"ph":"X","name":f.name,"cat":f.kind.name(),"pid":1,"tid":thread.tid,
                        "ts":f.start_ns as f64/1000.0,"dur":event.duration_ns as f64/1000.0,
                        "args":{"event_id":format!("{}:{}",thread.tid,f.id),
                            "parent_id":f.parent.map(|id|format!("{}:{id}",thread.tid)),"parent_retained":false,
                            "start_ns":f.start_ns,"duration_ns":event.duration_ns,
                            "exclusive_ns":event.exclusive_ns,"bytes":f.bytes}})));
            }
        }
        let captured=rows.len();
        let mut output=serde_json::json!({"traceEvents":[],"displayTimeUnit":"ms","allEventTotals":totals,
            "metadata":{"schema":"skript-investigation-trace.v1","clock":"monotonic parse-origin; ts/dur in microseconds",
                "parse_wall_ns":wall_ns,"event_limit":self.0.limit,"coarse_reserve":self.0.reserve,
                "minimum_detail_ns":self.0.minimum_ns,"thread_limit":MAX_THREADS,"output_byte_limit":max_bytes,
                "captured_events":captured,"filtered_short_events":short,"capacity_dropped_events":drops,
                "unregistered_thread_events":self.0.thread_drops.load(Ordering::Relaxed),
                "export_dropped_events":0,"retained_events":0,"missing_direct_parents":0,
                "exclusive_definition":"all directly instrumented children, including filtered/dropped events and Catalog callbacks; different from scopes-only aggregate exclusive",
                "limits":"Other imports and WIT transfer remain inside guest boundary. Sum of nested inclusive or cross-thread totals is not wall time. Thread ordinals are local, not OS thread IDs. No payload/source text. Trace excludes host initialization and post-parse export."}});
        let header_bytes=serde_json::to_vec(&output).unwrap().len();
        assert!(header_bytes+4096<max_bytes,"trace metadata exceeds output budget");
        let mut budget=max_bytes-header_bytes-4096;
        // Choose coarse real intervals first if bytes are capped; then restore
        // actual chronological order, without synthesizing omitted spans.
        rows.sort_by_key(|(priority,start,tid,id,_)|(!*priority,*start,*tid,*id));
        let mut kept=Vec::new();
        for row in rows {
            let bytes=serde_json::to_vec(&row.4).unwrap().len()+1;
            if bytes<=budget { budget-=bytes; kept.push(row); }
        }
        kept.sort_by_key(|(_,start,tid,id,_)|(*start,*tid,*id));
        let ids=kept.iter().map(|row|row.4["args"]["event_id"].as_str().unwrap().to_owned()).collect::<HashSet<_>>();
        let mut missing=0;
        for row in &mut kept {
            let present=row.4["args"]["parent_id"].as_str().map(|id|ids.contains(id));
            if present==Some(false) { missing+=1; }
            row.4["args"]["parent_retained"]=present.into();
        }
        output["metadata"]["retained_events"]=kept.len().into();
        output["metadata"]["export_dropped_events"]=(captured-kept.len()).into();
        output["metadata"]["missing_direct_parents"]=missing.into();
        output["metadata"]["instrumented_totals_complete"]=(self.0.thread_drops.load(Ordering::Relaxed)==0).into();
        output["metadata"]["complete_timeline"]=(short==0 && drops==0 && captured==kept.len()
            && self.0.thread_drops.load(Ordering::Relaxed)==0).into();
        metadata.extend(kept.into_iter().map(|row|row.4)); output["traceEvents"]=metadata.into();
        assert!(serde_json::to_vec(&output).unwrap().len()<=max_bytes); output
    }
    pub(super) fn write(&self, path: &std::path::Path, wall_ns: u64) -> serde_json::Value {
        use std::io::Write;
        let output=self.output(wall_ns,OUTPUT_BYTES); let bytes=serde_json::to_vec(&output).unwrap();
        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(&bytes).unwrap();
        serde_json::json!({"metadata":output["metadata"],"bytes":bytes.len(),"path":path})
    }
}
impl Guard {
    pub(super) fn finish(mut self, ended: Instant, returned_bytes: Option<u64>) { self.complete(ended,returned_bytes); }
    fn complete(&mut self, ended: Instant, returned_bytes: Option<u64>) {
        if self.id==0 { return; }
        let mut thread=self.buffer.lock().unwrap(); let mut frame=thread.stack.pop().unwrap();
        assert_eq!(frame.id,self.id); self.id=0;
        if returned_bytes.is_some() { frame.bytes=returned_bytes; }
        let duration=nanos(self.trace.0.origin,ended).saturating_sub(frame.start_ns);
        let exclusive=duration.saturating_sub(frame.children_ns);
        if let Some(parent)=thread.stack.last_mut() { parent.children_ns+=duration; }
        let total=thread.totals.entry((frame.kind,frame.name)).or_default();
        total.calls+=1; total.inclusive_ns+=duration; total.exclusive_ns+=exclusive; total.bytes+=frame.bytes.unwrap_or(0);
        let priority=coarse(frame.name,frame.kind);
        if !priority && duration<self.trace.0.minimum_ns { thread.short+=1; return; }
        let (attempts,limit)=if priority { (&self.trace.0.coarse_attempts,self.trace.0.reserve) }
            else { (&self.trace.0.detail_attempts,self.trace.0.limit-self.trace.0.reserve) };
        if attempts.fetch_add(1,Ordering::Relaxed)>=limit { thread.capacity_drops+=1; return; }
        thread.events.push(Event { frame,duration_ns:duration,exclusive_ns:exclusive });
    }
}
impl Drop for Guard { fn drop(&mut self) { self.complete(Instant::now(),None); } }

#[test]
fn actual_intervals_keep_parentage_and_complete_exclusive_accounting() {
    let origin=Instant::now(); let trace=Trace::new(origin,20,0);
    let outer=trace.begin("document.total",Kind::Scope,Instant::now(),None).unwrap();
    let inner=trace.begin("copy.test",Kind::Scope,Instant::now(),Some(123)).unwrap();
    drop(inner); drop(outer); let out=trace.output(nanos(origin,Instant::now()),64*1024);
    let rows=out["traceEvents"].as_array().unwrap().iter().filter(|r|r["ph"]=="X").collect::<Vec<_>>();
    let ns=|row:&serde_json::Value,key:&str|row["args"][key].as_u64().unwrap();
    assert_eq!(rows.len(),2); assert_eq!(rows[1]["args"]["parent_id"],rows[0]["args"]["event_id"]);
    assert!(ns(rows[1],"start_ns")>=ns(rows[0],"start_ns"));
    assert_eq!(ns(rows[0],"duration_ns"),ns(rows[0],"exclusive_ns")+ns(rows[1],"duration_ns"));
    assert_eq!(rows[1]["args"]["bytes"],123); assert_eq!(out["metadata"]["missing_direct_parents"],0);
}
#[test]
fn filtering_and_capacity_keep_coarse_intervals_and_full_totals() {
    for threshold in [0,u64::MAX] {
        let origin=Instant::now(); let trace=Trace::new(origin,4,threshold);
        let outer=trace.begin("document.total",Kind::Scope,Instant::now(),None).unwrap();
        for _ in 0..10 { drop(trace.begin("copy.test",Kind::Scope,Instant::now(),None)); }
        drop(outer); let out=trace.output(nanos(origin,Instant::now()),64*1024);
        assert_eq!(out["allEventTotals"].as_array().unwrap().iter().map(|r|r["sample"]["calls"].as_u64().unwrap()).sum::<u64>(),11);
        assert_eq!(out["metadata"][if threshold==0 {"capacity_dropped_events"} else {"filtered_short_events"}],if threshold==0 {7} else {10});
        assert!(out["traceEvents"].as_array().unwrap().iter().any(|r|r["name"]=="document.total"));
    }
}
#[test]
fn thread_buffers_keep_independent_stacks_without_payload_output() {
    let origin=Instant::now(); let trace=Trace::new(origin,40,0);
    let barrier=Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| { for _ in 0..2 { let trace=trace.clone(); let barrier=Arc::clone(&barrier); scope.spawn(move|| {
        let outer=trace.begin("document.total",Kind::Scope,Instant::now(),None);
        barrier.wait();
        drop(trace.begin("catalog.read",Kind::Catalog,Instant::now(),None)); drop(outer);
    }); }});
    let out=trace.output(nanos(origin,Instant::now()),64*1024);
    let rows=out["traceEvents"].as_array().unwrap().iter().filter(|r|r["ph"]=="X").collect::<Vec<_>>();
    assert_eq!(rows.len(),4); assert_eq!(out["metadata"]["missing_direct_parents"],0);
    assert_eq!(rows.iter().map(|r|r["tid"].as_u64().unwrap()).collect::<HashSet<_>>().len(),2);
    assert!(!serde_json::to_string(&out).unwrap().contains("input"));
    let roots=rows.iter().filter(|r|r["args"]["parent_id"].is_null()).collect::<Vec<_>>();
    let starts=roots.iter().map(|r|r["args"]["start_ns"].as_u64().unwrap()).collect::<Vec<_>>();
    let ends=roots.iter().map(|r|r["args"]["start_ns"].as_u64().unwrap()+r["args"]["duration_ns"].as_u64().unwrap()).collect::<Vec<_>>();
    assert!(starts.iter().max().unwrap()<ends.iter().min().unwrap(),"actual thread intervals overlap");
}

#[test]
fn byte_and_thread_limits_are_reported_without_losing_aggregate_counts() {
    let origin=Instant::now(); let trace=Trace::new(origin,1000,0);
    let outer=trace.begin("document.total",Kind::Scope,Instant::now(),None);
    for _ in 0..700 { drop(trace.begin("copy.test",Kind::Scope,Instant::now(),None)); }
    drop(outer); let out=trace.output(nanos(origin,Instant::now()),16*1024);
    assert!(serde_json::to_vec(&out).unwrap().len()<=16*1024);
    assert!(out["metadata"]["export_dropped_events"].as_u64().unwrap()>0);
    assert_eq!(out["metadata"]["instrumented_totals_complete"],true);
    let trace=Trace::new(Instant::now(),1000,0);
    std::thread::scope(|scope| { for _ in 0..MAX_THREADS+2 { let trace=trace.clone(); scope.spawn(move|| {
        drop(trace.begin("document.total",Kind::Scope,Instant::now(),None));
    }); }});
    let out=trace.output(0,64*1024);
    assert_eq!(out["metadata"]["unregistered_thread_events"],2);
    assert_eq!(out["metadata"]["instrumented_totals_complete"],false);
}
