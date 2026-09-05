use skript_parser::{ExpressionParseContext, SectionCandidate, SectionScopeFrame};

/// One parser-validated Section body inherited by later Effect parses.
#[derive(Debug, Clone)]
pub struct SectionContext {
    /// Normalized header supplied by the caller, without its trailing colon.
    pub input: String,
    /// Parser-owned identity and semantics inherited by statements in the body.
    pub frame: SectionScopeFrame,
    /// Non-fatal diagnostics emitted while selecting the Section.
    pub diagnostics: Vec<SectionContextDiagnostic>,
    /// WASM hook failures retained without discarding an otherwise valid frame.
    pub component_failures: Vec<SectionContextComponentFailure>,
    pub(crate) parser_context: ExpressionParseContext,
}

impl SectionContext {
    pub(crate) fn from_candidate(
        input: String,
        candidate: SectionCandidate,
        diagnostics: Vec<SectionContextDiagnostic>,
        component_failures: Vec<SectionContextComponentFailure>,
    ) -> Result<Self, &'static str> {
        let parser_context = candidate.body_context;
        let frame = parser_context
            .section_stack
            .last()
            .cloned()
            .ok_or("Section parser did not retain an active scope frame")?;
        Ok(Self {
            input,
            frame,
            diagnostics,
            component_failures,
            parser_context,
        })
    }
}

/// Diagnostic emitted while selecting a Section context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionContextDiagnostic {
    /// Stable diagnostic code supplied by the parser or addon.
    pub code: String,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Diagnostic severity name.
    pub severity: String,
}

/// WASM component failure emitted while selecting a Section context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionContextComponentFailure {
    /// Component that failed while the Section candidate was processed.
    pub component_id: String,
    /// Subscription invoked when the failure occurred.
    pub subscription_id: String,
    /// Failure message returned by the host.
    pub message: String,
}

pub(crate) fn normalize_section_header(input: &str) -> Result<String, &'static str> {
    let mut input = input.trim();
    if input.len() >= 2 && input.starts_with('"') && input.ends_with('"') {
        input = &input[1..input.len() - 1];
    }
    input = input.trim();
    if let Some(without_colon) = input.strip_suffix(':') {
        input = without_colon.trim_end();
    }
    if input.is_empty() {
        return Err("Section header is empty");
    }
    if input.contains(['\r', '\n']) {
        return Err("Section header must be one line");
    }
    Ok(input.to_owned())
}
