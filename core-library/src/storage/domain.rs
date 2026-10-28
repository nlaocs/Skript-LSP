//! Implementation domains are evidence about a producer, not public type aliases.
//! This checker knows no Bukkit type names and never executes a callback.
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const BASE: &str = "ch.njol.skript.classes.ClassInfo";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Class {
    pub name: String,
    pub kind: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub lineage_complete: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Graph(BTreeMap<String, Class>);

impl Graph {
    pub fn new(classes: Vec<Class>) -> Option<Self> {
        if classes.is_empty() || classes.len() > 8192 {
            return None;
        }
        let count = classes.len();
        let map: BTreeMap<_, _> = classes.into_iter().map(|c| (c.name.clone(), c)).collect();
        (map.len() == count && map.values().all(|c| !c.name.is_empty()
            && matches!(c.kind.as_str(), "Class" | "Interface" | "Enum" | "Array" | "Primitive" | "Annotation" | "Record" | "Sealed" | "Synthetic" | "MemberClass" | "LocalClass" | "AnonymousClass")))
            .then_some(Self(map))
    }

    /// A missing ancestor is unknown, never proof of disjointness.
    pub fn assignable(&self, from: &str, to: &str) -> Option<bool> {
        let mut pending = vec![from];
        let mut seen = BTreeSet::new();
        let mut complete = true;
        while let Some(name) = pending.pop() {
            if name == to { return Some(true); }
            if !seen.insert(name) { continue; }
            if seen.len() > 8192 { return None; }
            let Some(class) = self.0.get(name) else { complete = false; continue; };
            if class.lineage_complete {
                if class.interfaces.iter().any(|ancestor|ancestor==to) || class.super_class.as_deref()==Some(to) { return Some(true); }
                continue;
            }
            if let Some(parent) = &class.super_class { pending.push(parent); }
            pending.extend(class.interfaces.iter().map(String::as_str));
        }
        complete.then_some(false)
    }

    fn concrete(&self, name: &str) -> Option<bool> {
        self.0.get(name).map(|c| matches!(c.kind.as_str(), "Class" | "Enum" | "Record" | "Sealed" | "Synthetic" | "MemberClass" | "LocalClass" | "AnonymousClass"))
    }

    /// Two unrelated Java classes cannot have a shared subtype. Interfaces can.
    fn overlap(&self, a: &str, b: &str) -> Option<bool> {
        if self.assignable(a, b)? || self.assignable(b, a)? { return Some(true); }
        Some(!(self.concrete(a)? && self.concrete(b)?))
    }
}

fn represented_class(record: &Value) -> bool {
    let r = &record["typeRuntime"];
    matches!(r["version"].as_u64(),Some(2 | 3))
        && r["classGetterDeclaringClass"] == BASE
        && r["classState"] == "present"
        && r["classValue"] == record["originalClass"]
        && r["classUnknownReason"].is_null()
}

fn base_clone(record: &Value) -> bool {
    let r = &record["typeRuntime"];
    matches!(r["version"].as_u64(),Some(2 | 3))
        && r["classGetterDeclaringClass"] == BASE
        && r["classState"] == "present"
        && r["classValue"] == record["originalClass"]
        && r["classUnknownReason"].is_null()
        && r["cloneMethodState"] == "known"
        && r["cloneMethodDeclaringClass"] == BASE
        && r["clonerGetterDeclaringClass"] == BASE
        && r["clonerState"] == "absent"
        && r["clonerClass"].is_null()
        && r["cloneMethodUnknownReason"].is_null()
        && r["clonerUnknownReason"].is_null()
        && r["classInfoClass"].as_str().is_some_and(|s| !s.is_empty())
}

fn storage_identity(record: &Value, public: &str, implementation: &str, graph: &Graph) -> bool {
    let r = &record["typeRuntime"];
    if !base_clone(record)
        || r["serializeAsGetterDeclaringClass"] != BASE
        || !r["serializeAsUnknownReason"].is_null() { return false; }
    match r["serializeAsState"].as_str() {
        Some("absent") => r["serializeAsClass"].is_null() && record["serializeAs"].is_null(),
        Some("present") => r["serializeAsClass"].as_str().is_some_and(|target| {
            record["serializeAs"].as_str() == Some(target)
                && (graph.assignable(public, target) == Some(true)
                    || graph.assignable(implementation, target) == Some(true)
                    || record["originalClass"].as_str().is_some_and(|class| graph.assignable(class, target) == Some(true)))
        }),
        _ => false,
    }
}

/// Geometry of a retained registry projection, not authentication of its
/// completeness, collection generation, Java identity or producer domain.
/// Legacy callers retain their separately audited source/profile assumptions.
pub(super) struct RegistryGeometry<'a> {
    records: &'a [Value],
    graph: &'a Graph,
}

impl<'a> RegistryGeometry<'a> {
    pub(super) fn check(records: &'a [Value], graph: &'a Graph) -> Option<Self> {
        if records.is_empty() || records.len() > 1024 { return None; }
        let mut classes = BTreeSet::new();
        for (index, record) in records.iter().enumerate() {
            let class = record["originalClass"].as_str()?;
            if !represented_class(record) || class.is_empty()
                || record["typeParseOrder"].as_u64() != Some(index as u64)
                || !classes.insert(class) || graph.concrete(class).is_none() { return None; }
        }
        Some(Self { records, graph })
    }

    pub(super) fn preserves(&self, public: &str, implementation: &str, slot: &str) -> bool {
        preserves_checked(public, implementation, self.records, self.graph, slot)
    }
}

/// Every non-null value is in public ∩ implementation. Completeness and
/// provenance of that producer domain must be established by the caller.
/// Exact runtime-class registrations beat ordered getSuperClassInfo scanning,
/// including exact registrations AFTER its first guaranteed ordered match.
pub(super) fn preserves(
    public: &str, implementation: &str, records: &[Value], graph: &Graph, slot: &str,
) -> bool {
    RegistryGeometry::check(records, graph)
        .is_some_and(|registry| registry.preserves(public, implementation, slot))
}

fn preserves_checked(public: &str, implementation: &str, records: &[Value], graph: &Graph, slot: &str) -> bool {
    if graph.concrete(implementation) != Some(true)
        || graph.overlap(implementation, public) != Some(true)
        || graph.overlap(implementation, slot) != Some(false) { return false; }
    for record in records {
        let class = record["originalClass"].as_str().expect("checked registry geometry");
        // Exact-class dispatch does not apply to interface class tokens.
        if graph.concrete(class) == Some(true)
            && graph.assignable(class, implementation) != Some(false)
            && graph.assignable(class, public) != Some(false)
            && !storage_identity(record, public, implementation, graph) { return false; }
    }
    for record in records {
        let class = record["originalClass"].as_str().expect("validated class");
        if graph.overlap(class, implementation) != Some(false)
            && graph.overlap(class, public) != Some(false)
            && !storage_identity(record, public, implementation, graph) { return false; }
        if graph.assignable(implementation, class) == Some(true)
            || graph.assignable(public, class) == Some(true) { return true; }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn class(name: &str, kind: &str, parent: Option<&str>, interfaces: &[&str]) -> Class {
        Class { name: name.into(), kind: kind.into(), super_class: parent.map(str::to_owned),
            interfaces: interfaces.iter().map(|s| s.to_string()).collect(), lineage_complete:false }
    }
    fn graph() -> Graph {
        Graph::new(vec![class("Object", "Class", None, &[]),
            class("Public", "Interface", None, &["Super"]),
            class("Super", "Interface", None, &[]),
            class("Native", "Class", Some("Object"), &["Public"]),
            class("Child", "Class", Some("Native"), &[]),
            class("Other", "Class", Some("Object"), &[]),
            class("Foreign", "Interface", None, &[]),
            class("Slot", "Class", Some("Object"), &[])]).unwrap()
    }
    fn record(class: &str, order: u64) -> Value {
        json!({"originalClass":class,"typeParseOrder":order,"typeRuntime":{
            "version":2,"classInfoClass":"fixture.Info","classGetterDeclaringClass":BASE,
            "classState":"present","classValue":class,"cloneMethodState":"known",
            "cloneMethodDeclaringClass":BASE,"clonerGetterDeclaringClass":BASE,
            "clonerState":"absent","serializeAsGetterDeclaringClass":BASE,"serializeAsState":"absent"}})
    }
    #[test]
    fn generic_domain_dispatch_matrix() {
        let graph = graph();
        let clean = vec![record("Other",0),record("Public",1),record("Object",2)];
        let check = |r: &[Value]| preserves("Public","Native",r,&graph,"Slot");
        assert!(check(&clean));
        let mut redirect = clean.clone();
        redirect[1]["serializeAs"] = json!("Super");
        redirect[1]["typeRuntime"]["serializeAsState"] = json!("present");
        redirect[1]["typeRuntime"]["serializeAsClass"] = json!("Super");
        assert!(check(&redirect), "serialization to an existing supertype is identity");
        redirect[1]["serializeAs"] = json!("Other");
        redirect[1]["typeRuntime"]["serializeAsClass"] = json!("Other");
        assert!(!check(&redirect), "a real conversion needs independent evidence");
        for field in ["classGetterDeclaringClass","cloneMethodDeclaringClass","clonerGetterDeclaringClass","serializeAsGetterDeclaringClass"] {
            let mut changed = clean.clone(); changed[1]["typeRuntime"][field] = json!("addon.Override");
            assert!(!check(&changed), "{field}");
        }
        let mut late = clean.clone();
        late.push(record("Child",3)); late[3]["typeRuntime"]["clonerState"] = json!("present");
        assert!(!check(&late), "late exact dispatch must not be overlooked");
        for field in ["classGetterDeclaringClass", "classState", "classValue"] {
            let mut changed = clean.clone(); changed[0]["typeRuntime"][field] = json!("unknown");
            assert!(!check(&changed), "even an excluded ClassInfo must have reliable getC: {field}");
        }
        let mut unrelated = clean.clone(); unrelated[0]["typeRuntime"]["clonerState"] = json!("present");
        assert!(check(&unrelated), "unrelated concrete callback cannot receive domain values");
        let mut interface = vec![record("Foreign",0),record("Public",1),record("Object",2)];
        interface[0]["typeRuntime"]["clonerState"] = json!("present");
        assert!(!check(&interface), "overlapping open interfaces cannot be excluded");
        assert!(!preserves("Public","Public",&clean,&graph,"Slot"), "public interface alone is not an implementation domain");
        assert!(!preserves("Public","Native",&clean,&graph,"missing.Slot"));
        assert!(!check(&clean[..1]), "missing registry suffix cannot prove dispatch");
        let mut old = clean.clone(); old[1]["typeRuntime"]["version"] = json!(1);
        assert!(!check(&old));
    }
    #[test]
    fn graph_missing_data_and_duplicate_classes_fail_closed() {
        let mut g = graph(); g.0.remove("Object");
        assert_eq!(g.assignable("Native","Other"),None);
        assert_eq!(g.assignable("Native","Public"),Some(true));
        let a = class("A","Class",None,&[]);
        assert!(Graph::new(vec![a.clone(),a]).is_none());
    }
}
