use skript_parser::{FunctionArgument, FunctionCall};

fn call() -> FunctionCall {
    FunctionCall {
        name: "log".into(),
        definition_id: "fixture:log".into(),
        registration_id: "fixture:log:0".into(),
        arguments: vec![
            FunctionArgument {
                parameter_name: "n".into(),
                supplied_name: Some("n".into()),
                child_start: 0,
                child_count: 1,
                omitted: false,
            },
            FunctionArgument {
                parameter_name: "base".into(),
                supplied_name: None,
                child_start: 1,
                child_count: 0,
                omitted: true,
            },
        ],
    }
}

#[test]
fn function_child_ownership_accepts_omitted_plural_and_zero_parameter_calls() {
    call().validate_children(1).unwrap();
    let mut plural = call();
    plural.arguments.truncate(1);
    plural.arguments[0].child_count = 2;
    plural.validate_children(2).unwrap();
    plural.arguments.clear();
    plural.validate_children(0).unwrap();
}

#[test]
fn function_child_ownership_rejects_corrupt_provider_mappings() {
    let mutations: [fn(&mut FunctionCall); 11] = [
        |call| call.name.clear(),
        |call| call.definition_id = " ".into(),
        |call| call.registration_id.clear(),
        |call| call.arguments[0].parameter_name.clear(),
        |call| call.arguments[1].parameter_name = "n".into(),
        |call| call.arguments[0].supplied_name = Some(" ".into()),
        |call| call.arguments[0].child_start = usize::MAX,
        |call| call.arguments[1].child_start = 0,
        |call| call.arguments[0].omitted = true,
        |call| call.arguments[1].supplied_name = Some("base".into()),
        |call| call.arguments.clear(),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut value = call();
        mutate(&mut value);
        assert!(value.validate_children(1).is_err(), "mutation {index}");
    }
    assert!(call().validate_children(0).is_err());
    assert!(call().validate_children(2).is_err());
}
