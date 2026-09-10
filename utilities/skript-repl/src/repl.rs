use crate::{EXIT_FAILURE, EXIT_SUCCESS, OutputFormat, SkriptSession};
use rustyline::{DefaultEditor, error::ReadlineError};
use skript_parser::{MappedSource, RawDiagnosticCode, RawNodeKind, RawTreeOptions, parse_raw_tree};
use std::io::{self, BufRead, Write};

const PRIMARY_PROMPT: &str = "skript> ";
const CONTINUATION_PROMPT: &str = "......> ";

const REPL_HELP: &str = r#"REPL commands:
  :help                 Show this command list
  :reload               Reload the SSG snapshot and parser addons
  :event <header>       Select an Event context (`:` is optional)
  :event off            Clear the Event context
  :events               List registered Events
  :section <header>     Push a Section context (`:` is optional)
  :section pop          Pop the innermost Section context
  :section off|clear    Clear all Section contexts
  :context              Show the active Event and Section contexts
  :json on              Use JSON reports
  :json off             Use human-readable reports
  :submit               Parse the current multiline document
  :cancel               Discard the current multiline document
  :quit, :exit          Exit the REPL

A simple line is parsed immediately as one Effect. A Structure or pasted
multiline source is collected until an empty line or :submit. Ctrl+C discards
only the current draft. Submitted Skript is analyzed but never executed."#;

pub(crate) fn run_stream<R: BufRead, W: Write, E: Write>(
    session: &mut SkriptSession,
    mut format: OutputFormat,
    mut input: R,
    mut output: W,
    mut error: E,
    color: bool,
) -> u8 {
    if write_banner(&mut output).is_err() {
        return EXIT_FAILURE;
    }
    let mut buffer = InputBuffer::default();
    let mut line = String::new();
    loop {
        let prompt = if buffer.is_collecting() {
            CONTINUATION_PROMPT
        } else {
            PRIMARY_PROMPT
        };
        if write!(output, "\n{prompt}")
            .and_then(|_| output.flush())
            .is_err()
        {
            return EXIT_FAILURE;
        }
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => {
                if let Some(source) = buffer.take() {
                    let control = handle_action(
                        InputAction::AnalyzeDocument(source),
                        session,
                        &mut format,
                        &mut output,
                        &mut error,
                        color,
                    );
                    if control == ReplControl::Failure {
                        return EXIT_FAILURE;
                    }
                }
                let _ = writeln!(output);
                return EXIT_SUCCESS;
            }
            Ok(_) => {}
            Err(read_error) if read_error.kind() == io::ErrorKind::Interrupted => {
                let discarded = buffer.cancel();
                let _ = writeln!(
                    output,
                    "^C{}",
                    if discarded {
                        " (multiline input discarded)"
                    } else {
                        ""
                    }
                );
                continue;
            }
            Err(read_error) => {
                let _ = writeln!(error, "error: failed to read REPL input: {read_error}");
                return EXIT_FAILURE;
            }
        }
        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        let action = buffer.accept(line, session.raw_tree_options());
        match handle_action(action, session, &mut format, &mut output, &mut error, color) {
            ReplControl::Continue => {}
            ReplControl::Exit => return EXIT_SUCCESS,
            ReplControl::Failure => return EXIT_FAILURE,
        }
    }
}

pub(crate) fn run_terminal<W: Write, E: Write>(
    session: &mut SkriptSession,
    mut format: OutputFormat,
    mut output: W,
    mut error: E,
    color: bool,
) -> u8 {
    if write_banner(&mut output).is_err() {
        return EXIT_FAILURE;
    }
    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(init_error) => {
            let _ = writeln!(
                error,
                "error: failed to initialize terminal input: {init_error}"
            );
            return EXIT_FAILURE;
        }
    };
    let mut buffer = InputBuffer::default();
    loop {
        let read = if buffer.is_collecting() {
            let indent = buffer.suggested_indent(session.raw_tree_options());
            editor.readline_with_initial(CONTINUATION_PROMPT, (&indent, ""))
        } else {
            editor.readline(PRIMARY_PROMPT)
        };
        match read {
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = editor.add_history_entry(line.as_str());
                }
                let action = buffer.accept(line, session.raw_tree_options());
                match handle_action(action, session, &mut format, &mut output, &mut error, color) {
                    ReplControl::Continue => {}
                    ReplControl::Exit => return EXIT_SUCCESS,
                    ReplControl::Failure => return EXIT_FAILURE,
                }
            }
            Err(ReadlineError::Interrupted) => {
                let discarded = buffer.cancel();
                let _ = writeln!(
                    output,
                    "^C{}",
                    if discarded {
                        " (multiline input discarded)"
                    } else {
                        ""
                    }
                );
            }
            Err(ReadlineError::Eof) => {
                if let Some(source) = buffer.take() {
                    let control = handle_action(
                        InputAction::AnalyzeDocument(source),
                        session,
                        &mut format,
                        &mut output,
                        &mut error,
                        color,
                    );
                    if control == ReplControl::Failure {
                        return EXIT_FAILURE;
                    }
                }
                let _ = writeln!(output);
                return EXIT_SUCCESS;
            }
            Err(read_error) => {
                let _ = writeln!(error, "error: failed to read terminal input: {read_error}");
                return EXIT_FAILURE;
            }
        }
    }
}

fn write_banner(output: &mut dyn Write) -> io::Result<()> {
    writeln!(output, "Skript REPL")?;
    writeln!(output, "Type :help for available commands.")
}

#[derive(Debug, Default)]
struct InputBuffer {
    source: String,
}

impl InputBuffer {
    fn is_collecting(&self) -> bool {
        !self.source.is_empty()
    }

    fn accept(&mut self, input: String, options: RawTreeOptions) -> InputAction {
        if self.is_collecting() {
            if !input.contains(['\r', '\n']) {
                if input.trim().is_empty() && self.has_unclosed_block_comment(options) {
                    self.append(&input);
                    return InputAction::ContinueDocument;
                }
                match input.trim() {
                    ":submit" => {
                        return InputAction::AnalyzeDocument(
                            self.take().expect("collecting buffer is non-empty"),
                        );
                    }
                    ":cancel" => {
                        self.cancel();
                        return InputAction::Cancelled;
                    }
                    ":quit" | ":exit" => {
                        self.cancel();
                        return InputAction::ExitWithDiscard;
                    }
                    _ if input.starts_with(':') => {
                        return InputAction::CommandWhileCollecting(input);
                    }
                    _ if input.trim().is_empty() => {
                        return InputAction::AnalyzeDocument(
                            self.take().expect("collecting buffer is non-empty"),
                        );
                    }
                    _ => {}
                }
            }
            self.append(&input);
            return InputAction::ContinueDocument;
        }

        if input.trim().is_empty() {
            return InputAction::Ignore;
        }
        if !input.contains(['\r', '\n']) && input.starts_with(':') {
            return InputAction::Command(input);
        }
        if starts_document(&input, options) {
            self.append(&input);
            InputAction::ContinueDocument
        } else {
            InputAction::AnalyzeEffect(input)
        }
    }

    fn append(&mut self, input: &str) {
        if input.is_empty() {
            if !self.source.ends_with(['\r', '\n']) {
                self.source.push('\n');
            }
            self.source.push('\n');
            return;
        }
        if !self.source.is_empty() && !self.source.ends_with(['\r', '\n']) {
            self.source.push('\n');
        }
        self.source.push_str(input);
    }

    fn suggested_indent(&self, options: RawTreeOptions) -> String {
        let source = MappedSource::identity(self.source.as_str());
        let tree = parse_raw_tree(&source, options);
        let Some(node) = tree
            .nodes
            .iter()
            .rev()
            .find(|node| !matches!(node.kind, RawNodeKind::Blank | RawNodeKind::Comment))
        else {
            return String::new();
        };
        let mut indent = node.line.indentation.text.clone();
        if node.kind == RawNodeKind::Section {
            indent.push_str(
                tree.indentation
                    .as_ref()
                    .map_or("    ", |indentation| indentation.unit.as_str()),
            );
        }
        indent
    }

    fn has_unclosed_block_comment(&self, options: RawTreeOptions) -> bool {
        let source = MappedSource::identity(self.source.as_str());
        parse_raw_tree(&source, options)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == RawDiagnosticCode::UnclosedBlockComment)
    }

    fn cancel(&mut self) -> bool {
        let discarded = self.is_collecting();
        self.source.clear();
        discarded
    }

    fn take(&mut self) -> Option<String> {
        self.is_collecting()
            .then(|| std::mem::take(&mut self.source))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InputAction {
    Ignore,
    Command(String),
    AnalyzeEffect(String),
    ContinueDocument,
    AnalyzeDocument(String),
    Cancelled,
    CommandWhileCollecting(String),
    ExitWithDiscard,
}

fn starts_document(input: &str, options: RawTreeOptions) -> bool {
    if input.contains(['\r', '\n']) {
        return true;
    }
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, options);
    if tree
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == RawDiagnosticCode::UnclosedBlockComment)
    {
        return true;
    }
    tree.roots
        .first()
        .and_then(|id| tree.get(*id))
        .is_some_and(|node| {
            matches!(
                node.kind,
                RawNodeKind::Section | RawNodeKind::Comment | RawNodeKind::Invalid
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplControl {
    Continue,
    Exit,
    Failure,
}

fn handle_action(
    action: InputAction,
    session: &mut SkriptSession,
    format: &mut OutputFormat,
    output: &mut dyn Write,
    error: &mut dyn Write,
    color: bool,
) -> ReplControl {
    match action {
        InputAction::Ignore | InputAction::ContinueDocument => ReplControl::Continue,
        InputAction::Cancelled => {
            let _ = writeln!(output, "multiline input discarded");
            ReplControl::Continue
        }
        InputAction::CommandWhileCollecting(command) => {
            let _ = writeln!(
                output,
                "REPL command {command:?} is unavailable while collecting source; use :submit or :cancel"
            );
            ReplControl::Continue
        }
        InputAction::ExitWithDiscard => {
            let _ = writeln!(output, "multiline input discarded");
            ReplControl::Exit
        }
        InputAction::Command(command) => repl_command(&command, session, format, output),
        InputAction::AnalyzeEffect(effect) => match session.analyze_repl_effect(&effect) {
            Ok(report) => {
                if let Err(write_error) = report.write_with_color(*format, output, color) {
                    let _ = writeln!(error, "error: failed to write output: {write_error}");
                    ReplControl::Failure
                } else {
                    ReplControl::Continue
                }
            }
            Err(parse_error) => {
                let _ = writeln!(output, "error: {parse_error}");
                ReplControl::Continue
            }
        },
        InputAction::AnalyzeDocument(source) => match session.analyze_document(&source) {
            Ok(report) => {
                if let Err(write_error) = report.write_with_color(*format, output, color) {
                    let _ = writeln!(error, "error: failed to write output: {write_error}");
                    ReplControl::Failure
                } else {
                    ReplControl::Continue
                }
            }
            Err(parse_error) => {
                let _ = writeln!(output, "error: {parse_error}");
                ReplControl::Continue
            }
        },
    }
}

fn repl_command(
    command: &str,
    session: &mut SkriptSession,
    format: &mut OutputFormat,
    output: &mut dyn Write,
) -> ReplControl {
    let command = command.trim();
    if let Some(selector) = event_selector(command) {
        select_event(selector.trim(), session, output);
        return ReplControl::Continue;
    }
    if let Some(selector) = section_selector(command) {
        select_section(selector.trim(), session, output);
        return ReplControl::Continue;
    }
    match command {
        ":quit" | ":exit" => ReplControl::Exit,
        ":help" => {
            let _ = writeln!(output, "{REPL_HELP}");
            ReplControl::Continue
        }
        ":reload" => {
            match session.reload() {
                Ok(()) => {
                    let _ = writeln!(output, "reloaded {}", session.snapshot_path().display());
                }
                Err(error) => {
                    let _ = writeln!(output, "error: {error}");
                }
            }
            ReplControl::Continue
        }
        ":event" | ":section" | ":context" => {
            write_context(session, output);
            ReplControl::Continue
        }
        ":events" => {
            let events = match session.events() {
                Ok(events) => events,
                Err(error) => {
                    let _ = writeln!(output, "error: {error}");
                    return ReplControl::Continue;
                }
            };
            let _ = writeln!(output, "Events ({}):", events.len());
            for event in events {
                let owner = event.addon.as_ref().map_or_else(
                    || event.handler.as_deref().unwrap_or("dynamic"),
                    |addon| addon.name.as_str(),
                );
                let _ = writeln!(
                    output,
                    "  - {} {} [{}]",
                    owner,
                    event.patterns.join(" | "),
                    event.registration_id,
                );
            }
            ReplControl::Continue
        }
        ":json on" => {
            *format = OutputFormat::Json;
            let _ = writeln!(output, "JSON output enabled");
            ReplControl::Continue
        }
        ":json off" => {
            *format = OutputFormat::Human;
            let _ = writeln!(output, "JSON output disabled");
            ReplControl::Continue
        }
        ":json" => {
            let enabled = matches!(format, OutputFormat::Json);
            let _ = writeln!(
                output,
                "JSON output is {}",
                if enabled { "enabled" } else { "disabled" }
            );
            ReplControl::Continue
        }
        ":submit" | ":cancel" => {
            let _ = writeln!(output, "no multiline input is active");
            ReplControl::Continue
        }
        unknown => {
            let _ = writeln!(
                output,
                "unknown REPL command {unknown:?}; type :help for available commands"
            );
            ReplControl::Continue
        }
    }
}

fn event_selector(command: &str) -> Option<&str> {
    let selector = command.strip_prefix(":event")?;
    selector
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then(|| selector.trim())
}

fn section_selector(command: &str) -> Option<&str> {
    let selector = command.strip_prefix(":section")?;
    selector
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then(|| selector.trim())
}

fn select_event(selector: &str, session: &mut SkriptSession, output: &mut dyn Write) {
    if selector.eq_ignore_ascii_case("off") || selector.eq_ignore_ascii_case("clear") {
        match session.clear_event_context() {
            Ok(()) => {
                let _ = writeln!(output, "Event context cleared");
            }
            Err(error) => {
                let _ = writeln!(output, "error: {error}");
            }
        }
        return;
    }
    match session.select_event_header(selector) {
        Ok(event) => {
            let _ = writeln!(
                output,
                "Event context: {} [{}]",
                event.input, event.registration_id
            );
        }
        Err(error) => {
            let _ = writeln!(output, "error: {error}");
        }
    }
}

fn select_section(selector: &str, session: &mut SkriptSession, output: &mut dyn Write) {
    if selector.eq_ignore_ascii_case("off") || selector.eq_ignore_ascii_case("clear") {
        match session.clear_section_contexts() {
            Ok(()) => {
                let _ = writeln!(output, "Section contexts cleared");
            }
            Err(error) => {
                let _ = writeln!(output, "error: {error}");
            }
        }
        return;
    }
    if selector.eq_ignore_ascii_case("pop") {
        match session.pop_section_context() {
            Ok(Some(section)) => {
                let _ = writeln!(output, "Section context popped: {}", section.input);
            }
            Ok(None) => {
                let _ = writeln!(output, "Section context: none");
            }
            Err(error) => {
                let _ = writeln!(output, "error: {error}");
            }
        }
        return;
    }
    match session.select_section_header(selector) {
        Ok(section) => {
            let _ = writeln!(
                output,
                "Section context: {} [{}]",
                section.input, section.frame.registration_id
            );
        }
        Err(error) => {
            let _ = writeln!(output, "error: {error}");
        }
    }
}

fn write_context(session: &SkriptSession, output: &mut dyn Write) {
    if let Some(event) = session.event_context() {
        let _ = writeln!(output, "Event context:");
        let _ = writeln!(output, "  input: {}", event.input);
        let _ = writeln!(output, "  registrationId: {}", event.registration_id);
        if let Some(class) = &event.element_class {
            let _ = writeln!(output, "  class: {class}");
        }
        let _ = writeln!(output, "  referenceEvents: {:?}", event.reference_events);
        let _ = writeln!(output, "  eventValues: {}", event.event_values.len());
    } else {
        let _ = writeln!(output, "Event context: none");
    }
    let sections = session.section_contexts().collect::<Vec<_>>();
    if sections.is_empty() {
        let _ = writeln!(output, "Section contexts: none");
        return;
    }
    let _ = writeln!(output, "Section contexts (outermost to innermost):");
    for (depth, section) in sections.into_iter().enumerate() {
        let _ = writeln!(
            output,
            "  {}. {} [{}]",
            depth + 1,
            section.input,
            section.frame.registration_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> RawTreeOptions {
        RawTreeOptions::for_skript_version(2, 16)
    }

    #[test]
    fn help_documents_every_required_command() {
        for command in [
            ":help",
            ":reload",
            ":event <header>",
            ":event off",
            ":events",
            ":section <header>",
            ":section pop",
            ":section off|clear",
            ":context",
            ":json on",
            ":json off",
            ":submit",
            ":cancel",
            ":quit",
        ] {
            assert!(REPL_HELP.contains(command));
        }
    }

    #[test]
    fn event_selector_accepts_any_whitespace_without_matching_events_command() {
        assert_eq!(event_selector(":event join"), Some("join"));
        assert_eq!(event_selector(":event\tjoin"), Some("join"));
        assert_eq!(event_selector(":event   on join:"), Some("on join:"));
        assert_eq!(event_selector(":event"), None);
        assert_eq!(event_selector(":events"), None);
    }

    #[test]
    fn section_selector_accepts_any_whitespace() {
        assert_eq!(
            section_selector(":section loop all players"),
            Some("loop all players")
        );
        assert_eq!(
            section_selector(":section\tloop 3 times:"),
            Some("loop 3 times:")
        );
        assert_eq!(section_selector(":section"), None);
    }

    #[test]
    fn structure_starts_a_document_but_colons_in_effects_do_not() {
        assert!(starts_document("on join:", options()));
        assert!(!starts_document("send \"value:\"", options()));
        assert!(starts_document("# note:", options()));
    }

    #[test]
    fn multiline_paste_keeps_internal_blank_lines_until_explicit_submit() {
        let mut buffer = InputBuffer::default();
        assert_eq!(
            buffer.accept("on join:\n    send 1\n\n    send 2".to_owned(), options()),
            InputAction::ContinueDocument
        );
        assert_eq!(
            buffer.accept(":submit".to_owned(), options()),
            InputAction::AnalyzeDocument("on join:\n    send 1\n\n    send 2".to_owned())
        );
    }

    #[test]
    fn blank_line_submits_and_cancel_discards_only_the_current_document() {
        let mut buffer = InputBuffer::default();
        assert_eq!(
            buffer.accept("on join:".to_owned(), options()),
            InputAction::ContinueDocument
        );
        assert_eq!(
            buffer.accept("    send 1".to_owned(), options()),
            InputAction::ContinueDocument
        );
        assert_eq!(
            buffer.accept(String::new(), options()),
            InputAction::AnalyzeDocument("on join:\n    send 1".to_owned())
        );
        assert!(!buffer.is_collecting());

        buffer.accept("on load:".to_owned(), options());
        assert_eq!(
            buffer.accept(":cancel".to_owned(), options()),
            InputAction::Cancelled
        );
        assert!(!buffer.is_collecting());

        buffer.accept("on join:".to_owned(), options());
        assert_eq!(
            buffer.accept("    :submit".to_owned(), options()),
            InputAction::AnalyzeDocument("on join:".to_owned())
        );
    }

    #[test]
    fn indentation_follows_nested_sections_and_can_be_replaced_by_the_editor() {
        let mut buffer = InputBuffer::default();
        buffer.accept("on join:".to_owned(), options());
        assert_eq!(buffer.suggested_indent(options()), "    ");
        buffer.accept("    loop all players:".to_owned(), options());
        assert_eq!(buffer.suggested_indent(options()), "        ");
        buffer.accept("        send loop-player".to_owned(), options());
        assert_eq!(buffer.suggested_indent(options()), "        ");
        buffer.accept("    send \"done\"".to_owned(), options());
        assert_eq!(buffer.suggested_indent(options()), "    ");
    }

    #[test]
    fn triple_hash_comment_uses_version_selected_raw_tree_rules() {
        let mut buffer = InputBuffer::default();
        assert_eq!(
            buffer.accept("###".to_owned(), options()),
            InputAction::ContinueDocument
        );
        buffer.accept("comment".to_owned(), options());
        assert_eq!(
            buffer.accept(String::new(), options()),
            InputAction::ContinueDocument
        );
        buffer.accept("###".to_owned(), options());
        assert!(buffer.is_collecting());
        assert_eq!(
            buffer.accept(String::new(), options()),
            InputAction::AnalyzeDocument("###\ncomment\n\n###".to_owned())
        );
    }

    #[test]
    fn meta_commands_remain_available_inside_an_unclosed_block_comment() {
        let mut buffer = InputBuffer::default();
        assert_eq!(
            buffer.accept("###".to_owned(), options()),
            InputAction::ContinueDocument
        );
        assert_eq!(
            buffer.accept(":submit".to_owned(), options()),
            InputAction::AnalyzeDocument("###".to_owned())
        );
    }
}
