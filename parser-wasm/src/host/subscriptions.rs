//! Indexed subscription routes, retaining the existing deterministic order.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TargetKey {
    Stage,
    Kind(u8),
    Definition(String),
    Registration(String),
    Pattern(String, u64),
    Parser(String),
}

#[derive(Default)]
pub(super) struct SubscriptionRoutes {
    routes: HashMap<(u8, TargetKey), TargetRoutes>,
}

// Priority, component load order, declaration order, subscription index.
type SubscriptionOrder = (i32, usize, usize, usize);

#[derive(Default)]
struct TargetRoutes {
    general: Vec<SubscriptionOrder>,
    // A default's requested Type is immutable throughout dispatch. Exact Type
    // selectors can therefore be indexed before any subscription is cloned.
    default_types: HashMap<String, Vec<SubscriptionOrder>>,
}

impl SubscriptionRoutes {
    pub(super) fn register(&mut self, index: usize, entries: &[RegisteredSubscription]) {
        let entry = &entries[index];
        let key = match &entry.subscription.target {
            HookTarget::ParseStage => TargetKey::Stage,
            HookTarget::SyntaxKind(kind) => TargetKey::Kind(*kind as u8),
            HookTarget::Definition(id) => TargetKey::Definition(id.clone()),
            HookTarget::Registration(id) => TargetKey::Registration(id.clone()),
            HookTarget::Pattern(pattern) => {
                TargetKey::Pattern(pattern.registration_id.clone(), pattern.pattern_index)
            }
            HookTarget::Parser(id) => TargetKey::Parser(id.clone()),
        };
        let routes = self
            .routes
            .entry((entry.subscription.phase as u8, key))
            .or_default();
        let route = match &entry.subscription.selector.return_type {
            Some(selector)
                if entry.subscription.phase == HookPhase::DefaultExpression
                    && selector.relation == SelectorTypeRelation::Exact =>
            {
                routes
                    .default_types
                    .entry(selector.class_name.clone())
                    .or_default()
            }
            _ => &mut routes.general,
        };
        let order = (
            entry.subscription.priority,
            entry.load_order,
            entry.declaration_order,
            index,
        );
        let position = route
            .binary_search(&order)
            .unwrap_or_else(|position| position);
        route.insert(position, order);
    }

    pub(super) fn matching(
        &self,
        target: &DispatchTarget,
        phase: HookPhase,
        default_type: Option<&str>,
    ) -> Vec<usize> {
        let mut keys = Vec::with_capacity(4);
        match target {
            DispatchTarget::ParseStage => keys.push(TargetKey::Stage),
            DispatchTarget::Parser(id) => keys.push(TargetKey::Parser(id.clone())),
            DispatchTarget::Pattern {
                registration_id,
                pattern_index,
                ..
            } => {
                keys.push(TargetKey::Pattern(registration_id.clone(), *pattern_index));
            }
            _ => {}
        }
        if let Some(id) = dispatch_registration_id(target) {
            keys.push(TargetKey::Registration(id.to_owned()));
        }
        if let Some(id) = dispatch_definition_id(target) {
            keys.push(TargetKey::Definition(id.to_owned()));
        }
        if let Some(kind) = dispatch_syntax_kind(target) {
            keys.push(TargetKey::Kind(kind as u8));
        }
        let mut result = Vec::new();
        for key in keys {
            let Some(routes) = self.routes.get(&(phase as u8, key)) else {
                continue;
            };
            let mut matching = routes.general.clone();
            if let Some(value_type) = default_type {
                if let Some(typed) = routes.default_types.get(value_type) {
                    matching.extend_from_slice(typed);
                }
            } else {
                // Queries without a payload enumerate a route (tests and macro
                // discovery); actual default dispatch always supplies its Type.
                matching.extend(routes.default_types.values().flatten().copied());
            }
            matching.sort_unstable();
            result.extend(matching.into_iter().map(|entry| entry.3));
        }
        result
    }
}
