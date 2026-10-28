//! Read-only, owner-private source jobs. These reports never certify semantics.
use super::*;
use serde::Serialize;
use std::time::Instant;
use crate::bindings::nlaocs::skript_parser_addon::types::{SourcePreparationContext, SourcePreparationStatus};

const MAX_STEPS: u32 = 64;
const MAX_UNIQUE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RETURNED_BYTES: usize = 2 * 1024 * 1024;
const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;
const MAX_JOB_MS: u64 = 5000;
const MAX_QUERIES_PER_STEP: u32 = 128;
const MAX_REPORTS:usize=16;
const PREPARATION_MEMORY_BYTES:usize=16*1024*1024;
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Cumulative preparation time for one host's retained immutable source.
/// Compilation and time between loads are excluded; retries cannot refill it.
#[derive(Default)]
pub(super) struct Clock { used:std::time::Duration }
impl Clock {
    fn remaining(&self)->std::time::Duration {std::time::Duration::from_millis(MAX_JOB_MS).saturating_sub(self.used)}
    fn charge(&mut self,time:std::time::Duration){self.used=self.used.saturating_add(time);}
}

pub(super) fn record_report(reports:&mut Vec<SourcePreparationReport>,dropped:&mut u64,report:SourcePreparationReport) {
    if reports.len()>=MAX_REPORTS {reports.remove(0);*dropped=dropped.saturating_add(1);}
    reports.push(report);
}

/// One measured bounded guest call. Fuel excludes native callbacks and RSS.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct SourcePreparationStep {
    pub step: u32,
    pub label: String,
    pub call_ms: f64,
    pub callback_ms: f64,
    pub fuel_used: u64,
    pub payload_bytes: usize,
    /// Logical raw record bytes examined by native projections, not copies.
    pub native_row_bytes: usize,
    pub unique_input_bytes: usize,
    pub index_bytes: usize,
    /// Maximum approved linear-memory growth request, not RSS or heap live bytes.
    pub max_requested_linear_bytes: usize,
}
/// Complete job observations, associated with its real component and generation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct SourcePreparationReport {
    pub component: String,
    pub component_sha256: String,
    pub generation: u64,
    pub source_digest: Option<String>,
    pub snapshot_id: Option<String>,
    pub status: String,
    pub steps: Vec<SourcePreparationStep>,
    pub elapsed_ms: f64,
    pub fuel_used: u64,
}

#[derive(Default)]
pub(super) struct Budget {
    unique: BTreeMap<String, usize>,
    unique_bytes: usize,
    returned: usize,
    step_queries: u32,
    step_returned: usize,
    pub(super) callback_ns: u128,
    step_raw_record_bytes: usize,
    exceeded: Option<&'static str>,
}
impl Budget {
    fn query(&mut self) -> Result<(), &'static str> {
        self.step_queries=self.step_queries.saturating_add(1);
        if self.step_queries>MAX_QUERIES_PER_STEP { self.exceeded=Some("query-limit"); return Err("source preparation query limit"); }
        Ok(())
    }
    fn input(&mut self, name:&str, bytes:usize) -> Result<(), &'static str> {
        if !self.unique.contains_key(name) {
            if self.unique_bytes.saturating_add(bytes)>MAX_UNIQUE_BYTES { self.exceeded=Some("input-limit"); return Err("source preparation unique input exceeds4MiB"); }
            self.unique.insert(name.into(),bytes);self.unique_bytes+=bytes;
        }
        Ok(())
    }
    fn output(&mut self, bytes:usize) -> Result<(), &'static str> {
        if self.returned.saturating_add(bytes)>MAX_RETURNED_BYTES { self.exceeded=Some("response-limit");return Err("source preparation payload exceeds2MiB"); }
        self.returned+=bytes;self.step_returned+=bytes;Ok(())
    }
}

type IndexKey=(String,String,String,Vec<String>);
#[derive(Default)]
pub(super) struct Pages { entries:BTreeMap<IndexKey,syntaxes::JsonArrayIndex>, bytes:usize }
impl Pages {
    fn read(&mut self, source:&syntaxes::CatalogSource, document:&str, path:&[String], fields:&[String], offset:u64, limit:u32, max:u32)
        -> Result<Option<syntaxes::JsonArrayPage>, syntaxes::JsonPageError>
    {
        let key=(source.source_digest.clone(),source.snapshot_id.clone(),document.into(),path.into());
        if !self.entries.contains_key(&key) {
            let Some(index)=source.json_array_index(document,path)? else {return Ok(None);};
            let bytes=index.index_bytes().saturating_add(document.len()).saturating_add(path.iter().map(String::len).sum::<usize>());
            if bytes>MAX_INDEX_BYTES {return Err(syntaxes::JsonPageError::Limit);}
            if self.entries.len()>=8||self.bytes.saturating_add(bytes)>MAX_INDEX_BYTES {self.entries.clear();self.bytes=0;}
            self.bytes+=bytes;self.entries.insert(key.clone(),index);
        }
        self.entries[&key].page(fields,offset,limit,max).map(Some)
    }
}

impl StoreData {
    pub(super) fn source_query(&mut self, document:Option<(&str,usize)>) -> Result<(),wit_catalog_data::CatalogError> {
        let Some(budget)=self.source_budget.as_mut() else {return Ok(());};
        budget.query().map_err(invalid_catalog_input)?;
        if let Some((name,bytes))=document {budget.input(name,bytes).map_err(invalid_catalog_input)?;}
        Ok(())
    }
    pub(super) fn source_output(&mut self, bytes:usize) -> Result<(),wit_catalog_data::CatalogError> {
        if let Some(budget)=self.source_budget.as_mut() {budget.output(bytes).map_err(invalid_catalog_input)?;}
        Ok(())
    }
    pub(super) fn raw_catalog(&self) -> Result<Arc<Catalog>,wit_catalog_data::CatalogError> {
        self.catalog.clone().ok_or_else(||wit_catalog_data::CatalogError {kind:wit_catalog_data::CatalogErrorKind::Unavailable,message:"catalog data requires an SSG Catalog".into()})
    }
    pub(super) fn json_records(&mut self,digest:String,snapshot:String,document:String,path:Vec<String>,fields:Vec<String>,offset:u64,limit:u32,max:u32)
        -> Result<Option<wit_catalog_data::CatalogJsonPage>,wit_catalog_data::CatalogError>
    {
        let start=Instant::now();
        let result=(||{
            let catalog=self.raw_catalog()?;
            let Some(source)=catalog.source() else {return Ok(None);};
            if source.source_digest!=digest||source.snapshot_id!=snapshot {return Err(invalid_catalog_input("JSON page belongs to another source or snapshot"));}
            let Some(bytes)=source.document(&document) else {self.source_query(None)?;return Ok(None);};
            self.source_query(Some((&document,bytes.len())))?;
            let page=self.source_pages.read(source,&document,&path,&fields,offset,limit,max).map_err(|e|wit_catalog_data::CatalogError {
                kind:if e==syntaxes::JsonPageError::Limit {wit_catalog_data::CatalogErrorKind::ResponseTooLarge}else{wit_catalog_data::CatalogErrorKind::InvalidInput},message:e.to_string()})?;
            let Some(page)=page else {return Ok(None);};
            if let Some(b)=self.source_budget.as_mut(){b.step_raw_record_bytes=b.step_raw_record_bytes.saturating_add(page.source_bytes_examined);}
            self.source_output(page.items.iter().map(|r|r.json.len()+8).sum())?;
            Ok(Some(wit_catalog_data::CatalogJsonPage {total_records:page.total_records,items:page.items.into_iter().map(|r|wit_catalog_data::CatalogFieldRecord{index:r.index,json:r.json}).collect(),next_offset:page.next_offset}))
        })();
        if let Some(b)=self.source_budget.as_mut(){b.callback_ns+=start.elapsed().as_nanos();}
        result
    }
}

/// Called only outside invocation/dynamic transactions. Pending never reaches parse.
pub(super) fn run(config:&HostConfig, id:&str, hash:&str, manifest:&ComponentManifest,
    store:&mut Store<StoreData>,bindings:&ParserAddon,cancel:Option<DocumentCancellationToken>,clock:&mut Clock)
    -> Result<SourcePreparationReport,HostError>
{
    let generation=GENERATION.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|g|g.checked_add(1))
        .map_err(|_|HostError::Runtime {component_id:id.into(),operation:"source preparation",message:"generation exhausted".into()})?;
    let source=store.data().catalog.as_deref().and_then(Catalog::source);
    let (digest,snapshot)=source.map_or((None,None),|s|(Some(s.source_digest.clone()),Some(s.snapshot_id.clone())));
    let mut report=SourcePreparationReport {component:id.into(),component_sha256:hash.into(),generation,source_digest:digest.clone(),snapshot_id:snapshot.clone(),status:"not-requested".into(),steps:Vec::new(),elapsed_ms:0.0,fuel_used:0};
    if !component_promises(&manifest.capabilities,crate::CAPABILITY_SOURCE_PREPARATION) {return Ok(report);}
    if store.data().invocation.is_some()||store.data().dynamic_syntax_update.is_some() {return Err(HostError::Runtime {component_id:id.into(),operation:"source preparation",message:"preparation cannot enter an active transaction".into()});}
    let started=Instant::now();let deadline=started+clock.remaining();
    if store.data().limits.requested_memory_high_water>PREPARATION_MEMORY_BYTES {return Err(HostError::InitializationRejected {component_id:id.into(),message:"source preparation starts above its16MiB linear-memory ceiling".into()});}
    let original_interrupt_safe=store.data().interrupt_safe;
    store.data_mut().limits.preparation_memory_bytes=Some(PREPARATION_MEMORY_BYTES);
    store.data_mut().source_preparing=true;
    // The new preparation contract permits discarding private staging memory.
    // Its only host imports are immutable, so cancellation can discard the Store.
    store.data_mut().interrupt_safe=true;
    store.data().source_violation.set(false);
    store.data_mut().source_budget=Some(Budget::default());
    store.data_mut().cancellation=cancel;
    let outcome:Result<(),HostError>=(||{
        for step in 0..MAX_STEPS {
            if store.data().cancellation.as_ref().is_some_and(DocumentCancellationToken::native_cancelled) {return Err(skript_parser::ParseCancelled.into());}
            let remaining=deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {return Err(HostError::InitializationRejected {component_id:id.into(),message:"source preparation shared wall budget exhausted".into()});}
            let b=store.data_mut().source_budget.as_mut().unwrap();b.step_queries=0;b.step_returned=0;b.callback_ns=0;b.step_raw_record_bytes=0;
            let context=SourcePreparationContext {generation,step,snapshot_id:snapshot.clone(),source_digest:digest.clone()};
            let ticks=remaining.as_nanos().div_ceil(config.epoch_tick.as_nanos()).min(u64::MAX as u128) as u64;
            prepare_store(store,config.fuel_per_call,config.deadline_ticks(id).min(ticks.max(1)),id,"source preparation")?;
            let start=Instant::now();
            let response=bindings.nlaocs_skript_parser_addon_addon().call_prepare_source_step(&mut *store,&context)
                .map_err(|e|classify_wasmtime_error(id.into(),"source preparation",e))?;
            let used=config.fuel_per_call.saturating_sub(store.get_fuel().unwrap_or(0));
            report.fuel_used=report.fuel_used.saturating_add(used);
            let b=store.data().source_budget.as_ref().unwrap();
            let response=response.map_err(|e|HostError::InitializationRejected {component_id:id.into(),message:format!("source preparation: {}",e.message)})?;
            let label=match &response {SourcePreparationStatus::Pending(s)|SourcePreparationStatus::Ready(s)|SourcePreparationStatus::Unavailable(s)=>s};
            if label.len()>256||label.chars().any(char::is_control) {return Err(HostError::InitializationRejected {component_id:id.into(),message:"invalid preparation diagnostic label".into()});}
            report.steps.push(SourcePreparationStep {step,label:label.clone(),call_ms:start.elapsed().as_secs_f64()*1000.0,callback_ms:b.callback_ns as f64/1e6,fuel_used:used,payload_bytes:b.step_returned,native_row_bytes:b.step_raw_record_bytes,unique_input_bytes:b.unique_bytes,index_bytes:store.data().source_pages.bytes,max_requested_linear_bytes:store.data().limits.requested_memory_high_water});
            if Instant::now()>=deadline {return Err(HostError::InitializationRejected {component_id:id.into(),message:"source preparation exceeded shared wall budget before acceptance".into()});}
            if store.data().source_violation.get(){return Err(HostError::InitializationRejected {component_id:id.into(),message:"source preparation attempted a forbidden import".into()});}
            if let Some(reason)=b.exceeded {return Err(HostError::InitializationRejected {component_id:id.into(),message:format!("source preparation exceeded {reason}")});}
            if store.data().cancellation.as_ref().is_some_and(DocumentCancellationToken::native_cancelled) {return Err(skript_parser::ParseCancelled.into());}
            let current=store.data().catalog.as_deref().and_then(Catalog::source);
            if current.map(|s|(&s.source_digest,&s.snapshot_id))!=digest.as_ref().zip(snapshot.as_ref()) {return Err(HostError::InitializationRejected {component_id:id.into(),message:"source changed during preparation".into()});}
            match response {SourcePreparationStatus::Pending(_)=>{},SourcePreparationStatus::Ready(label)=>{report.status=format!("ready:{label}");return Ok(());},SourcePreparationStatus::Unavailable(label)=>{report.status=format!("unavailable:{label}");return Ok(());}}
        }
        Err(HostError::InitializationRejected {component_id:id.into(),message:"source preparation step budget exhausted".into()})
    })();
    report.elapsed_ms=started.elapsed().as_secs_f64()*1000.0;
    clock.charge(started.elapsed());
    store.data_mut().source_preparing=false;store.data_mut().source_budget=None;store.data_mut().cancellation=None;
    store.data_mut().interrupt_safe=original_interrupt_safe;
    store.data_mut().limits.preparation_memory_bytes=None;
    outcome?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_input_and_response_budgets_are_separate_and_finite() {
        let mut b=Budget::default();b.input("Manifest.json",443856).unwrap();b.input("Types.json",1487344).unwrap();b.input("EventValues.json",158941).unwrap();b.input("Operations.json",9297).unwrap();assert_eq!(b.unique_bytes,2099438);
        b.input("Manifest.json",443856).unwrap();assert_eq!(b.unique_bytes,2099438);
        assert!(b.input("extra",MAX_UNIQUE_BYTES).is_err());
        let mut b=Budget::default();b.output(MAX_RETURNED_BYTES).unwrap();assert!(b.output(1).is_err());
        let mut b=Budget::default();for _ in 0..MAX_QUERIES_PER_STEP {b.query().unwrap();}assert!(b.query().is_err());
    }
    #[test]
    fn shared_clock_charges_jobs_and_never_refills_after_overrun() {
        let mut clock=Clock::default();clock.charge(std::time::Duration::from_millis(3000));assert_eq!(clock.remaining(),std::time::Duration::from_millis(2000));
        clock.charge(std::time::Duration::from_millis(2500));assert!(clock.remaining().is_zero());
    }

    fn fixture_host(case:&str)->ParserHost {
        ParserHost::new(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/../artifacts/core-library.wasm")),HostConfig {
            runtime_profile:RuntimeProfile {language:Some(format!("source-preparation:{case}")),skript_version:Some("2.15.4".into()),..Default::default()},
            ..Default::default()
        }).unwrap()
    }
    const FAULT_GUEST:&[u8]=include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/../artifacts/catalog-data-addon.wasm"));

    #[test]
    fn fault_guest_is_not_published_after_pending_forbidden_query_fuel_or_memory_failure() {
        for case in ["pending","forbidden","queries","fuel","memory"] {
            let mut host=fixture_host(case);
            let before=host.components();
            let result=host.load_addon(FAULT_GUEST);
            assert!(result.is_err(),"{case} unexpectedly published");
            let error=result.unwrap_err();let message=error.to_string();
            assert!(match case {
                "pending"=>message.contains("step budget"),
                "forbidden"=>message.contains("forbidden import"),
                "queries"=>message.contains("query-limit"),
                "fuel"=>message.contains("fuel"),
                "memory"=>matches!(error,HostError::ResourceLimit {operation:"source preparation",..}),
                _=>false,
            },"{case}: {message}");
            assert_eq!(host.components().len(),before.len());
            assert_eq!(host.source_preparation_reports().len(),1);
            assert!(host.source_preparation_clock.used>std::time::Duration::ZERO);
        }
    }

    #[test]
    fn cancelled_pending_job_is_discarded_and_retry_has_a_new_generation() {
        let mut host=fixture_host("pending");
        host.native_cancellation=Some(DocumentCancellationToken::after_native_probes(2));
        let error=host.load_addon(FAULT_GUEST).unwrap_err();
        assert!(matches!(error,HostError::Cancelled(_)),"{error:?}");
        assert_eq!(host.components().len(),1);
        let next_after_cancel=GENERATION.load(Ordering::Relaxed);
        host.native_cancellation=None;
        host.config.runtime_profile.language=None;
        host.load_addon(FAULT_GUEST).unwrap();
        let reports=host.source_preparation_reports();
        assert_eq!(reports.len(),2);
        assert_eq!(reports[1].steps.len(),2);
        assert!(reports[1].generation>reports[0].generation);
        assert!(reports[1].generation>=next_after_cancel,"retry must follow every generation allocated before cancellation returned");
        assert!(reports[1].status.starts_with("ready:fixture-generation-"));
    }

    #[test]
    fn unavailable_is_explicit_and_report_storage_is_bounded() {
        let mut host=fixture_host("unavailable");host.load_addon(FAULT_GUEST).unwrap();
        assert_eq!(host.source_preparation_reports()[1].status,"unavailable:fixture-missing-proof");
        let report=host.source_preparation_reports()[1].clone();
        let mut reports=Vec::new();let mut dropped=0;
        for _ in 0..(MAX_REPORTS+3) {record_report(&mut reports,&mut dropped,report.clone());}
        assert_eq!(reports.len(),MAX_REPORTS);assert_eq!(dropped,3);
    }

    #[test]
    fn json_page_source_identity_is_checked_before_cached_rows_are_used() {
        fn source(value:&str)->syntaxes::CatalogSource {
            syntaxes::CatalogSource::from_json_documents("ssg",3,"same-snapshot",BTreeMap::from([
                ("Manifest.json".into(),format!(r#"{{"rows":[{{"value":"{value}"}}]}}"#).into_bytes())
            ])).unwrap()
        }
        let base=||ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")).unwrap().into_catalog();
        let first=source("old");let old=first.source_digest.clone();
        let config=HostConfig{syntax_catalog:Some(Arc::new(base().with_unchecked_source(first))),..Default::default()};
        let runtime=shared_host_runtime(config.epoch_tick).unwrap();
        let mut store=create_store(&runtime.engine,&config,build_type_user_input_matchers(None));
        let read=|s:&mut StoreData,d:String|s.json_records(d,"same-snapshot".into(),"Manifest.json".into(),vec!["rows".into()],vec!["value".into()],0,1,1024);
        assert_eq!(read(store.data_mut(),old.clone()).unwrap().unwrap().items[0].json,br#"{"value":"old"}"#.to_vec());
        let second=source("new");let new=second.source_digest.clone();
        store.data_mut().catalog=Some(Arc::new(base().with_unchecked_source(second)));
        assert!(read(store.data_mut(),old).is_err());
        assert_eq!(read(store.data_mut(),new).unwrap().unwrap().items[0].json,br#"{"value":"new"}"#.to_vec());
    }

    #[test]
    #[ignore="explicit offline real-capture measurement; requires a dedicated capture root"]
    fn measure_real_capture_preparation() {
        let root=std::env::var_os("SKRIPT_PREPARATION_CAPTURE_ROOT").expect("explicit dedicated capture root");
        for name in ["core","reflect"] {
            let snapshot=std::path::PathBuf::from(&root).join(name).join("snapshot");
            let catalog=Arc::new(ssg::load(snapshot).unwrap().into_catalog());
            let host=ParserHost::new(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/../artifacts/core-library.wasm")),HostConfig{syntax_catalog:Some(catalog),..Default::default()}).unwrap();
            let report=&host.source_preparation_reports()[0];
            println!("source-preparation-observation {}",serde_json::to_string(&serde_json::json!({"capture":name,"report":report})).unwrap());
            assert!(report.status.starts_with("ready:registry-projection-ready"));assert!(report.steps.len()<=64);assert!(report.steps.iter().all(|s|s.fuel_used<=20_000_000));
            assert!(report.steps.iter().filter(|s|s.label.starts_with("page:")).count()>=34);
            assert!(report.steps.last().unwrap().unique_input_bytes<=MAX_UNIQUE_BYTES);
        }
    }
}
