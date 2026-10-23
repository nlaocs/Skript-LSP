use super::*;
use std::cell::Cell;

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
fn property_source_missing_facts_and_overrides_never_certify_storage() {
    for field in ["kind", "definitionId", "registrationId", "elementClass", "superClass", "patterns", "addon"] {
        let mut changed = record();
        changed.as_object_mut().unwrap().remove(field);
        assert_eq!(from_records(&key(), &records(changed), |_| panic!("incomplete record queried")), None);
    }
    let mut malformed = records(record());
    malformed[0].bytes = b"not JSON".to_vec();
    assert_eq!(from_records(&key(), &malformed, |_| panic!("malformed record queried")), None);
    for missing_or_override in [None, Some(true)] {
        for target in 0..3 {
            let calls = Cell::new(0);
            let result = from_records(&key(), &records(record()), |_| {
                let current = calls.get();
                calls.set(current + 1);
                if current == target { missing_or_override } else { Some(false) }
            });
            assert_eq!(result, missing_or_override.map(|_| false));
            assert_eq!(calls.get(), target + 1);
        }
    }
    let mut queried = Vec::new();
    assert_eq!(from_records(&key(), &records(record()), |class| {
        queried.push(class.to_owned());
        Some(false)
    }), Some(true));
    assert_eq!(queried, [key().class, PROPERTY_BASE.into(), "ch.njol.skript.lang.util.SimpleExpression".into()]);
}
