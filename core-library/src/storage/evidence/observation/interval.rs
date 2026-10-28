//! Finite supported-API observations. The initialized/quiescent premise is
//! explicit; this is neither a JVM mutation epoch nor an all-deliveries claim.
use super::*;
use serde::{Deserialize,Serialize};

#[derive(Debug,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(in crate::storage) struct Interval {
    version:u32,scope:String,premise:String,registry_instance_id:Option<String>,
    before:Stamp,after:Stamp,type_details_before:Option<String>,type_details_after:Option<String>,
    event_values_before:Option<String>,event_values_after:Option<String>,coverage:Vec<Coverage>,complete:bool,
}
#[derive(Debug,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Stamp {lifetime:u64,sequence:u64,coverage_sequence:u64,in_flight:u32,incomplete:bool}
#[derive(Debug,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Coverage {class_id:String,methods:Vec<String>}
#[derive(Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct FactorySource {id:String,class_id:String,class_sha256:String,artifact_sha256:String}
#[derive(Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct FactoryMethod {id:String,source_id:String,name:String,descriptor:String}
#[derive(Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct FactorySite {
    id:String,method_id:String,bytecode_index:u32,event_class_id:String,value_class_id:String,factory_class_id:String,
    target_class_id:String,target_name:String,target_descriptor:String,reference_kind:u32,target_interface:bool,
    registration_method_id:String,registration_bytecode_index:u32,registry_class_id:String,
}

pub(super) fn check(o:&Observation)->Result<(),Failure> {
    if o.version==1 {
        return if o.interval.is_some()||!o.factory_sources.is_empty()||!o.factory_methods.is_empty()||!o.factory_sites.is_empty()
            ||o.getters.iter().any(|g|g.event_value_instance_id.is_some()||g.factory_site_id.is_some()) {Err(Failure::Version)}else{Ok(())};
    }
    let Some(interval)=&o.interval else {return if o.complete {Err(Failure::Incomplete)}else{Ok(())};};
    if interval.version!=1||interval.scope!="synchronous-export-supported-api"||interval.premise!="initialized-quiescent-registry"
        ||interval.coverage.len()>16||o.factory_sources.len()>64||o.factory_methods.len()>256||o.factory_sites.len()>512 {return Err(Failure::Version);}
    let classes=o.classes.iter().map(|c|(c.id.as_str(),c)).collect::<BTreeMap<_,_>>();
    let coherent=interval.registry_instance_id.as_ref().is_some_and(|id|bounded(id))
        &&interval.before.lifetime>0&&interval.before.lifetime==interval.after.lifetime
        &&interval.before.sequence==interval.after.sequence&&interval.before.coverage_sequence==interval.after.coverage_sequence&&interval.before.in_flight==0&&interval.after.in_flight==0
        &&!interval.before.incomplete&&!interval.after.incomplete
        &&interval.type_details_before.as_ref().is_some_and(|s|digest(s))&&interval.type_details_before==interval.type_details_after
        &&interval.event_values_before.as_ref().is_some_and(|s|digest(s))&&interval.event_values_before==interval.event_values_after;
    if interval.complete&&!coherent || o.complete&&!interval.complete {return Err(Failure::Generation);}
    let mut covered=BTreeSet::new();
    for row in &interval.coverage {
        if !covered.insert(&row.class_id)||!classes.contains_key(row.class_id.as_str())||row.methods.len()>16
            ||row.methods.iter().any(|s|!bounded(s))||row.methods.iter().collect::<BTreeSet<_>>().len()!=row.methods.len(){return Err(Failure::Identity);}
    }
    let sources=o.factory_sources.iter().map(|r|(r.id.as_str(),r)).collect::<BTreeMap<_,_>>();
    let methods=o.factory_methods.iter().map(|r|(r.id.as_str(),r)).collect::<BTreeMap<_,_>>();
    let sites=o.factory_sites.iter().map(|r|(r.id.as_str(),r)).collect::<BTreeMap<_,_>>();
    if sources.len()!=o.factory_sources.len()||methods.len()!=o.factory_methods.len()||sites.len()!=o.factory_sites.len(){return Err(Failure::Identity);}
    for source in &o.factory_sources {
        if !bounded(&source.id)||!classes.contains_key(source.class_id.as_str())||!digest(&source.class_sha256)||!digest(&source.artifact_sha256)
            ||classes[source.class_id.as_str()].code_origin_sha256.as_deref()!=Some(&source.artifact_sha256){return Err(Failure::Identity);}
    }
    for method in &o.factory_methods {
        if !bounded(&method.id)||!sources.contains_key(method.source_id.as_str())||!bounded(&method.name)||!bounded(&method.descriptor){return Err(Failure::Identity);}
    }
    for site in &o.factory_sites {
        if !bounded(&site.id)||!methods.contains_key(site.method_id.as_str())||!methods.contains_key(site.registration_method_id.as_str())
            ||site.bytecode_index>65535||site.registration_bytecode_index>65535||!bounded(&site.target_name)||!bounded(&site.target_descriptor)
            ||!matches!(site.reference_kind,5|6|7|9)
            ||[&site.event_class_id,&site.value_class_id,&site.factory_class_id,&site.target_class_id,&site.registry_class_id].iter().any(|id|!classes.contains_key(id.as_str())){return Err(Failure::Identity);}
    }
    let mut objects=BTreeSet::new();
    for getter in &o.getters {
        if getter.event_value_instance_id.as_ref().is_none_or(|id|!bounded(id)||!objects.insert(id))
            ||getter.factory_site_id.as_ref().is_some_and(|id|sites.get(id.as_str()).is_none_or(|s|s.event_class_id!=getter.event_class_id||s.value_class_id!=getter.value_class_id)) {return Err(Failure::Identity);}
    }
    Ok(())
}

pub(in crate::storage) const BASIS:&str="audited-standard-native-event-target-v1";
const SKRIPT:&str="277393a965e164e854f5280aa1c30ce92e69b3fab4df5063711743f9d7847327";
const API:&str="c577b181c11a8674310e56c92a91e31c010b7f04c9bd10b91c3be18374401070";
const BASE:&str="ch.njol.skript.classes.ClassInfo";
const EVENT:&str="org.bukkit.event.player.PlayerEvent";
const VALUE:&str="org.bukkit.entity.Player";
const FACTORY:&str="org.skriptlang.skript.bukkit.lang.eventvalue.EventValue";
const REGISTRY:&str="org.skriptlang.skript.bukkit.lang.eventvalue.EventValueRegistryImpl";
const CALLER:&str="ch.njol.skript.classes.data.BukkitEventValues";
const SIMPLE:&str="simple(Ljava/lang/Class;Ljava/lang/Class;Lorg/skriptlang/skript/lang/converter/Converter;)Lorg/skriptlang/skript/bukkit/lang/eventvalue/EventValue;";
const REGISTER:&str="register(Lorg/skriptlang/skript/bukkit/lang/eventvalue/EventValue;)V";
const UNREGISTER:&str="unregister(Lorg/skriptlang/skript/bukkit/lang/eventvalue/EventValue;)Z";

#[derive(Debug)]
pub(in crate::storage) struct NativeTarget {pub getter:String}

impl BoundObservation {
    /// Authorize the audited source rule only after full structural projection.
    /// `collector_sha` is the shipped, reviewed collector artifact, not a value
    /// copied from a supplied Manifest. No plugin-count condition is used.
    pub(in crate::storage) fn standard_native_target(&self,manifest:&Value,collector_sha:&str)->Option<NativeTarget> {
        let o=&self.observation;let interval=o.interval.as_ref()?;
        if o.version!=2||!o.complete||!interval.complete||!digest(collector_sha)
            ||manifest["server"]["implementationJarSha256"]!="3a73f0cc4020a5eb45629a67629b14263331c68de45552f8505df8060fa8ed43"
            ||manifest["server"]["implementationClass"]!="org.bukkit.craftbukkit.CraftServer"||manifest["server"]["name"]!="Paper"||manifest["server"]["version"]!="1.21.11-132-c5eb079 (MC: 1.21.11)" {return None;}
        let plugins=manifest["plugins"].as_array()?;
        for (name,main,sha) in [("Skript","ch.njol.skript.Skript",SKRIPT),("SkriptSyntaxGenerator","jp.nlaocs.skriptSyntaxGenerator.SkriptSyntaxGenerator",collector_sha)] {
            let rows=plugins.iter().filter(|p|p["name"]==name).collect::<Vec<_>>();
            if rows.len()!=1||rows[0]["main"]!=main||rows[0]["enabled"]!=true||rows[0]["jarSha256"]!=sha{return None;}
        }
        let unique=|name:&str,sha:&str|->Option<&JavaClass>{let mut rows=o.classes.iter().filter(|c|c.name==name&&c.catalog_name==name);let row=rows.next()?;
            (rows.next().is_none()&&!row.hidden&&row.code_origin_sha256.as_deref()==Some(sha)&&row.loader_id.is_some()).then_some(row)};
        let base=unique(BASE,SKRIPT)?;let factory=unique(FACTORY,SKRIPT)?;let registry=unique(REGISTRY,SKRIPT)?;let caller=unique(CALLER,SKRIPT)?;
        let event=unique(EVENT,API)?;let value=unique(VALUE,API)?;
        if [base,factory,registry,caller].iter().any(|c|c.loader_id!=base.loader_id)||event.loader_id!=value.loader_id{return None;}
        for (class,required) in [(base,vec!["cloner(Lch/njol/skript/classes/Cloner;)Lch/njol/skript/classes/ClassInfo;","serializeAs(Ljava/lang/Class;)Lch/njol/skript/classes/ClassInfo;","user([Ljava/lang/String;)Lch/njol/skript/classes/ClassInfo;"]),
            (factory,vec![SIMPLE]),(registry,vec![REGISTER,UNREGISTER])] {
            let coverage=interval.coverage.iter().find(|r|r.class_id==class.id)?;
            if required.iter().any(|method|!coverage.methods.iter().any(|m|m==method)){return None;}
        }
        let mut class_registry=o.classes.iter().filter(|c|c.name=="ch.njol.skript.registrations.Classes"&&c.code_origin_sha256.as_deref()==Some(SKRIPT)&&c.loader_id==base.loader_id);
        let classes=class_registry.next()?;if class_registry.next().is_some(){return None;}
        let coverage=interval.coverage.iter().find(|r|r.class_id==classes.id)?;
        for required in ["registerClass(Lch/njol/skript/classes/ClassInfo;)V","onRegistrationsStop()V"]{if !coverage.methods.iter().any(|s|s==required){return None;}}
        // Names never substitute for actual declaring-class identity/loader.
        let by_id=o.classes.iter().map(|c|(c.id.as_str(),c)).collect::<BTreeMap<_,_>>();
        for entry in &o.entries {for role in ["classGetterDeclaringClass","cloneMethodDeclaringClass","clonerGetterDeclaringClass","serializeAsGetterDeclaringClass"] {
            if let Some(id)=entry.runtime_class_ids.get(role) {
                let class=by_id.get(id.as_str())?;
                if class.catalog_name==BASE&&class.id!=base.id{return None;}
            }
        }}
        // The existing storage Graph uses catalog names. Do not collapse two
        // actual Class/loader tokens into one node, including ancestor tokens.
        // Unrelated getter-only class rows do not participate in that Graph.
        let mut graph_classes=BTreeMap::new();
        for entry in &o.entries {
            let class=by_id.get(entry.class_id.as_deref()?)?;
            for used in std::iter::once(*class).chain(class.ancestors.iter().map(|id|by_id[id.as_str()])) {
                if graph_classes.insert(used.catalog_name.as_str(),used).is_some_and(|prior:&JavaClass|prior.id!=used.id){return None;}
            }
        }
        for (name,sha,loader) in [("org.bukkit.entity.Entity",API,&value.loader_id),("ch.njol.skript.util.slot.Slot",SKRIPT,&base.loader_id)] {
            let class=graph_classes.get(name)?;
            if class.name!=name||class.hidden||class.code_origin_sha256.as_deref()!=Some(sha)||&class.loader_id!=loader{return None;}
        }
        // This node is supplied by the pinned source model. A captured token
        // with the same spelling cannot be identified with it by name alone.
        if graph_classes.contains_key("org.bukkit.craftbukkit.entity.CraftEntity"){return None;}
        let methods=o.factory_methods.iter().map(|m|(m.id.as_str(),m)).collect::<BTreeMap<_,_>>();
        let sources=o.factory_sources.iter().map(|s|(s.id.as_str(),s)).collect::<BTreeMap<_,_>>();
        let mut matches=Vec::new();
        for getter in &o.getters {
            if getter.event_class_id!=event.id||getter.value_class_id!=value.id{continue;}
            let Some(site)=getter.factory_site_id.as_ref().and_then(|id|o.factory_sites.iter().find(|s|&s.id==id))else{continue;};
            let method=methods[site.method_id.as_str()];let registration=methods[site.registration_method_id.as_str()];
            if site.factory_class_id==factory.id&&site.registry_class_id==registry.id&&site.target_class_id==event.id
                &&site.target_name=="getPlayer"&&site.target_descriptor=="()Lorg/bukkit/entity/Player;"&&site.reference_kind==5&&!site.target_interface
                &&sources[method.source_id.as_str()].class_id==caller.id&&sources[registration.source_id.as_str()].class_id==caller.id
                &&method.name=="register"&&method.descriptor=="(Lorg/skriptlang/skript/bukkit/lang/eventvalue/EventValueRegistry;)V"
                &&registration.name=="register"&&registration.descriptor==method.descriptor {
                matches.push(getter.registration_id.clone());
            }
        }
        (matches.len()==1).then(||NativeTarget{getter:matches.remove(0)})
    }
}

#[cfg(test)]
pub(in crate::storage) mod tests;
