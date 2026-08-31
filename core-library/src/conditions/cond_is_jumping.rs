use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".CondIsJumping";
const HANDLER_ID: &str = "core.condition.cond-is-jumping";
const LIVING_ENTITY: &str = "org.bukkit.entity.LivingEntity";
const HUMAN_ENTITY: &str = "org.bukkit.entity.HumanEntity";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpingVerdict {
    Accepted,
    NonLivingEntity,
    HumanEntity,
    Unresolved,
}

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    super::annotate(&mut payload, "semantic-mode", "mob-jumping");
    let Some(source) = super::child(&payload, 0) else {
        return Some(unresolved(payload, "the source Expression is unavailable"));
    };
    let span = super::child_span(&payload, 0);
    let Some(return_type) = source.return_type.as_deref() else {
        return Some(unresolved(payload, "the source return type is unavailable"));
    };
    let living = crate::catalog::is_class_assignable(return_type, LIVING_ENTITY)
        .unwrap_or(TypeRelation::Unknown);
    let human = crate::catalog::is_class_assignable(return_type, HUMAN_ENTITY)
        .unwrap_or(TypeRelation::Unknown);
    Some(match jumping_verdict(living, human) {
        JumpingVerdict::Accepted => super::accept(payload),
        JumpingVerdict::NonLivingEntity => super::reject_with(
            "the 'is jumping' Condition requires living entities",
            "core.cond-is-jumping.non-living-entity",
            span,
        ),
        JumpingVerdict::HumanEntity => super::reject_with(
            "the 'is jumping' Condition only works on mobs, not human entities",
            "core.cond-is-jumping.human-entity",
            span,
        ),
        JumpingVerdict::Unresolved => unresolved(
            payload,
            "the LivingEntity or HumanEntity relationship of the source type is unresolved",
        ),
    })
}

fn jumping_verdict(living: TypeRelation, human: TypeRelation) -> JumpingVerdict {
    match living {
        TypeRelation::Incompatible => JumpingVerdict::NonLivingEntity,
        TypeRelation::Unknown => JumpingVerdict::Unresolved,
        TypeRelation::Compatible => match human {
            TypeRelation::Compatible => JumpingVerdict::HumanEntity,
            TypeRelation::Incompatible => JumpingVerdict::Accepted,
            TypeRelation::Unknown => JumpingVerdict::Unresolved,
        },
    }
}

fn unresolved(mut payload: ConditionPayload, message: &str) -> HookOutput {
    let span = payload.candidate.span.clone();
    super::mark_unresolved(&mut payload, "core.cond-is-jumping.unresolved-type");
    let mut output = super::accept(payload);
    output.effects.diagnostics.push(super::warning(
        "core.cond-is-jumping.unresolved-type",
        message,
        span,
    ));
    output
}

#[cfg(test)]
mod tests {
    use super::{JumpingVerdict, jumping_verdict};
    use crate::catalog::TypeRelation;

    #[test]
    fn jumping_requires_a_non_human_living_entity() {
        assert_eq!(
            jumping_verdict(TypeRelation::Compatible, TypeRelation::Incompatible),
            JumpingVerdict::Accepted
        );
        assert_eq!(
            jumping_verdict(TypeRelation::Incompatible, TypeRelation::Incompatible),
            JumpingVerdict::NonLivingEntity
        );
        assert_eq!(
            jumping_verdict(TypeRelation::Compatible, TypeRelation::Compatible),
            JumpingVerdict::HumanEntity
        );
        assert_eq!(
            jumping_verdict(TypeRelation::Unknown, TypeRelation::Incompatible),
            JumpingVerdict::Unresolved
        );
    }
}
