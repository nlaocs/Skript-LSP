use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredExpressionTag,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprAllBannedEntries";
const OFFLINE_PLAYER: &str = "org.bukkit.OfflinePlayer";
const STRING: &str = "java.lang.String";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let ip_addresses = has_tag(&payload.tags, "ips");
        SemanticResolution::Resolved {
            return_type: if ip_addresses {
                STRING.to_owned()
            } else {
                OFFLINE_PLAYER.to_owned()
            },
            multiplicity: DynamicMultiplicity::Multiple,
            metadata: vec![
                metadata("semantic-mode", "all-banned-entries"),
                metadata(
                    "entry-kind",
                    if ip_addresses { "ip-address" } else { "player" },
                ),
            ],
        }
    })
}

// This mirrors ExprAllBannedEntries.init: the `ips` parse tag is the semantic
// signal, while the surrounding optional words are only syntax sugar.
fn has_tag(tags: &[RegisteredExpressionTag], value: &str) -> bool {
    tags.iter().any(|tag| tag.value == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(value: &str) -> RegisteredExpressionTag {
        RegisteredExpressionTag {
            value: value.to_owned(),
            implicit: false,
        }
    }

    #[test]
    fn ips_tag_selects_ip_address_entries() {
        assert!(has_tag(&[tag("ips")], "ips"));
        assert!(!has_tag(&[tag("players")], "ips"));
    }

    #[test]
    fn absent_ips_tag_keeps_the_player_variant() {
        assert!(!has_tag(&[], "ips"));
    }
}
