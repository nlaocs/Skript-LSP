use super::*;
use std::cell::Cell;

#[test]
#[ignore = "manual native key/lookup cost model; not real Wasm time; requires public measurement plan"]
fn measure_property_certificate_key_cost() {
    use std::{hint::black_box, time::Instant};
    let plan = std::fs::read(std::env::var("SKRIPT_STORAGE_MEASUREMENT_PLAN").unwrap()).unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&plan).unwrap();
    let model: Key = serde_json::from_value(plan["keyModel"].clone()).unwrap();
    assert!(model.cacheable());
    let cache = RwLock::new(HashMap::new());
    assert_eq!(cached(&cache, &model, || Some(true)), Some(true));
    let sample = |run: &mut dyn FnMut()| {
        let started = Instant::now();
        for _ in 0..10_000 { run(); }
        started.elapsed().as_secs_f64() * 1e9 / 10_000.0
    };
    let mut rows = Vec::new();
    for round in 0..7 {
        let clone_ns = sample(&mut || { black_box(model.clone()); });
        let bounded_key_check_ns = sample(&mut || { black_box(model.cacheable()); });
        let clone_and_hit_ns = sample(&mut || {
            let key = black_box(model.clone());
            assert_eq!(black_box(cached(&cache, &key, || panic!("warm key unexpectedly missed"))), Some(true));
        });
        let previous_authentication_key_ns = sample(&mut || {
            black_box(serde_json::json!([model.source.digest, "Expressions.json", model.definition,
                model.registration, model.class, model.index, model.pattern]).to_string());
        });
        rows.push(serde_json::json!({"round":round,"cloneNs":clone_ns,"boundedKeyCheckNs":bounded_key_check_ns,
            "cloneAndCacheHitNs":clone_and_hit_ns,"previousAuthenticationKeyNs":previous_authentication_key_ns}));
    }
    println!("storage_key_cost_model {}", serde_json::json!({"rows":rows,
        "ownedStringBytes":model.source.format.len()+model.source.snapshot.len()+model.source.digest.len()+model.definition.len()+model.registration.len()+model.class.len()+model.pattern.len(),
        "iterationsPerSample":10_000,
        "limits":"Native debug-test cost model with actual public snapshot key lengths. Excludes runtime checks, source import, WIT lifting/lowering and Wasm execution. Old serialized key also excludes its source import and cached lookup. No end-to-end speedup is asserted."}));
}

fn key() -> Key {
    Key {
        source: Source {
            format: "fixture".into(),
            schema: 7,
            snapshot: "snapshot".into(),
            digest: "digest".into(),
        },
        definition: "expression:fixture".into(),
        registration: "expression:fixture:0".into(),
        class: "fixture.Property".into(),
        index: 0,
        pattern: "value of %objects%".into(),
    }
}

fn record() -> serde_json::Value {
    let key = key();
    serde_json::json!({
        "kind": "expression", "definitionId": key.definition,
        "registrationId": key.registration, "elementClass": key.class,
        "patterns": [key.pattern], "superClass": PROPERTY_BASE,
        "addon": {"name": "Skript", "version": "2.15.4"}
    })
}

fn records(record: serde_json::Value) -> Vec<SourceRecord> {
    vec![SourceRecord {
        document: "Expressions.json".into(),
        bytes: serde_json::to_vec(&record).unwrap(),
    }]
}

#[test]
fn source_cache_reuses_definite_results_but_not_unknown() {
    for result in [true, false] {
        let cache = RwLock::new(HashMap::new());
        let key = key();
        assert_eq!(cached(&cache, &key, || Some(result)), Some(result));
        assert_eq!(cached(&cache, &key, || panic!("known source queried again")), Some(result));
    }
    let cache = RwLock::new(HashMap::new());
    let key = key();
    assert_eq!(cached(&cache, &key, || None), None);
    assert!(cache.read().unwrap().is_empty());
    assert_eq!(cached(&cache, &key, || Some(true)), Some(true));
}

#[test]
fn source_cache_keys_every_immutable_dependency() {
    let cache = RwLock::new(HashMap::new());
    let original = key();
    assert_eq!(cached(&cache, &original, || Some(true)), Some(true));
    for field in 0..9 {
        let mut key = original.clone();
        match field {
            0 => key.source.format.push('2'),
            1 => key.source.schema += 1,
            2 => key.source.snapshot.push('2'),
            3 => key.source.digest.push('2'),
            4 => key.definition.push('2'),
            5 => key.registration.push('2'),
            6 => key.class.push('2'),
            7 => key.index += 1,
            _ => key.pattern.push('2'),
        }
        assert_eq!(cached(&cache, &key, || Some(false)), Some(false), "field {field}");
    }
    assert_eq!(cached(&cache, &original, || panic!("original lost")), Some(true));
}

#[test]
fn source_cache_bounds_count_and_owned_key_bytes_without_skipping_checks() {
    let cache = RwLock::new(HashMap::new());
    for index in 0..=super::super::MAX_CACHED_CHANGE_CONTRACTS {
        let mut key = key();
        key.index = index as u64;
        assert_eq!(cached(&cache, &key, || Some(true)), Some(true));
        assert!(cache.read().unwrap().len() <= super::super::MAX_CACHED_CHANGE_CONTRACTS);
    }
    let before = cache.read().unwrap().len();
    let mut key = key();
    key.pattern = "x".repeat(MAX_KEY_BYTES + 1);
    let calls = Cell::new(0);
    for _ in 0..2 {
        assert_eq!(cached(&cache, &key, || {
            calls.set(calls.get() + 1);
            Some(true)
        }), Some(true));
    }
    assert_eq!(calls.get(), 2);
    assert_eq!(cache.read().unwrap().len(), before);
}

#[test]
fn source_cache_fences_positive_and_negative_results_before_retaining_them() {
    for known in [true, false] {
        let key = key();
        let mut value = record();
        if !known {
            value["superClass"] = "fixture.OtherBase".into();
        }
        let records = records(value);
        let mut changed = key.source.clone();
        changed.digest.push('2');
        for after in [None, Some(changed)] {
            let cache = RwLock::new(HashMap::new());
            assert_eq!(cached(&cache, &key, || {
                certify_source(&key, &records, |_| Some(false), || after)
            }), None);
            assert!(cache.read().unwrap().is_empty());
            assert_eq!(cached(&cache, &key, || {
                certify_source(&key, &records, |_| Some(false), || Some(key.source.clone()))
            }), Some(known));
            assert_eq!(cached(&cache, &key, || panic!("source was needlessly rechecked")), Some(known));
        }
    }
}

#[test]
fn property_source_rejects_nonmatching_registration_before_method_queries() {
    let original = record();
    for field in ["kind", "definitionId", "registrationId", "elementClass", "superClass"] {
        let mut changed = original.clone();
        changed[field] = "different".into();
        assert_eq!(from_records(&key(), &records(changed), |_| panic!("unrelated source queried")), Some(false));
    }
    for field in ["name", "version"] {
        let mut changed = original.clone();
        changed["addon"][field] = "different".into();
        assert_eq!(from_records(&key(), &records(changed), |_| panic!("foreign owner queried")), Some(false));
    }
    let mut changed = original.clone();
    changed["patterns"][0] = "different".into();
    assert_eq!(from_records(&key(), &records(changed), |_| panic!("wrong pattern queried")), Some(false));
    assert_eq!(from_records(&key(), &[], |_| panic!("absent registration queried")), Some(false));
    let mut duplicate = records(original.clone());
    duplicate.extend(records(original));
    assert_eq!(from_records(&key(), &duplicate, |_| panic!("ambiguous registration queried")), Some(false));
}

#[test]
fn property_source_missing_or_malformed_facts_never_certify_storage() {
    for field in ["kind", "definitionId", "registrationId", "elementClass", "superClass", "patterns", "addon"] {
        let mut changed = record();
        changed.as_object_mut().unwrap().remove(field);
        assert_eq!(from_records(&key(), &records(changed), |_| panic!("incomplete record queried")), None);
    }
    let mut malformed = records(record());
    malformed[0].bytes = b"not JSON".to_vec();
    assert_eq!(from_records(&key(), &malformed, |_| panic!("malformed record queried")), None);

}

#[test]
fn both_property_families_require_the_complete_override_free_storage_path() {
    for (base,path) in [
        (PROPERTY_BASE,vec![PROPERTY_BASE,SIMPLE_EXPRESSION]),
        (LEGACY_SIMPLE_PROPERTY,vec![LEGACY_SIMPLE_PROPERTY,LEGACY_PROPERTY,SIMPLE_EXPRESSION]),
        (LEGACY_PROPERTY,vec![LEGACY_PROPERTY,SIMPLE_EXPRESSION]),
    ] {
        let mut value=record();
        value["superClass"]=base.into();
        let records=records(value);
        let mut expected=vec![key().class];
        expected.extend(path.into_iter().map(str::to_owned));
        let mut queried=Vec::new();
        assert_eq!(from_records(&key(),&records,|class|{queried.push(class.to_owned());Some(false)}),Some(true));
        assert_eq!(queried,expected);
        for (index,rejected) in expected.iter().enumerate() {
            for outcome in [None,Some(true)] {
                let mut queried=Vec::new();
                let actual=from_records(&key(),&records,|class|{
                    queried.push(class.to_owned());
                    if class==rejected {outcome}else{Some(false)}
                });
                assert_eq!(actual,outcome.map(|_|false),"{base}: {rejected}");
                assert_eq!(queried,expected[..=index],"stop at the first unknown/override: {base}: {rejected}");
            }
        }
    }
}
