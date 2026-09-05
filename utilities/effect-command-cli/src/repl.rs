use crate::{EXIT_FAILURE, EXIT_SUCCESS, EffectCommandSession, OutputFormat};
use std::io::{self, BufRead, Write};

const REPL_HELP: &str = r#"REPL commands:
  :help                 Show this command list
  :reload               Reload the configured SSG snapshot
  :event <header>       Select an Event context (`:` is optional)
  :event off            Clear the Event context
  :events               List registered Events
  :section <header>     Push a Section context (`:` is optional)
  :section pop          Pop the innermost Section context
  :section off|clear    Clear all Section contexts
  :context              Show the active Event and Section contexts
  :json on              Use JSON reports
  :json off             Use human-readable reports
  :quit, :exit          Exit the REPL

Every other non-empty line is parsed as one Effect and is never executed."#;

pub(crate) fn run<R: BufRead, W: Write, E: Write>(
    session: &mut EffectCommandSession,
    mut format: OutputFormat,
    mut input: R,
    mut output: W,
    mut error: E,
    color: bool,
) -> u8 {
    if writeln!(output, "Effect Command CLI")
        .and_then(|_| writeln!(output, "Type :help for available commands."))
        .is_err()
    {
        return EXIT_FAILURE;
    }
    let mut line = String::new();
    loop {
        if write!(output, "\neffect> ")
            .and_then(|_| output.flush())
            .is_err()
        {
            return EXIT_FAILURE;
        }
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => {
                let _ = writeln!(output);
                return EXIT_SUCCESS;
            }
            Ok(_) => {}
            Err(read_error) if read_error.kind() == io::ErrorKind::Interrupted => {
                let _ = writeln!(output, "^C");
                continue;
            }
            Err(read_error) => {
                let _ = writeln!(error, "error: failed to read REPL input: {read_error}");
                return EXIT_FAILURE;
            }
        }
        let effect = line.trim_end_matches(['\r', '\n']);
        if effect.trim().is_empty() {
            continue;
        }
        if effect.starts_with(':') {
            match repl_command(effect, session, &mut format, &mut output) {
                ReplControl::Continue => continue,
                ReplControl::Exit => return EXIT_SUCCESS,
            }
        }
        match session.analyze(effect) {
            Ok(report) => {
                if let Err(write_error) = report.write_with_color(format, &mut output, color) {
                    let _ = writeln!(error, "error: failed to write output: {write_error}");
                    return EXIT_FAILURE;
                }
            }
            Err(parse_error) => {
                let _ = writeln!(output, "error: {parse_error}");
            }
        }
    }
}

enum ReplControl {
    Continue,
    Exit,
}

fn repl_command(
    command: &str,
    session: &mut EffectCommandSession,
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

fn select_event(selector: &str, session: &mut EffectCommandSession, output: &mut dyn Write) {
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
    let result = session.select_event_header(selector);
    match result {
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

fn select_section(selector: &str, session: &mut EffectCommandSession, output: &mut dyn Write) {
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

fn write_context(session: &EffectCommandSession, output: &mut dyn Write) {
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
}
