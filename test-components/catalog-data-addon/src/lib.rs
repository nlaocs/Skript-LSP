#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component for the complete SSG Catalog Data import contract.
//!
//! The host integration test selects a document payload to make this guest call
//! the same import surface a real addon would use. Assertions stay in the guest
//! so the test proves that values crossed the actual Component ABI.
#![allow(missing_docs)] // `wit_bindgen` generates the exported guest API.

use core::sync::atomic::{AtomicBool, Ordering};

wit_bindgen::generate!({
    path: "../../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::{
    catalog_data::{self, CatalogError, CatalogErrorKind, TypeRelation},
    types::{
        AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput,
        CapabilityRequirement, ComponentManifest, Diagnostic, DiagnosticSeverity, HookDecision,
        HookEffects, HookInvocation, HookMode, HookOutput, HookPayload, HookPhase, HookSelector,
        HookSubscription, HookTarget, HostProfile, MappedSpan, OriginKind, SourceOrigin,
        TextMacroInput, TextMacroOutput, TextRange, TreeMacroInput, TreeMacroOutput,
    },
};
use parser_wasm::{ABI_VERSION, CAPABILITY_CATALOG_DATA, CAPABILITY_HOOKS};

const COMPONENT_ID: &str = "test.catalog-data-addon";
const SUBSCRIPTION_ID: &str = "catalog-data.integration";
const EXPECTED_EFFECTS: &[u8] = br#"[
  {
    "registrationId": "duplicate-registration",
    "definitionId": "duplicate-definition",
    "futureField": {"enabled": true},
    "label": "first"
  },
  {
    "registrationId": "duplicate-registration",
    "definitionId": "duplicate-definition",
    "futureField": {"enabled": false},
    "label": "second"
  }
]"#;
const EXPECTED_FIRST_RECORD: &[u8] = br#"{"definitionId":"duplicate-definition","futureField":{"enabled":true},"label":"first","registrationId":"duplicate-registration"}"#;

static CATALOG_CAPABILITY_ADVERTISED: AtomicBool = AtomicBool::new(false);

struct CatalogDataAddon;

fn empty_selector() -> HookSelector {
    HookSelector {
        pattern_index: None,
        pattern_source: None,
        mark: None,
        tags: Vec::new(),
        captures: Vec::new(),
        return_type: None,
        multiplicity: None,
        metadata: Vec::new(),
    }
}

impl addon::Guest for CatalogDataAddon {
    fn manifest() -> ComponentManifest {
        ComponentManifest {
            component_id: COMPONENT_ID.to_owned(),
            component_version: env!("CARGO_PKG_VERSION").to_owned(),
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![
                CapabilityRequirement {
                    id: CAPABILITY_HOOKS.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_CATALOG_DATA.to_owned(),
                    minimum_version: 1,
                    required: false,
                },
            ],
            subscriptions: vec![HookSubscription {
                id: SUBSCRIPTION_ID.to_owned(),
                target: HookTarget::ParseStage,
                phase: HookPhase::Document,
                priority: 0,
                mode: HookMode::Observe,
                capability_id: CAPABILITY_HOOKS.to_owned(),
                selector: empty_selector(),
            }],
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        }
    }

    fn initialize(
        profile: HostProfile,
    ) -> Result<(), nlaocs::skript_parser_addon::types::CompatibilityError> {
        CATALOG_CAPABILITY_ADVERTISED.store(
            profile
                .capabilities
                .iter()
                .any(|capability| capability.id == CAPABILITY_CATALOG_DATA),
            Ordering::Relaxed,
        );
        Ok(())
    }
}

impl hooks::Guest for CatalogDataAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let HookPayload::Document(document) = input.payload else {
            return Err(addon_error(
                "catalog fixture received a non-document payload",
            ));
        };
        if input.context.subscription_id != SUBSCRIPTION_ID {
            return Err(addon_error(
                "catalog fixture received an unknown subscription",
            ));
        }

        let message = match document.text.as_str() {
            "catalog-success" => run_success_checks(),
            "catalog-full-snapshot" => run_full_snapshot_checks(),
            "catalog-quota" => run_quota_checks(),
            "catalog-reconstruct" => run_reconstruction_checks(),
            "catalog-unavailable" => run_unavailable_checks(),
            "catalog-profile" => Ok(format!(
                "catalog capability advertised: {}",
                CATALOG_CAPABILITY_ADVERTISED.load(Ordering::Relaxed)
            )),
            other => Err(format!("unknown catalog fixture mode: {other}")),
        }
        .map_err(|message| addon_error(message))?;

        Ok(report("catalog-data.fixture.passed", message))
    }
}

fn run_full_snapshot_checks() -> Result<String, String> {
    const EXPECTED: &[&str] = &[
        "Aliases.json",
        "ClassHierarchy.json",
        "Comparators.json",
        "Conditions.json",
        "Converters.json",
        "Differences.json",
        "Effects.json",
        "EventValues.json",
        "Events.json",
        "Expressions.json",
        "Functions.json",
        "Manifest.json",
        "Operations.json",
        "Operators.json",
        "PluralRules.json",
        "Properties.json",
        "Sections.json",
        "Structures.json",
        "Types.json",
    ];

    let mut offset = 0;
    let mut names = Vec::new();
    loop {
        let page = catalog_data::documents(offset, 8).map_err(catalog_error)?;
        for document in page.items {
            catalog_data::read_document(&document.name, 0, 1)
                .map_err(catalog_error)?
                .ok_or_else(|| format!("{} was listed but not readable", document.name))?;
            names.push(document.name);
        }
        let Some(next) = page.next_offset else {
            break;
        };
        offset = next;
    }
    if names != EXPECTED {
        return Err(format!("unexpected real snapshot inventory: {names:?}"));
    }
    Ok("all 19 real SSG documents are reachable".to_owned())
}

impl text_macro::Guest for CatalogDataAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(addon_error(
            "catalog fixture does not implement text macros",
        ))
    }
}

impl tree_macro::Guest for CatalogDataAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(addon_error(
            "catalog fixture does not implement tree macros",
        ))
    }
}

impl ast_macro::Guest for CatalogDataAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(addon_error("catalog fixture does not implement AST macros"))
    }
}

fn run_success_checks() -> Result<String, String> {
    let source = catalog_data::source()
        .map_err(catalog_error)?
        .ok_or_else(|| "source metadata was absent".to_owned())?;
    if source.format != "ssg"
        || source.schema_version != 3
        || source.snapshot_id != "catalog-test"
        || source.source_digest.len() != 64
    {
        return Err(format!("unexpected source metadata: {source:?}"));
    }

    let first_documents = catalog_data::documents(0, 1).map_err(catalog_error)?;
    if first_documents.items.len() != 1 || first_documents.next_offset != Some(1) {
        return Err(format!(
            "document pagination did not return the first page: {first_documents:?}"
        ));
    }
    let second_documents = catalog_data::documents(1, 1).map_err(catalog_error)?;
    if second_documents.items.len() != 1 || second_documents.next_offset.is_some() {
        return Err(format!(
            "document pagination did not return the final page: {second_documents:?}"
        ));
    }

    let document = catalog_data::read_document("Effects.json", 0, 4096)
        .map_err(catalog_error)?
        .ok_or_else(|| "Effects.json was not readable".to_owned())?;
    let document_text = String::from_utf8(document.bytes.clone())
        .map_err(|error| format!("document chunk was not UTF-8 JSON: {error}"))?;
    if document.offset != 0
        || document.total_length as usize != document.bytes.len()
        || !document_text.contains("futureField")
    {
        return Err("the full document chunk did not retain its unknown field".to_owned());
    }

    let registration_first =
        catalog_data::records_by_registration_id("duplicate-registration", 0, 1)
            .map_err(catalog_error)?;
    let registration_second =
        catalog_data::records_by_registration_id("duplicate-registration", 1, 1)
            .map_err(catalog_error)?;
    if registration_first.items.len() != 1
        || registration_first.next_offset != Some(1)
        || registration_second.items.len() != 1
        || registration_second.next_offset.is_some()
    {
        return Err("duplicate registration IDs were not paginated".to_owned());
    }

    let definition_page = catalog_data::records_by_definition_id("duplicate-definition", 0, 10)
        .map_err(catalog_error)?;
    if definition_page.items.len() != 2 || definition_page.next_offset.is_some() {
        return Err("duplicate definition IDs were not retained".to_owned());
    }

    let first_record = registration_first
        .items
        .first()
        .ok_or_else(|| "registration page unexpectedly had no record".to_owned())?;
    if first_record.source_digest != source.source_digest {
        return Err("record reference was not bound to the retained source".to_owned());
    }
    let stale = catalog_data::read_record(
        &first_record.source_digest,
        "different-snapshot",
        &first_record.document,
        first_record.index,
        0,
        1,
    )
    .expect_err("a record reference from another snapshot must be rejected");
    if stale.kind != CatalogErrorKind::InvalidInput {
        return Err(format!("unexpected stale record error: {stale:?}"));
    }
    let stale_source = catalog_data::read_record(
        "different-source",
        &first_record.snapshot_id,
        &first_record.document,
        first_record.index,
        0,
        1,
    )
    .expect_err("a record reference from another retained source must be rejected");
    if stale_source.kind != CatalogErrorKind::InvalidInput {
        return Err(format!(
            "unexpected stale source record error: {stale_source:?}"
        ));
    }
    let record = catalog_data::read_record(
        &first_record.source_digest,
        &first_record.snapshot_id,
        &first_record.document,
        first_record.index,
        0,
        4096,
    )
    .map_err(catalog_error)?
    .ok_or_else(|| "indexed record was not readable".to_owned())?;
    let record_text = String::from_utf8(record.bytes)
        .map_err(|error| format!("record chunk was not UTF-8 JSON: {error}"))?;
    if !record_text.contains("futureField") {
        return Err("indexed record lost its unknown field".to_owned());
    }

    let known = catalog_data::class_known("org.bukkit.entity.Player").map_err(catalog_error)?;
    let unknown = catalog_data::class_known("test.Missing").map_err(catalog_error)?;
    let assignable =
        catalog_data::is_class_assignable("org.bukkit.entity.Player", "org.bukkit.entity.Entity")
            .map_err(catalog_error)?;
    let convertible = catalog_data::can_convert("java.lang.Number", "java.lang.Integer")
        .map_err(catalog_error)?;
    let unknown_relation =
        catalog_data::can_convert("test.Missing", "java.lang.Integer").map_err(catalog_error)?;
    if !known
        || unknown
        || assignable != TypeRelation::Compatible
        || convertible != TypeRelation::Compatible
        || unknown_relation != TypeRelation::Unknown
    {
        return Err(format!(
            "catalog relations were wrong: known={known}, unknown={unknown}, assignable={assignable:?}, convertible={convertible:?}, unknown_relation={unknown_relation:?}"
        ));
    }

    Ok(
        "source, documents, chunks, duplicate IDs, unknown fields, and type relations verified"
            .to_owned(),
    )
}

fn run_quota_checks() -> Result<String, String> {
    let error =
        catalog_data::documents(0, 1).expect_err("document page should exceed the fixture quota");
    if error.kind != CatalogErrorKind::ResponseTooLarge {
        return Err(format!("unexpected page quota error: {error:?}"));
    }
    let chunk = catalog_data::read_document("Effects.json", 0, 4096)
        .map_err(catalog_error)?
        .ok_or_else(|| "quota fixture could not read its document".to_owned())?;
    if chunk.bytes.len() > 8 || chunk.total_length <= chunk.bytes.len() as u64 {
        return Err(format!(
            "chunk quota was not enforced as expected: {chunk:?}"
        ));
    }
    Ok("page quota rejection and bounded document chunk verified".to_owned())
}

fn run_reconstruction_checks() -> Result<String, String> {
    let page = catalog_data::documents(0, 1).map_err(catalog_error)?;
    if page.items.len() != 1 || page.next_offset != Some(1) {
        return Err(format!(
            "descriptor page should fit the response quota: {page:?}"
        ));
    }

    let (document, document_chunks) = read_document_chunks("Effects.json", 64)?;
    if document_chunks < 2 {
        return Err("read-document unexpectedly fit in a single 64-byte chunk".to_owned());
    }
    if !document
        .windows(b"futureField".len())
        .any(|window| window == b"futureField")
    {
        return Err("read-document reconstruction lost the unknown field".to_owned());
    }
    if document.as_slice() != EXPECTED_EFFECTS {
        return Err(
            "read-document chunks did not reconstruct the original document bytes".to_owned(),
        );
    }

    let records = catalog_data::records_by_registration_id("duplicate-registration", 0, 1)
        .map_err(catalog_error)?;
    let record = records
        .items
        .first()
        .ok_or_else(|| "record page was empty during chunk reconstruction".to_owned())?;
    let (record_bytes, record_chunks) = read_record_chunks(
        &record.source_digest,
        &record.snapshot_id,
        &record.document,
        record.index,
        64,
    )?;
    if record_chunks < 2 {
        return Err("read-record unexpectedly fit in a single 64-byte chunk".to_owned());
    }
    if record_bytes.as_slice() != EXPECTED_FIRST_RECORD {
        return Err("read-record chunks did not reconstruct the indexed record bytes".to_owned());
    }

    Ok("descriptor paging and 64-byte document/record reconstruction verified".to_owned())
}

fn read_document_chunks(name: &str, max_bytes: u32) -> Result<(Vec<u8>, usize), String> {
    collect_chunks(|offset| catalog_data::read_document(name, offset, max_bytes))
}

fn read_record_chunks(
    source_digest: &str,
    snapshot_id: &str,
    document: &str,
    index: u64,
    max_bytes: u32,
) -> Result<(Vec<u8>, usize), String> {
    collect_chunks(|offset| {
        catalog_data::read_record(
            source_digest,
            snapshot_id,
            document,
            index,
            offset,
            max_bytes,
        )
    })
}

fn collect_chunks<F>(mut read: F) -> Result<(Vec<u8>, usize), String>
where
    F: FnMut(u64) -> Result<Option<catalog_data::CatalogChunk>, CatalogError>,
{
    let mut offset = 0_u64;
    let mut total_length = None;
    let mut chunk_count = 0;
    let mut bytes = Vec::new();
    loop {
        let chunk = read(offset).map_err(catalog_error)?.ok_or_else(|| {
            format!("catalog source ended before the {offset}-byte offset was readable")
        })?;
        chunk_count += 1;
        if chunk.offset != offset {
            return Err(format!(
                "catalog chunk returned offset {}, expected {offset}",
                chunk.offset
            ));
        }
        if let Some(expected) = total_length {
            if expected != chunk.total_length {
                return Err("catalog chunk total length changed during reconstruction".to_owned());
            }
        } else {
            total_length = Some(chunk.total_length);
        }
        if chunk.bytes.is_empty() && offset < chunk.total_length {
            return Err("catalog chunk made no progress before its declared end".to_owned());
        }
        bytes.extend_from_slice(&chunk.bytes);
        offset = offset
            .checked_add(chunk.bytes.len() as u64)
            .ok_or_else(|| "catalog chunk offset overflowed".to_owned())?;
        if offset == chunk.total_length {
            break;
        }
        if offset > chunk.total_length {
            return Err("catalog chunks exceeded their declared total length".to_owned());
        }
    }
    if bytes.len() as u64 != total_length.unwrap_or(0) {
        return Err("catalog chunks did not reconstruct their declared length".to_owned());
    }
    Ok((bytes, chunk_count))
}

fn run_unavailable_checks() -> Result<String, String> {
    let error = catalog_data::source().expect_err("source access should fail without a Catalog");
    if error.kind != CatalogErrorKind::Unavailable {
        return Err(format!("unexpected unavailable error: {error:?}"));
    }
    Ok("source-unavailable error verified".to_owned())
}

fn catalog_error(error: CatalogError) -> String {
    format!(
        "catalog import failed ({:?}): {}",
        error.kind, error.message
    )
}

fn report(code: &str, message: String) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: None,
        effects: HookEffects {
            diagnostics: vec![Diagnostic {
                code: code.to_owned(),
                message,
                severity: DiagnosticSeverity::Information,
                span: empty_span(),
                related: Vec::new(),
            }],
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

fn empty_span() -> MappedSpan {
    MappedSpan {
        virtual_range: TextRange { start: 0, end: 0 },
        origins: vec![SourceOrigin {
            original_range: TextRange { start: 0, end: 0 },
            kind: OriginKind::Exact,
            expansion: None,
        }],
    }
}

fn addon_error(message: impl Into<String>) -> AddonError {
    AddonError {
        kind: AddonErrorKind::InvalidPayload,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

#[cfg(target_arch = "wasm32")]
export!(CatalogDataAddon);
