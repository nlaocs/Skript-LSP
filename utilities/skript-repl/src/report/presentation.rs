use std::fmt::{self, Display};

/// Presentation only: the caller decides whether this stream supports color.
#[derive(Clone, Copy)]
pub(crate) struct HumanStyle(pub bool);

#[derive(Clone, Copy)]
pub(crate) enum Role {
    Heading,
    Success,
    Warning,
    Error,
    JavaClass,
    Name,
    Muted,
}

impl HumanStyle {
    pub fn paint<T: Display>(self, role: Role, value: T) -> impl Display {
        Painted {
            color: self.0,
            role,
            value,
        }
    }

    pub fn status<T: Display>(self, status: &str, value: T) -> impl Display {
        self.paint(
            match status {
                "matched" => Role::Success,
                "warning" | "incomplete" => Role::Warning,
                "error" | "unknown" => Role::Error,
                _ => Role::Name,
            },
            value,
        )
    }

    pub fn java_classes_debug(self, classes: &[String]) -> String {
        let classes = classes
            .iter()
            .map(|class| {
                let quoted = format!("{class:?}");
                format!(
                    "\"{}\"",
                    self.paint(Role::JavaClass, &quoted[1..quoted.len() - 1])
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{classes}]")
    }

    pub fn java_classes(self, classes: &[String], separator: &str) -> String {
        classes
            .iter()
            .map(|class| self.paint(Role::JavaClass, class).to_string())
            .collect::<Vec<_>>()
            .join(separator)
    }
}

struct Painted<T> {
    color: bool,
    role: Role,
    value: T,
}

impl<T: Display> Display for Painted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.color {
            return self.value.fmt(f);
        }
        let code = match self.role {
            Role::Heading => "1",
            Role::Success => "1;38;2;131;193;103",
            Role::Warning => "1;38;2;240;172;95",
            Role::Error => "1;38;2;252;98;85",
            Role::JavaClass => super::SourceColor::TypeName.ansi_code(),
            Role::Name => return self.value.fmt(f),
            Role::Muted => "2",
        };
        write!(f, "\x1b[{code}m{}\x1b[0m", self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OutputFormat, SkriptSession};
    use std::path::Path;

    fn strip_sgr(text: &str) -> String {
        let mut chars = text.chars();
        let mut plain = String::new();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                assert_eq!(chars.next(), Some('['));
                assert!(chars.by_ref().any(|ch| ch == 'm'), "unterminated SGR");
            } else {
                plain.push(ch);
            }
        }
        plain
    }

    fn session() -> SkriptSession {
        SkriptSession::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/current-multi-addon-2.15.4"),
        )
        .unwrap()
    }

    #[test]
    fn human_roles_preserve_plain_text_and_json_and_prioritize_results() {
        let report = session()
            .analyze_effect("send sin(1 + cos(2)) to console")
            .unwrap();
        assert!(report.matched());
        let plain =
            String::from_utf8(report.clone().render(OutputFormat::Human, false).unwrap()).unwrap();
        let colored =
            String::from_utf8(report.clone().render(OutputFormat::Human, true).unwrap()).unwrap();
        assert_eq!(strip_sgr(&colored), plain);
        println!("\n--- UX EXAMPLE effect ---\n{colored}--- UX END ---");
        assert!(!plain.contains('\x1b'));
        let element = colored
            .lines()
            .find(|line| line.contains("- expression"))
            .unwrap();
        assert!(element.contains("\x1b[38;2;160;160;160msin("));
        assert!(!element.contains("\\u{1b}"));
        let class_prefix = "\x1b[38;2;255;134;47m";
        assert!(colored.contains(&format!(
            "returnType: {class_prefix}java.lang.Number\x1b[0m"
        )));
        assert!(colored.contains(&format!(
            "expectedTypes: object[] ({class_prefix}java.lang.Object\x1b[0m)"
        )));
        assert!(colored.contains(&format!("class: {class_prefix}org.skriptlang.skript.bukkit.text.elements.effects.EffMessage\x1b[0m")));
        for line in colored
            .lines()
            .filter(|line| line.contains("multiplicity:") || line.contains("name:"))
        {
            assert!(
                !line.contains("38;2;"),
                "metadata must stay neutral: {line}"
            );
        }

        assert!(plain.starts_with("result: matched\n"));
        assert!(plain.find("  elements:").unwrap() < plain.find("  details:").unwrap());
        assert!(plain.find("arguments:").unwrap() < plain.find("identity:").unwrap());
        assert!(plain.find("arguments:").unwrap() < plain.find("metadata:").unwrap());
        assert!(plain.find("\nrun:\n").unwrap() > plain.find("effect:").unwrap());
        for role in [Role::Heading, Role::Success, Role::JavaClass, Role::Muted] {
            let marker = HumanStyle(true).paint(role, "ROLE").to_string();
            let prefix = marker.split("ROLE").next().unwrap();
            assert!(colored.contains(prefix), "missing role {prefix:?}");
        }
        assert_eq!(
            report.clone().render(OutputFormat::Json, false).unwrap(),
            report.render(OutputFormat::Json, true).unwrap()
        );
    }

    #[test]
    fn document_roles_preserve_tree_source_and_json() {
        let report = session()
            .analyze_document("on join:\n    send sin(1) to console\n")
            .unwrap();
        assert!(report.matched());
        let mut plain = Vec::new();
        let mut colored = Vec::new();
        report
            .write_with_color(OutputFormat::Human, &mut plain, false)
            .unwrap();
        report
            .write_with_color(OutputFormat::Human, &mut colored, true)
            .unwrap();
        let plain = String::from_utf8(plain).unwrap();
        let colored = String::from_utf8(colored).unwrap();
        assert_eq!(strip_sgr(&colored), plain);
        println!("\n--- UX EXAMPLE document ---\n{colored}--- UX END ---");
        assert!(plain.starts_with("document: matched (0 diagnostics, 0 recoveries)\n"));
        assert!(plain.contains("\nsource:\n"));
        assert!(plain.contains("\ntree:\n"));
        assert!(plain.contains("\nrun:\n  snapshot:"));
        assert!(colored.contains("- \x1b[1mEvent\x1b[0m \x1b[38;2;255;134;47mch.njol.skript.lang.util.SimpleEvent\x1b[0m at"));
        assert!(colored.contains("returnType: \x1b[38;2;255;134;47mjava.lang.Number\x1b[0m"));
        let mut json = Vec::new();
        report
            .write_with_color(OutputFormat::Json, &mut json, true)
            .unwrap();
        assert!(!json.contains(&b'\x1b'));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&json).unwrap(),
            serde_json::from_str::<serde_json::Value>(&report.to_json().unwrap()).unwrap()
        );
    }

    #[test]
    fn statuses_and_diagnostics_have_distinct_roles_without_losing_labels() {
        for status in [
            "matched",
            "incomplete",
            "unknown",
            "error",
            "warning",
            "info",
        ] {
            let colored = HumanStyle(true).status(status, status).to_string();
            assert_eq!(strip_sgr(&colored), status);
            assert_eq!(HumanStyle(false).status(status, status).to_string(), status);
        }
        assert_ne!(
            HumanStyle(true).status("error", "same").to_string(),
            HumanStyle(true).status("warning", "same").to_string()
        );
        let report = session().analyze_effect("send").unwrap();
        assert!(!report.matched());
        let colored =
            String::from_utf8(report.clone().render(OutputFormat::Human, true).unwrap()).unwrap();
        let plain = String::from_utf8(report.render(OutputFormat::Human, false).unwrap()).unwrap();
        assert!(plain.starts_with("result: incomplete\n"));
        assert!(plain.contains("Effect candidate is incomplete"));
        assert!(colored.contains("\x1b[1;38;2;240;172;95mincomplete\x1b[0m"));
        assert!(!plain.contains('\x1b'));
        println!("\n--- UX EXAMPLE incomplete ---\n{colored}--- UX END ---");
    }

    #[test]
    fn document_warning_keeps_its_severity_in_plain_and_colored_output() {
        let report = session().analyze_document("on join:\n").unwrap();
        let json: serde_json::Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
        assert!(
            json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["severity"] == "warning")
        );
        for color in [false, true] {
            let mut output = Vec::new();
            report
                .write_with_color(OutputFormat::Human, &mut output, color)
                .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(strip_sgr(&output).contains("diagnostic: [warning]"));
            assert_eq!(
                output.contains("\x1b[1;38;2;240;172;95mwarning\x1b[0m"),
                color
            );
            if color {
                println!("\n--- UX EXAMPLE warning ---\n{output}--- UX END ---");
            } else {
                assert!(!output.contains('\x1b'));
            }
        }
    }
}
