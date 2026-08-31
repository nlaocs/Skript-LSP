use serde::{Deserialize, Serialize};

pub(crate) const CONTEXT_KEY: &str = "core.loop.frames";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LoopFrame {
    pub source: String,
    pub return_type: String,
    pub possible_return_types: Vec<String>,
    pub keyed: Option<bool>,
    pub supports_peeking: Option<bool>,
}

pub(crate) fn decode(value: Option<&str>) -> Vec<LoopFrame> {
    value
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}

pub(crate) fn push(value: Option<&str>, frame: LoopFrame) -> String {
    let mut frames = decode(value);
    frames.push(frame);
    serde_json::to_string(&frames).expect("LoopFrame only contains serializable strings")
}

#[cfg(test)]
mod tests {
    use super::{LoopFrame, decode, push};

    #[test]
    fn nested_loop_frames_round_trip_in_source_order() {
        let outer = LoopFrame {
            source: "all players".to_owned(),
            return_type: "org.bukkit.entity.Player".to_owned(),
            possible_return_types: vec!["org.bukkit.entity.Player".to_owned()],
            keyed: Some(false),
            supports_peeking: Some(true),
        };
        let inner = LoopFrame {
            source: "indices of {values::*}".to_owned(),
            return_type: "java.lang.Long".to_owned(),
            possible_return_types: vec!["java.lang.Long".to_owned()],
            keyed: Some(true),
            supports_peeking: None,
        };
        let first = push(None, outer.clone());
        let second = push(Some(&first), inner.clone());
        assert_eq!(decode(Some(&second)), vec![outer, inner]);
    }
}
