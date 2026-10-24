//! Audited producer models. The generic domain checker is independent of them.
//! No sampled runtime object closes a public interface's value domain.
use super::{ResultType, domain::{self, Class, Graph}};
use crate::nlaocs::skript_parser_addon::types::{MetadataEntry, RegisteredExpressionPayload};
use crate::catalog::EventValueOption;
use serde_json::Value;
use std::{collections::HashMap, sync::{Arc, RwLock}};

#[cfg(target_arch="wasm32")]
use std::sync::LazyLock;

const ENTITY: &str = "org.bukkit.entity.Entity";
const IMPLEMENTATION: &str = "org.bukkit.craftbukkit.entity.CraftEntity";
const SLOT: &str = "ch.njol.skript.util.slot.Slot";
const SKRIPT_SHA: &str = "277393a965e164e854f5280aa1c30ce92e69b3fab4df5063711743f9d7847327";
const GENERATOR_SHA: &str = "e020f22f423efef832041dc10656be8d2233b4ddeb842c07d94c1e7e7df48325";
const SERVER_SHA: &str = "3a73f0cc4020a5eb45629a67629b14263331c68de45552f8505df8060fa8ed43";
const MODEL: &str = "skript-2.15.4-paper-c5eb079-native-reference-v1";

struct Model {
    records: Vec<Value>,
    graph: Graph,
    snapshot: String,
    digest: String,
    proofs: RwLock<HashMap<(String,String), bool>>,
}

fn profile(manifest: &Value) -> bool {
    let server = &manifest["server"];
    if manifest["schemaVersion"] != 7 || server["name"] != "Paper"
        || server["version"] != "1.21.11-132-c5eb079 (MC: 1.21.11)"
        || server["implementationClass"] != "org.bukkit.craftbukkit.CraftServer"
        || server["implementationJarSha256"] != SERVER_SHA { return false; }
    let Some(plugins) = manifest["plugins"].as_array() else { return false; };
    plugins.len() == 2 && plugins.iter().enumerate().all(|(i,p)| {
        p["enabled"] == true && p["loadOrder"].as_u64() == Some(i as u64) && match i {
            0 => p["name"] == "SkriptSyntaxGenerator" && p["main"] == "jp.nlaocs.skriptSyntaxGenerator.SkriptSyntaxGenerator"
                && p["jarSha256"] == GENERATOR_SHA,
            1 => p["name"] == "Skript" && p["version"] == "2.15.4"
                && p["main"] == "ch.njol.skript.Skript" && p["jarSha256"] == SKRIPT_SHA,
            _ => false,
        }
    })
}

/// Declaration lineages avoid transferring unrelated ClassHierarchy nodes.
fn graph_from_records(records: &[Value]) -> Option<Graph> {
    let mut classes=Vec::new();
    for record in records {
        let runtime=&record["typeRuntime"];
        if runtime["version"]!=3 || runtime["ancestryComplete"]!=true || matches!(runtime["classKind"].as_str(),Some("Array" | "Primitive")) { return None; }
        let ancestors=runtime["ancestors"].as_array()?;
        if ancestors.len()>1024 { return None; }
        let ancestors=ancestors.iter().map(|a|a.as_str().filter(|s|!s.is_empty()&&s.len()<=1024).map(str::to_owned)).collect::<Option<Vec<_>>>()?;
        if ancestors.iter().collect::<std::collections::BTreeSet<_>>().len()!=ancestors.len() || ancestors.iter().any(|a|record["originalClass"].as_str()==Some(a.as_str())) { return None; }
        classes.push(Class {name:record["originalClass"].as_str()?.into(),kind:runtime["classKind"].as_str()?.into(),super_class:None,interfaces:ancestors,lineage_complete:true});
    }
    let entity=classes.iter().find(|c|c.name==ENTITY)?;
    let mut lineage=entity.interfaces.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    lineage.insert(ENTITY.into()); lineage.insert("java.lang.Object".into());
    // Exact source/JAR-fenced implementation and storage-interceptor classes.
    classes.push(Class {name:IMPLEMENTATION.into(),kind:"Class".into(),super_class:None,interfaces:lineage.into_iter().collect(),lineage_complete:true});
    Graph::new(classes)
}

fn profile_for_runtime(manifest: &Value, runtime: Option<&str>) -> bool {
    runtime==Some("2.15.4") && profile(manifest)
}

impl Model {
    fn stored(&self, public: &str, implementation: &str) -> bool {
        let proof_key=(public.to_owned(),implementation.to_owned());
        if public.len()>1024 { return false; }
        if let Some(result)=self.proofs.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&proof_key).copied() { return result; }
        let result=domain::preserves(public, implementation, &self.records, &self.graph, SLOT);
        let mut proofs=self.proofs.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if proofs.len()>=64 { proofs.clear(); }
        proofs.insert(proof_key,result);
        result
    }

    fn result(&self, public: &str, producer: &str) -> Vec<MetadataEntry> {
        vec![ResultType { version:1, scope:"local-scalar".into(), types:vec![public.into()],
            complete:true, multiple:false, maybe_unset:true, preserves_local_bindings:true }.metadata(),
            MetadataEntry { key:"skript.semantic.storage-provenance".into(), owner_component_id:None,
                value:serde_json::json!({"version":1,"model":MODEL,"sourceDigest":self.digest,
                    "snapshotId":self.snapshot,"producer":producer,"implementationUpperBound":IMPLEMENTATION,
                    "publicBound":public,"domainComplete":true}).to_string() }]
    }
}

/// Prepare immutable proof inputs before entering individual body hooks. Cache
/// keys still include both retained raw source and snapshot identity, and each
/// lookup rechecks the active runtime version. No parse-state effect is emitted.
#[cfg(target_arch="wasm32")]
pub(super) fn prepare() -> bool {
    load_model().is_some()
}

/// A declared concrete Java class is itself an honest implementation upper
/// bound; open interfaces still need producer/backend evidence. No final-class
/// whitelist or unconditional public-bound copy is used.
pub(crate) fn declared_bounds(classes: &[String]) -> bool {
    if classes.is_empty() || classes.len()>16 { return false; }
    let Some(model)=model() else { return false; };
    classes.iter().all(|class|model.stored(class,class))
}

pub(crate) fn native_function(summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary) -> Option<ResultType> {
    use crate::nlaocs::skript_parser_addon::types::{DynamicMultiplicity,ExpressionPossibleReturnTypesState};
    if summary.kind!="function" || summary.possible_return_types_state!=ExpressionPossibleReturnTypesState::Complete
        || summary.multiplicity!=Some(DynamicMultiplicity::Single) { return None; }
    let markers=summary.metadata.iter().filter(|e|e.key=="parser.native-function-value-input").collect::<Vec<_>>();
    if markers.len()!=1 || markers[0].owner_component_id.is_some() || markers[0].value!="1"
        || !declared_bounds(&summary.possible_return_types) { return None; }
    Some(ResultType {version:1,scope:"local-scalar".into(),types:summary.possible_return_types.clone(),complete:true,multiple:false,maybe_unset:true,preserves_local_bindings:true})
}

pub(crate) fn command_argument(payload: &RegisteredExpressionPayload, public: &str, single: bool, parser_input: bool) -> Vec<MetadataEntry> {
    if !single || !parser_input || !crate::catalog::standard_expression_source(payload, "ch.njol.skript.expressions.ExprArgument") {
        return Vec::new();
    }
    let Some(model) = model() else { return Vec::new(); };
    let matches = model.records.iter().filter(|r| r["originalClass"].as_str() == Some(public)).collect::<Vec<_>>();
    if matches.len() != 1 { return Vec::new(); }
    let record = matches[0];
    // Audited parsers' non-null paths are exclusively Bukkit server queries:
    // PlayerParser: online players / UUID query; EntityParser: UUID query.
    // A default expression, custom parser or merely Entity-typed value is not enough.
    if !matches!(record["parserClass"].as_str(), Some("org.skriptlang.skript.bukkit.types.PlayerClassInfo$PlayerParser" | "org.skriptlang.skript.bukkit.types.EntityClassInfo$EntityParser"))
        || record["hasParser"] != true || model.graph.assignable(public, ENTITY) != Some(true)
        || !model.stored(public,IMPLEMENTATION) { return Vec::new(); }
    model.result(public, &format!("command-parser:{}",record["registrationId"].as_str().unwrap_or("unknown")))
}

fn audited_event_route(value: &EventValueOption) -> Option<&'static str> {
    if value.time!=0 || value.context_dependent!=Some(false)
        || value.has_custom_input_validator!=Some(false) || value.has_custom_event_validator!=Some(false) { return None; }
    let registration=match (value.event_class.as_str(),value.value_class.as_str()) {
        ("org.bukkit.event.player.PlayerEvent","org.bukkit.entity.Player") => "event-value:skript:4c5774c564483d1b20dc61834fa94fb8d42a6c9d00fd50729f35c31d441c513d",
        ("ch.njol.skript.command.CommandEvent","org.bukkit.command.CommandSender") => "event-value:skript:73e264537c4e18b1d02189f9e09c0f0128fd3afcb1793c91c2b202148c379a68",
        _ => return None,
    };
    (value.registration_id==registration).then_some(registration)
}

/// No state write for contexts without an audited producer. In particular,
/// unrelated reusable declarations keep their existing header certificates.
#[cfg(target_arch="wasm32")]
pub(super) fn context_has_producer(events: &[String]) -> bool {
    !events.is_empty() && events.len()<=64 && events.iter().all(|event| {
        crate::catalog::event_values_for(event).ok().is_some_and(|values|values.iter().any(|v|audited_event_route(v).is_some()))
    })
}

pub(crate) fn event_reference(payload: &RegisteredExpressionPayload, values: &[EventValueOption], public: &str, single: bool) -> Vec<MetadataEntry> {
    if !single || values.is_empty() || values.len() > 64
        || !crate::catalog::standard_expression_source(payload, "ch.njol.skript.expressions.ExprEntity") { return Vec::new(); }
    let Some(model) = model() else { return Vec::new(); };
    if model.graph.assignable(public, ENTITY) != Some(true) || !model.stored(public,IMPLEMENTATION) { return Vec::new(); }
    for value in values {
        if value.time != 0 || value.context_dependent != Some(false)
            || value.has_custom_input_validator != Some(false) || value.has_custom_event_validator != Some(false) { return Vec::new(); }
        let Some(audited)=audited_event_route(value) else { return Vec::new(); };
        // Identity/downcast only; no opaque converter may enter the domain.
        if model.graph.assignable(public, &value.value_class) != Some(true)
            && model.graph.assignable(&value.value_class, public) != Some(true) { return Vec::new(); }
        let Ok(records) = crate::catalog::registration_records(audited) else { return Vec::new(); };
        let records = records.iter().filter(|r|r.document=="EventValues.json").collect::<Vec<_>>();
        if records.len()!=1 { return Vec::new(); }
        let Ok(record) = serde_json::from_slice::<Value>(&records[0].bytes) else { return Vec::new(); };
        if record["eventClass"].as_str()!=Some(value.event_class.as_str())
            || record["valueClass"].as_str()!=Some(value.value_class.as_str())
            || record["addon"]["name"]!="Skript" || record["addon"]["version"]!="2.15.4" { return Vec::new(); }
    }
    model.result(public, &values.iter().map(|v|v.registration_id.as_str()).collect::<Vec<_>>().join(";"))
}

#[cfg(target_arch="wasm32")]
static MODELS: LazyLock<RwLock<HashMap<String, Option<Arc<Model>>>>> = LazyLock::new(||RwLock::new(HashMap::new()));

/// Consumers never perform a cold registry transfer inside a body hook. A
/// cancelled Store restart or standalone parse without structure preparation
/// therefore retains Unknown, rather than exhausting a hook's fuel budget.
#[cfg(target_arch="wasm32")]
fn model() -> Option<Arc<Model>> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    if !crate::runtime::skript_version_is("2.15.4") { return None; }
    let source=catalog_data::source().ok()??;
    if source.format!="ssg" || source.schema_version!=7 { return None; }
    let key=format!("{}:{}",source.source_digest,source.snapshot_id);
    MODELS.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key).cloned().flatten()
}

#[cfg(target_arch="wasm32")]
fn load_model() -> Option<Arc<Model>> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    if !crate::runtime::skript_version_is("2.15.4") { return None; }
    let source = catalog_data::source().ok()??;
    if source.format!="ssg" || source.schema_version!=7 { return None; }
    let key = format!("{}:{}",source.source_digest,source.snapshot_id);
    if let Some(value) = MODELS.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key).cloned() { return value; }
    let result = (|| {
        let chunk = catalog_data::read_document("Manifest.json",0,64*1024).ok()??;
        if chunk.offset!=0 || chunk.bytes.len() as u64!=chunk.total_length { return None; }
        let manifest: Value = serde_json::from_slice(&chunk.bytes).ok()?;
        if !profile_for_runtime(&manifest,crate::runtime::current().as_ref().and_then(|p|p.skript_version.as_deref())) || manifest["snapshotId"]!=source.snapshot_id { return None; }
        let records = read_fields(&source,"Types.json", &["originalClass","typeParseOrder","addon","typeRuntime","serializeAs","hasParser","parserClass","registrationId"],1024)?;
        if records.is_empty() || records.iter().any(|r|r["addon"]["name"]!="Skript" || r["addon"]["version"]!="2.15.4") { return None; }
        let graph = graph_from_records(&records)?;
        Some(Arc::new(Model {records,graph,snapshot:source.snapshot_id.clone(),digest:source.source_digest.clone(),proofs:RwLock::new(HashMap::new())}))
    })();
    let after = catalog_data::source().ok()??;
    if after.source_digest!=source.source_digest || after.snapshot_id!=source.snapshot_id { return None; }
    let mut cache = MODELS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len()>=16 { cache.clear(); }
    cache.insert(key,result.clone());
    result
}

#[cfg(target_arch="wasm32")]
fn read_fields(source: &crate::nlaocs::skript_parser_addon::catalog_data::CatalogSource, document: &str, fields: &[&str], limit: usize) -> Option<Vec<Value>> {
    use crate::nlaocs::skript_parser_addon::catalog_data;
    let fields = fields.iter().map(|s|s.to_string()).collect::<Vec<_>>();
    let mut records = Vec::new();
    loop {
        let before = records.len();
        let page = catalog_data::read_record_fields(&source.source_digest,&source.snapshot_id,document,&fields,before as u64,128).ok()??;
        if page.items.len()>128 || before.checked_add(page.items.len())?>limit { return None; }
        for item in page.items {
            if item.index!=records.len() as u64 { return None; }
            records.push(serde_json::from_slice(&item.json).ok()?);
        }
        match page.next_offset {
            None => return Some(records),
            Some(next) if next==records.len() as u64 && next>before as u64 => {},
            _ => return None,
        }
    }
}

#[cfg(not(target_arch="wasm32"))]
fn model() -> Option<Arc<Model>> { None }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genuine_runtime_snapshot_proves_bounded_storage_not_all_public_implementations() {
        let manifest: Value=serde_json::from_str(include_str!("../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/Manifest.json")).unwrap();
        assert!(profile(&manifest));
        let records:Vec<Value>=serde_json::from_str(include_str!("../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/Types.json")).unwrap();
        let graph=graph_from_records(&records).unwrap();
        let model=Model {records,graph,snapshot:manifest["snapshotId"].as_str().unwrap().into(),digest:"fixture.raw-source-digest".into(),proofs:RwLock::new(HashMap::new())};
        for public in ["org.bukkit.entity.Player","org.bukkit.entity.Entity"] {
            assert!(model.stored(public,IMPLEMENTATION), "genuine bounded storage: {public}");
            assert!(model.stored(public,IMPLEMENTATION), "cached proof: {public}");
            let entries=model.result(public,"fixture.producer");
            let result:ResultType=serde_json::from_str(&entries[0].value).unwrap();
            assert_eq!(result.types,[public]); assert!(result.maybe_unset);
        }
        assert!(model.stored("java.lang.String","java.lang.String"),"declared class bound uses the same generic proof");
        assert!(!model.stored("org.bukkit.inventory.ItemStack","org.bukkit.inventory.ItemStack"),"custom cloner requires a separate result guarantee");
        let mut full:Vec<Class>=serde_json::from_str(include_str!("../../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain/ClassHierarchy.json")).unwrap();
        full.push(Class {name:IMPLEMENTATION.into(),kind:"Class".into(),super_class:Some("java.lang.Object".into()),interfaces:vec![ENTITY.into()],lineage_complete:false});
        let full=Graph::new(full).unwrap();
        for from in model.records.iter().filter_map(|r|r["originalClass"].as_str()).chain([SLOT,IMPLEMENTATION]) {
            for to in model.records.iter().filter_map(|r|r["originalClass"].as_str()).filter(|t|*t!="java.lang.Object") {
                assert_eq!(model.graph.assignable(from,to),full.assignable(from,to),"compact relation {from} -> {to}");
            }
        }
        assert!(!domain::preserves("org.bukkit.entity.Player","org.bukkit.entity.Player",&model.records,&model.graph,SLOT));
        for public in ["unknown.Custom","java.lang.String","org.bukkit.inventory.ItemStack"] { assert!(!model.stored(public,IMPLEMENTATION),"not this producer domain: {public}"); }
    }
    #[test]
    fn profile_requires_exact_backend_and_complete_plugin_inventory() {
        let mut m = serde_json::json!({"schemaVersion":7,"server":{"name":"Paper","version":"1.21.11-132-c5eb079 (MC: 1.21.11)","implementationClass":"org.bukkit.craftbukkit.CraftServer","implementationJarSha256":SERVER_SHA},"plugins":[{"loadOrder":0,"name":"SkriptSyntaxGenerator","main":"jp.nlaocs.skriptSyntaxGenerator.SkriptSyntaxGenerator","enabled":true,"jarSha256":GENERATOR_SHA},{"loadOrder":1,"name":"Skript","version":"2.15.4","main":"ch.njol.skript.Skript","enabled":true,"jarSha256":SKRIPT_SHA}]});
        assert!(profile(&m));
        assert!(profile_for_runtime(&m,Some("2.15.4")));
        for runtime in [None,Some("2.12.2"),Some("2.16.0")] { assert!(!profile_for_runtime(&m,runtime),"snapshot alone is insufficient runtime evidence"); }
        for field in ["implementationClass","implementationJarSha256","version"] {
            let mut changed=m.clone(); changed["server"][field]=Value::from("unknown"); assert!(!profile(&changed));
        }
        m["plugins"].as_array_mut().unwrap().push(serde_json::json!({"name":"addon"}));
        assert!(!profile(&m),"foreign event injection/custom runtime providers are unsupported");
    }
}
