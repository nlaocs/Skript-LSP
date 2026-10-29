//! The public parse lifecycle must also revoke already materialized UI results.
use super::*;

#[test]
fn transfer_hints_are_revoked_on_close_and_rebuilt_for_the_new_document() {
    let project = "file:///storage-transfer";
    let document = "file:///storage-transfer/test.sk";
    let source = "on join:\n    set {_source} to \"value\"\n    set {_copy} to {_source}\n";
    let mut host = ParserHost::new(crate::core_library_component(), HostConfig {
        syntax_catalog: Some(catalog()),
        runtime_profile: RuntimeProfile {
            skript_version: Some("2.15.4".into()),
            ..RuntimeProfile::default()
        },
        ..HostConfig::default()
    }).unwrap();
    let build = |host: &mut ParserHost, revision| {
        let result = host.parse_document(
            DocumentParseRequest::new(project, document, revision, source),
            DocumentParserConfig::default(),
        ).unwrap();
        assert!(result.component_failures.is_empty(), "{:?}", result.component_failures);
        assert!(result.syntax.diagnostics.is_empty(), "{:?}", result.syntax.diagnostics);
        let semantic = semantic::Document::from_result_with_host(
            &result, Arc::from(source), document.parse().unwrap(), catalog(), host, None,
        ).unwrap();
        assert!(semantic.transfer_certificate().is_some(), "must exercise a real transfer, not the legacy fallback");
        let mut inlay = crate::lsp::inlay::Document::from_result(&result, Arc::from(source), catalog()).unwrap();
        inlay.add_variable_types(&semantic).unwrap();
        (semantic, inlay)
    };
    let range = lsp_types::Range::new(Position::new(0, 0), LineIndex::new(source).position(source, source.len()).unwrap());
    let (semantic, inlay) = build(&mut host, 1);
    assert_eq!(semantic.write_type_hints().len(), 2);
    assert_eq!(inlay.at(range).len(), 2);
    assert!(host.release_document(project, document, 1).unwrap());
    assert!(!semantic.is_current());
    assert!(semantic.write_type_hints().is_empty());
    assert!(inlay.at(range).is_empty(), "materialized transfer hints must obey the same fence");
    let (reopened, reopened_inlay) = build(&mut host, 2);
    assert!(reopened.is_current());
    assert_eq!(reopened.write_type_hints().len(), 2);
    assert_eq!(reopened_inlay.at(range).len(), 2);
    assert!(!semantic.is_current(), "reopen must not resurrect the old certificate");
}
