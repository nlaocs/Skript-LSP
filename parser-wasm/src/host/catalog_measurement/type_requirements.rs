//! Checks the actual Wasm manifests, not the native Core cfg branch.
use super::*;

fn identity(option: &crate::bindings::nlaocs::skript_parser_addon::types::ExpressionTypeOption)
    -> RegisteredSyntaxIdentity<'_> {
    RegisteredSyntaxIdentity {
        kind: CatalogSyntaxKind::Type, definition_id: &option.definition_id,
        registration_id: &option.registration_id, pattern_index: None, pattern_source: None,
        tags: None, mark: None, dynamic_handler: None,
    }
}

pub(super) fn verify(before: &[u8], after: &[u8], catalog: Arc<Catalog>) {
    let make_host = |bytes: &[u8]| ParserHost::new(bytes, HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)), ..Default::default()
    }).unwrap();
    let baseline = make_host(before);
    let mut candidate = make_host(after);
    let original = &baseline.components[0].manifest;
    let mut normalized = candidate.components[0].manifest.clone();
    let class_info = normalized.registered_syntax_handlers.iter_mut()
        .find(|handler|handler.handler_id == "core.type.class-info").unwrap();
    assert!(class_info.context_requirements.is_empty());
    class_info.context_requirements.push(REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned());
    assert_eq!(format!("{original:?}"),format!("{normalized:?}"),
        "only the Wasm ClassInfo context declaration may change");
    let options = all_expression_type_options(Some(&catalog));
    let class_info = options.iter().find(|option|option.class_name == "ch.njol.skript.classes.ClassInfo").unwrap();
    let entity_type = options.iter().find(|option|option.class_name == "ch.njol.skript.entity.EntityType").unwrap();
    assert!(registered_handler_requires_context(&baseline.components,identity(class_info),REGISTERED_CONTEXT_ALL_TYPE_OPTIONS));
    assert!(!registered_handler_requires_context(&candidate.components,identity(class_info),REGISTERED_CONTEXT_ALL_TYPE_OPTIONS));
    assert!(registered_handler_requires_context(&candidate.components,identity(entity_type),REGISTERED_CONTEXT_ALL_TYPE_OPTIONS));
    // A second matching provider's requirement must still win the generic union.
    let component = &mut candidate.components[0];
    let mut handler = component.manifest.registered_syntax_handlers.iter()
        .find(|handler|handler.handler_id == "core.type.class-info").unwrap().clone();
    let mut binding = component.registered_handler_bindings.iter()
        .find(|binding|binding.handler_id == handler.handler_id).unwrap().clone();
    handler.handler_id = "fixture.additional-type-provider".to_owned();
    binding.handler_id = handler.handler_id.clone();
    handler.context_requirements.push(REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned());
    component.manifest.registered_syntax_handlers.push(handler);
    component.registered_handler_bindings.push(binding);
    assert!(registered_handler_requires_context(&candidate.components,identity(class_info),REGISTERED_CONTEXT_ALL_TYPE_OPTIONS));
    candidate.components[0].disabled = true;
    assert!(!registered_handler_requires_context(&candidate.components,identity(class_info),REGISTERED_CONTEXT_ALL_TYPE_OPTIONS));
    candidate.components[0].disabled = false;
    candidate.components[0].unloaded = true;
    assert!(!registered_handler_requires_context(&candidate.components,identity(class_info),REGISTERED_CONTEXT_ALL_TYPE_OPTIONS));
}
