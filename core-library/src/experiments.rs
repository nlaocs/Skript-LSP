use crate::nlaocs::skript_parser_addon::types::ParseContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExperimentPhase {
    Stable,
    Experimental,
    #[allow(dead_code)]
    Deprecated,
    Mainstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Experiment {
    pub code_name: String,
    pub phase: ExperimentPhase,
}

pub(crate) fn find(input: &str) -> Result<Option<Experiment>, String> {
    let input = normalized(input);
    let Some((minimum, experiment)) = builtin(&input) else {
        let experiment = crate::catalog::experiments()?
            .into_iter()
            .find(|experiment| normalized(&experiment.code_name) == input)
            .and_then(|experiment| {
                phase(&experiment.phase).map(|phase| Experiment {
                    code_name: experiment.code_name,
                    phase,
                })
            });
        return Ok(experiment);
    };
    match crate::runtime::skript_at_least(minimum.0, minimum.1) {
        Some(true) => Ok(Some(experiment)),
        Some(false) => Ok(None),
        None => Err("Skript version is unavailable or newer than CoreLibrary supports".to_owned()),
    }
}

fn builtin(input: &str) -> Option<((u64, u64), Experiment)> {
    let value = match input {
        "examples" => ((2, 10), Experiment::stable("examples")),
        "queues" => ((2, 10), Experiment::experimental("queues")),
        "for loop" | "for loops" | "for each loop" | "for each loops" => {
            ((2, 10), Experiment::mainstream("for loop"))
        }
        "reflection" | "script reflection" => ((2, 10), Experiment::stable("reflection")),
        "catch runtime errors" | "error catching" | "error catching section" => {
            ((2, 12), Experiment::experimental("catch runtime errors"))
        }
        "type hints" | "local variable type hints" => {
            ((2, 12), Experiment::experimental("type hints"))
        }
        "damage source" | "damage sources" => ((2, 12), Experiment::experimental("damage source")),
        "equippable components" => ((2, 13), Experiment::experimental("equippable components")),
        _ => return None,
    };
    Some(value)
}

fn phase(value: &str) -> Option<ExperimentPhase> {
    match normalized(value).as_str() {
        "stable" => Some(ExperimentPhase::Stable),
        "experimental" => Some(ExperimentPhase::Experimental),
        "deprecated" => Some(ExperimentPhase::Deprecated),
        "mainstream" => Some(ExperimentPhase::Mainstream),
        _ => None,
    }
}

pub(crate) fn enabled(context: &ParseContext, code_name: &str) -> bool {
    let key = context_key(code_name);
    context
        .values
        .iter()
        .rfind(|entry| entry.key == key)
        .is_some_and(|entry| entry.value == "true")
}

pub(crate) fn context_key(code_name: &str) -> String {
    format!(
        "core.experiment.{}",
        normalized(code_name).replace(' ', "-")
    )
}

fn normalized(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

impl Experiment {
    fn stable(code_name: &str) -> Self {
        Self {
            code_name: code_name.to_owned(),
            phase: ExperimentPhase::Stable,
        }
    }

    fn experimental(code_name: &str) -> Self {
        Self {
            code_name: code_name.to_owned(),
            phase: ExperimentPhase::Experimental,
        }
    }

    fn mainstream(code_name: &str) -> Self {
        Self {
            code_name: code_name.to_owned(),
            phase: ExperimentPhase::Mainstream,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExperimentPhase, builtin, normalized};

    #[test]
    fn normalizes_builtin_feature_patterns() {
        assert_eq!(
            normalized(" error   catching section "),
            "error catching section"
        );
        assert_eq!(
            builtin("error catching section").unwrap().1.code_name,
            "catch runtime errors"
        );
        assert_eq!(
            builtin("for each loops").unwrap().1.phase,
            ExperimentPhase::Mainstream
        );
        assert!(builtin("my addon feature").is_none());
    }
}
