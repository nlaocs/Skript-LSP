use super::*;
use wasm_encoder::{
    Alias, BlockType, CanonicalFunctionSection, CodeSection, Component as EncodedComponent,
    ComponentAliasSection, ComponentExportKind, ComponentExportSection, ComponentTypeSection,
    ComponentValType, ExportKind, ExportSection, Function, FunctionSection, InstanceSection,
    Instruction, Module as EncodedModule, ModuleSection, TypeSection,
};

fn cancellable_store(engine: &Engine, clock: &Arc<EpochClock>) -> Store<StoreData> {
    let config = HostConfig::default();
    let mut store = create_store(engine, &config, build_type_user_input_matchers(None));
    store.data_mut().epoch_clock = Some(Arc::clone(clock));
    store.data_mut().interrupt_safe = true;
    store.data_mut().cancellation = Some(DocumentCancellationToken::new());
    store
}

fn looping_component(engine: &Engine) -> Component {
    let mut types = TypeSection::new();
    types.ty().function([], []);
    let mut functions = FunctionSection::new();
    functions.function(0);
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 0);
    exports.export("healthy", ExportKind::Func, 1);
    let mut run = Function::new([]);
    run.instruction(&Instruction::Loop(BlockType::Empty));
    run.instruction(&Instruction::Br(0));
    run.instruction(&Instruction::End);
    run.instruction(&Instruction::End);
    let mut healthy = Function::new([]);
    healthy.instruction(&Instruction::End);
    let mut code = CodeSection::new();
    code.function(&run);
    code.function(&healthy);
    let mut module = EncodedModule::new();
    module.section(&types);
    module.section(&functions);
    module.section(&exports);
    module.section(&code);

    let mut component = EncodedComponent::new();
    component.section(&ModuleSection(&module));
    let mut instances = InstanceSection::new();
    instances.instantiate(0, std::iter::empty::<(&str, wasm_encoder::ModuleArg)>());
    component.section(&instances);
    let mut aliases = ComponentAliasSection::new();
    for name in ["run", "healthy"] {
        aliases.alias(Alias::CoreInstanceExport {
            instance: 0,
            kind: ExportKind::Func,
            name,
        });
    }
    component.section(&aliases);
    let mut types = ComponentTypeSection::new();
    types
        .function()
        .params(std::iter::empty::<(&str, ComponentValType)>())
        .result(None);
    component.section(&types);
    let mut canonical = CanonicalFunctionSection::new();
    canonical.lift(0, 0, []);
    canonical.lift(1, 0, []);
    component.section(&canonical);
    let mut exports = ComponentExportSection::new();
    exports.export("run", ComponentExportKind::Func, 0, None);
    exports.export("healthy", ComponentExportKind::Func, 1, None);
    component.section(&exports);
    Component::new(engine, component.finish()).unwrap()
}

#[test]
fn project_stability_requires_exact_supported_opt_in_and_no_disabled_component() {
    let mut host = ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .unwrap();
    let manifest = &mut host.components[0].manifest;
    manifest
        .capabilities
        .retain(|requirement| requirement.id != crate::CAPABILITY_PROJECT_STABLE_SEMANTICS);
    assert!(!host.project_semantics_are_stable());
    host.components[0].manifest.capabilities.push(
        crate::bindings::nlaocs::skript_parser_addon::types::CapabilityRequirement {
            id: crate::CAPABILITY_PROJECT_STABLE_SEMANTICS.into(),
            minimum_version: 1,
            required: false,
        },
    );
    assert!(host.project_semantics_are_stable());
    host.components[0]
        .manifest
        .capabilities
        .last_mut()
        .unwrap()
        .minimum_version = 2;
    assert!(!host.project_semantics_are_stable());
    host.components[0]
        .manifest
        .capabilities
        .last_mut()
        .unwrap()
        .minimum_version = 1;
    host.components[0].disabled = true;
    assert!(!host.project_semantics_are_stable());
}

#[test]
fn epoch_safety_promise_is_separate_from_project_stability_and_future_versions() {
    use crate::bindings::nlaocs::skript_parser_addon::types::CapabilityRequirement;
    let project = CapabilityRequirement {
        id: crate::CAPABILITY_PROJECT_STABLE_SEMANTICS.into(),
        minimum_version: 1,
        required: false,
    };
    assert!(!component_promises(
        &[],
        crate::CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE
    ));
    assert!(!component_promises(
        &[project],
        crate::CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE
    ));
    let mut safe = CapabilityRequirement {
        id: crate::CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE.into(),
        minimum_version: 2,
        required: false,
    };
    assert!(!component_promises(
        &[safe.clone()],
        crate::CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE
    ));
    safe.minimum_version = 1;
    assert!(component_promises(
        &[safe],
        crate::CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE
    ));
}

fn engine() -> Engine {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    config.epoch_interruption(true);
    Engine::new(&config).unwrap()
}

#[test]
fn running_component_cancellation_is_typed_and_requires_a_fresh_store() {
    let engine = engine();
    let ticker = EpochTicker::start(engine.clone(), Duration::from_millis(1)).unwrap();
    let component = looping_component(&engine);
    let mut store = cancellable_store(&engine, &ticker.clock);
    let token = store.data().cancellation.as_ref().unwrap().clone();
    prepare_store(
        &mut store,
        1_000_000_000,
        CORE_LIBRARY_DEADLINE_TICKS,
        "test",
        "instantiate",
    )
    .unwrap();
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &component)
        .unwrap();
    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .unwrap();
    let healthy = instance
        .get_typed_func::<(), ()>(&mut store, "healthy")
        .unwrap();
    let clock = Arc::clone(&ticker.clock);
    let target_tick = clock.ticks.load(Ordering::Acquire) + 3;
    let canceller = thread::spawn(move || {
        while clock.ticks.load(Ordering::Acquire) < target_tick {
            thread::yield_now();
        }
        token.cancel();
    });
    let error = run
        .call(&mut store, ())
        .expect_err("running component must stop on cancellation");
    canceller.join().unwrap();
    let classified = classify_wasmtime_error("test".into(), "hook", error);
    assert!(
        matches!(classified, HostError::Cancelled(_)),
        "{classified:?}"
    );
    assert!(!classified.disables_component());
    assert!(store.data().epoch_cancelled);
    let error = healthy
        .call(&mut store, ())
        .expect_err("canonical trap poisons the old Store");
    assert!(matches!(
        error.downcast_ref::<Trap>(),
        Some(Trap::CannotEnterComponent)
    ));
    drop(store);
    let mut store = cancellable_store(&engine, &ticker.clock);
    prepare_store(&mut store, 10_000, 10, "test", "instantiate").unwrap();
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &component)
        .unwrap();
    instance
        .get_typed_func::<(), ()>(&mut store, "healthy")
        .unwrap()
        .call(&mut store, ())
        .expect("a fresh Store has healthy canonical state");
}

#[test]
fn delayed_epoch_callback_does_not_extend_the_original_addon_deadline() {
    let engine = engine();
    let clock = Arc::new(EpochClock::default());
    let component = looping_component(&engine);
    let mut store = cancellable_store(&engine, &clock);
    prepare_store(&mut store, 1_000_000, 3, "test", "instantiate").unwrap();
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &component)
        .unwrap();
    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .unwrap();
    prepare_store(&mut store, 1_000_000, 3, "test", "hook").unwrap();
    let deadline = store.data().call_deadline;
    for _ in 0..10 {
        clock.advance(&engine);
    }
    let error = run
        .call(&mut store, ())
        .expect_err("expired absolute deadline must trap immediately");
    assert_eq!(store.data().call_deadline, deadline);
    // A cancellation racing after the actual timeout cannot rewrite its cause.
    store.data().cancellation.as_ref().unwrap().cancel();
    let classified = classify_wasmtime_error("test".into(), "hook", error);
    assert!(
        matches!(classified, HostError::Timeout { .. }),
        "{classified:?}"
    );
    assert!(classified.disables_component());
}

#[test]
fn cancellation_polling_preserves_the_guest_fuel_quota() {
    let engine = engine();
    let clock = Arc::new(EpochClock::default());
    let component = looping_component(&engine);
    let mut store = cancellable_store(&engine, &clock);
    prepare_store(
        &mut store,
        10_000,
        CORE_LIBRARY_DEADLINE_TICKS,
        "test",
        "instantiate",
    )
    .unwrap();
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &component)
        .unwrap();
    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .unwrap();
    prepare_store(&mut store, 100, CORE_LIBRARY_DEADLINE_TICKS, "test", "hook").unwrap();
    let error = run
        .call(&mut store, ())
        .expect_err("fuel remains bounded with cancellation polling");
    let classified = classify_wasmtime_error("test".into(), "hook", error);
    assert!(
        matches!(classified, HostError::FuelExhausted { .. }),
        "{classified:?}"
    );
}

#[test]
fn a_guest_without_the_safety_promise_defers_cancellation_until_its_lock_is_released() {
    use wasm_encoder::{ConstExpr, EntityType, GlobalSection, GlobalType, ImportSection, ValType};
    let engine = engine();
    let clock = Arc::new(EpochClock::default());
    let mut types = TypeSection::new();
    types.ty().function([], []);
    types.ty().function([], [ValType::I32]);
    let mut imports = ImportSection::new();
    imports.import("test", "cancel", EntityType::Function(0));
    let mut functions = FunctionSection::new();
    functions.function(0);
    functions.function(1);
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    exports.export("lock-state", ExportKind::Func, 2);
    let mut run = Function::new([]);
    run.instruction(&Instruction::I32Const(1));
    run.instruction(&Instruction::GlobalSet(0));
    run.instruction(&Instruction::Call(0));
    run.instruction(&Instruction::I32Const(0));
    run.instruction(&Instruction::GlobalSet(0));
    run.instruction(&Instruction::End);
    let mut healthy = Function::new([]);
    healthy.instruction(&Instruction::GlobalGet(0));
    healthy.instruction(&Instruction::End);
    let mut code = CodeSection::new();
    code.function(&run);
    code.function(&healthy);
    let mut module = EncodedModule::new();
    module.section(&types);
    module.section(&imports);
    module.section(&functions);
    module.section(&globals);
    module.section(&exports);
    module.section(&code);
    let module = wasmtime::Module::new(&engine, module.finish()).unwrap();
    let mut store = cancellable_store(&engine, &clock);
    store.data_mut().interrupt_safe = false;
    let mut linker = wasmtime::Linker::new(&engine);
    let callback_clock = Arc::clone(&clock);
    linker
        .func_wrap(
            "test",
            "cancel",
            move |caller: wasmtime::Caller<'_, StoreData>| {
                caller.data().cancellation.as_ref().unwrap().cancel();
                callback_clock.advance(caller.engine());
            },
        )
        .unwrap();
    prepare_store(&mut store, 10_000, 10, "test", "instantiate").unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .unwrap();
    let healthy = instance
        .get_typed_func::<(), i32>(&mut store, "lock-state")
        .unwrap();
    run.call(&mut store, ())
        .expect("unsafe guest must return normally and release its live lock");
    assert!(store.data().cancellation.as_ref().unwrap().is_cancelled());
    store.data_mut().cancellation = Some(DocumentCancellationToken::new());
    prepare_store(&mut store, 10_000, 10, "test", "hook").unwrap();
    assert_eq!(healthy.call(&mut store, ()).unwrap(), 0);
}

const TEXT_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/text-macro-addon.wasm"
));
const TEXT_COMPONENT: &str = "nlaocs.test.text-macro";
const TEXT_NAMESPACE: &str = "macro-state";
const TEST_PROJECT: &str = "file:///restart-project";
const TEST_DOCUMENT: &str = "file:///restart-project/test.sk";

fn restart_test_host(language: Option<&str>) -> Result<ParserHost, HostError> {
    let mut host = ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            fuel_per_call: 1_000_000_000,
            call_timeout: Duration::from_secs(2),
            epoch_tick: Duration::from_millis(1),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                language: language.map(str::to_owned),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )?;
    host.load_addon(TEXT_ADDON)?;
    Ok(host)
}

fn restart_text_request(revision: u64, text: &str) -> TextMacroRequest {
    TextMacroRequest {
        context: InvocationContext {
            invocation_id: revision,
            subscription_id: String::new(),
            document_id: TEST_DOCUMENT.into(),
            document_revision: revision,
            expansion: None,
            syntax_context: 0,
        },
        source: MappedSource::identity(text),
    }
}

fn text_component_index(host: &ParserHost) -> usize {
    host.components
        .iter()
        .position(|entry| entry.manifest.component_id == TEXT_COMPONENT)
        .unwrap()
}

#[test]
fn actual_text_guest_cancellation_rolls_back_and_recovers_without_losing_host_state() {
    let mut host = restart_test_host(None).unwrap();
    let index = text_component_index(&host);
    let seed = host.begin_parse(TEST_PROJECT, TEST_DOCUMENT, 1).unwrap();
    let mut invocation = seed.begin_invocation(TEXT_COMPONENT).unwrap();
    let sentinel = StateValue {
        schema_id: "nlaocs.test.text-macro-state".into(),
        encoding: StateEncoding::Json,
        bytes: b"retained".to_vec(),
    };
    invocation
        .put(
            StateScope::Project,
            NamespaceVisibility::Private,
            TEXT_NAMESPACE,
            "sentinel",
            sentinel.clone(),
        )
        .unwrap();
    invocation.commit().unwrap();
    seed.commit().unwrap();

    let token = DocumentCancellationToken::new();
    host.native_cancellation = Some(token.clone());
    host.components[index]
        .store
        .data_mut()
        .cancel_token_after_state_put = Some(("text.first".into(), token.clone()));
    let cancelled = host.begin_parse(TEST_PROJECT, TEST_DOCUMENT, 2).unwrap();
    let error = host
        .expand_text_in_parse(&cancelled, restart_text_request(2, "epoch-cancel"))
        .unwrap_err();
    assert!(matches!(error, HostError::Cancelled(_)), "{error:?}");
    assert!(
        token.is_cancelled(),
        "guest reached and completed the real StateStore write"
    );
    assert!(
        host.components[index].store.data().epoch_cancelled,
        "cancellation happened inside the running guest"
    );
    assert!(!host.components[index].disabled);
    let mut check = cancelled.begin_invocation(TEXT_COMPONENT).unwrap();
    assert!(
        check
            .get(
                StateScope::Parse,
                NamespaceVisibility::Private,
                TEXT_NAMESPACE,
                "text.first"
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        check
            .get(
                StateScope::Project,
                NamespaceVisibility::Private,
                TEXT_NAMESPACE,
                "sentinel"
            )
            .unwrap(),
        Some(sentinel.clone())
    );
    check.rollback();
    cancelled.cancel().unwrap();

    host.native_cancellation = Some(DocumentCancellationToken::new());
    let healthy = host.begin_parse(TEST_PROJECT, TEST_DOCUMENT, 3).unwrap();
    let result = host
        .expand_text_in_parse(&healthy, restart_text_request(3, "healthy"))
        .unwrap();
    assert!(result.failures.is_empty());
    assert_eq!(result.calls.len(), 2);
    assert!(result.calls.iter().all(|call| call.accepted));
    assert!(!host.components[index].disabled);
    assert!(!host.components[index].restart_pending);
    assert!(!host.components[index].store.data().epoch_cancelled);
    let mut check = healthy.begin_invocation(TEXT_COMPONENT).unwrap();
    assert_eq!(
        check
            .get(
                StateScope::Project,
                NamespaceVisibility::Private,
                TEXT_NAMESPACE,
                "sentinel"
            )
            .unwrap(),
        Some(sentinel)
    );
    check.rollback();
    healthy.commit().unwrap();
}

#[test]
fn restarted_manifest_or_handler_mismatch_disables_without_running_the_old_binding() {
    for tamper_handlers in [false, true] {
        let mut host = restart_test_host(None).unwrap();
        let index = text_component_index(&host);
        let certificate = host.completion_type_certificate();
        host.components[index].restart_pending = true;
        if tamper_handlers {
            host.components[index]
                .registered_handler_bindings
                .push(WitRegisteredHandlerBinding {
                    handler_id: "forged-retained-handler".into(),
                    definition_ids: vec![],
                    registration_ids: vec![],
                });
        } else {
            host.components[index]
                .manifest
                .component_version
                .push_str("-changed");
        }
        let error = host.ensure_component_ready(index).unwrap_err();
        assert!(
            matches!(
                error,
                HostError::Runtime {
                    operation: "restart after cancellation",
                    ..
                }
            ),
            "{error:?}"
        );
        assert!(!certificate.is_current());
        assert!(host.components[index].disabled);
        assert!(host.components[index].restart_pending);
        assert!(host.components[index].store.data().invocation.is_none());
        assert!(host.ensure_component_ready(index).is_err());
    }
}

#[test]
fn restart_safe_initialization_rejects_ignored_state_and_dynamic_calls() {
    for language in [
        "test.restart-init-state-read",
        "test.restart-init-state-write",
        "test.restart-init-dynamic",
    ] {
        let error = restart_test_host(Some(language))
            .err()
            .expect("initializer call attempt must reject opt-in");
        assert!(
            matches!(error, HostError::InitializationRejected { .. }),
            "{language}: {error:?}"
        );
    }
}

#[test]
fn cancellation_during_restart_initialization_stays_pending_then_recovers() {
    let mut host = restart_test_host(None).unwrap();
    let index = text_component_index(&host);
    let certificate = host.completion_type_certificate();
    let token = DocumentCancellationToken::new();
    host.native_cancellation = Some(token.clone());
    host.components[index].restart_pending = true;
    host.components[index].cancel_during_restart_initialize = Some(token);
    let error = host.ensure_component_ready(index).unwrap_err();
    assert!(matches!(error, HostError::Cancelled(_)), "{error:?}");
    assert!(certificate.is_current());
    assert!(host.components[index].restart_pending);
    assert!(!host.components[index].disabled);
    assert!(host.components[index].store.data().epoch_cancelled);
    host.native_cancellation = Some(DocumentCancellationToken::new());
    host.ensure_component_ready(index).unwrap();
    assert!(certificate.is_current());
    assert!(!host.components[index].restart_pending);
    assert!(!host.components[index].disabled);
    host.expand_text(TEST_PROJECT, restart_text_request(1, "healthy"))
        .unwrap();
}

#[test]
fn restart_quota_fault_is_not_relabelled_as_cancellation() {
    let mut host = restart_test_host(None).unwrap();
    let index = text_component_index(&host);
    let certificate = host.completion_type_certificate();
    host.components[index].restart_pending = true;
    host.config.fuel_per_call = 1;
    let error = host.ensure_component_ready(index).unwrap_err();
    assert!(
        matches!(error, HostError::FuelExhausted { .. }),
        "{error:?}"
    );
    assert!(!certificate.is_current());
    assert!(host.components[index].disabled);
    assert!(host.components[index].restart_pending);
}

#[test]
fn instantiate_cancellation_and_resource_markers_keep_their_typed_causes() {
    let error = classify_component_error(
        "test".into(),
        "restart instantiate",
        skript_parser::ParseCancelled.into(),
    );
    assert!(matches!(error, HostError::Cancelled(_)));
    assert!(!error.disables_component());
}

#[test]
fn isolated_initialization_rejects_ignored_state_attempt_but_does_not_relax_restart() {
    let config = HostConfig::default();
    let runtime = SharedHostRuntime::new(config.epoch_tick).unwrap();
    let mut store = create_store(
        &runtime.engine,
        &config,
        build_type_user_input_matchers(None),
    );
    store.data_mut().initializing = true;
    assert!(store.data_mut().invocation().is_err());
    assert!(store.data().initialization_state_call_attempted);
    assert!(matches!(
        validate_isolated_preparation_initialization(&store, "test.isolated", true),
        Err(HostError::InitializationRejected { .. })
    ));
    assert!(validate_isolated_preparation_initialization(&store, "test.isolated", false).is_ok());
    store.data_mut().initialization_state_call_attempted = false;
    store.data_mut().initialization_host_call_attempted = false;
    assert!(store.data_mut().dynamic_update().is_err());
    assert!(store.data().initialization_host_call_attempted);
    assert!(validate_isolated_preparation_initialization(&store, "test.isolated", true).is_ok());
    assert!(matches!(
        validate_restart_initialization_for(&store, "test.isolated", true),
        Err(HostError::InitializationRejected { .. })
    ));
}
