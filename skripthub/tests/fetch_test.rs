use skripthub::api::types::AbstractAddonSyntaxList;

#[test]
fn test_parse_real_response() {
    let data = std::fs::read_to_string("tests/data/addonsyntaxlist.json")
        .expect("Failed to read test data file");
    let parsed: AbstractAddonSyntaxList =
        serde_json::from_str(&data).expect("Failed to parse JSON data");
    assert!(!parsed.is_empty());
}
