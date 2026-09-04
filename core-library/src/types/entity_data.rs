use super::registered_literal::candidate_from_option;
use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLiteralOption,
    ExpressionPayload,
};

const ENTITY_DATA: &str = "ch.njol.skript.entity.EntityData";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.entity-data",
    classes: &["ch.njol.skript.entity.EntityData"],
    parse,
    unresolved: None,
    all_type_options: true,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }

    // New snapshots expose these values as structured supplier literals.
    // Preserve their exact SSG metadata rather
    // than reducing an entity to a bare word and a guessed Bukkit class.
    let literal = crate::language::strip_indefinite_article(text);
    let literal_start = payload
        .remaining
        .start
        .checked_add(u64::try_from(text.len() - literal.len()).ok()?)?;
    let mut parsed = parse_without_indefinite_article(payload, literal, literal_start, end)?;
    parsed.range.start = payload.remaining.start;
    Some(parsed)
}

/// Parses the entity description after a containing Type has consumed its prefix.
/// Unlike `parse`, this must not strip another article from that description.
pub(super) fn parse_without_indefinite_article(
    payload: &ExpressionPayload,
    text: &str,
    start: u64,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    // Skript 2.6.4 through 2.9.5 reject these characters before parseStatic;
    // 2.10+ delegates directly to the registered EntityData patterns.
    // https://github.com/SkriptLang/Skript/blob/2.9.5/src/main/java/ch/njol/skript/entity/EntityData.java
    if crate::runtime::skript_at_least(2, 10) == Some(false)
        && (text.is_empty()
            || !text
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || matches!(byte, b' ' | b'-')))
    {
        return None;
    }
    // EntityData delegates to SkriptParser.parseStatic, whose String.trim()
    // removes only characters <= U+0020, not Rust's broader Unicode whitespace.
    let without_leading = text.trim_start_matches(|character| character <= '\u{20}');
    let literal = without_leading.trim_end_matches(|character| character <= '\u{20}');
    // The shared literal index uses Unicode trim. Do not let it consume extra
    // non-Java whitespace in the entity description on this parser's behalf.
    if literal.trim() != literal {
        return None;
    }
    let literal_start = start.checked_add((text.len() - without_leading.len()) as u64)?;
    let literal_end = end.checked_sub((without_leading.len() - literal.len()) as u64)?;
    if let Some(option) = payload
        .literal_options
        .iter()
        .filter(|option| {
            option.range.start == literal_start
                && option.range.end == literal_end
                && option.class_name == ENTITY_DATA
        })
        .min_by_key(|option| option.type_parse_order)
    {
        return Some(candidate_from_entity_option(option, start, end));
    }

    if !accepts_entity_data_fallback(payload) {
        return None;
    }

    let profile = crate::runtime::current();
    let version = profile
        .as_ref()
        .and_then(|profile| profile.minecraft_version.as_deref())?;
    let literal = legacy_entity_literal_without_article(literal, Some(version))?;
    let mut candidate = candidate(
        "core.literal.entity-data",
        ExpressionLeafKind::Literal,
        start,
        end,
        ENTITY_DATA,
        DynamicMultiplicity::Single,
    );
    candidate.metadata = vec![
        metadata("entity-class", literal.class_name),
        metadata(
            "entity-plural",
            if literal.is_plural { "true" } else { "false" },
        ),
        metadata("entity-source", "core.legacy-compatibility"),
    ];
    Some(candidate)
}

fn candidate_from_entity_option(
    option: &ExpressionLiteralOption,
    start: u64,
    end: u64,
) -> ExpressionLeafCandidate {
    let mut candidate = candidate_from_option(option, "core.literal.entity-data", start, end);
    if let Some(represented_class) = option.represented_class.as_deref() {
        candidate
            .metadata
            .push(metadata("entity-class", represented_class));
    }
    candidate.metadata.push(metadata(
        "entity-plural",
        if option.plural { "true" } else { "false" },
    ));
    candidate
}

fn accepts_entity_data_fallback(payload: &ExpressionPayload) -> bool {
    let accepts_entity_data = payload.expected_types.iter().any(|expected| {
        expected.class_name == ENTITY_DATA
            || expected.class_name == "ch.njol.skript.entity.EntityType"
            || expected.class_name == "java.lang.Object"
    });
    let supplier_is_available = payload
        .type_options
        .iter()
        .any(|option| option.class_name == ENTITY_DATA && option.has_supplier);
    // Even new SSG schemas can describe Skript 2.6.4, which has no supplier.
    // Composite Type handlers request all type options to inspect this flag.
    let structured_literals_are_available =
        supplier_is_available && crate::runtime::snapshot_schema_at_least(4) == Some(true);
    accepts_entity_data && !structured_literals_are_available
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LegacyEntityLiteral {
    singular: &'static str,
    plural_name: &'static str,
    class_name: &'static str,
    min_minecraft: (u16, u16, u16),
    is_plural: bool,
}

const fn entity(
    singular: &'static str,
    plural: &'static str,
    class_name: &'static str,
) -> LegacyEntityLiteral {
    LegacyEntityLiteral {
        singular,
        plural_name: plural,
        class_name,
        min_minecraft: (1, 0, 0),
        is_plural: false,
    }
}

const fn entity_since(
    singular: &'static str,
    plural: &'static str,
    class_name: &'static str,
    min_minecraft: (u16, u16, u16),
) -> LegacyEntityLiteral {
    LegacyEntityLiteral {
        singular,
        plural_name: plural,
        class_name,
        min_minecraft,
        is_plural: false,
    }
}

// Older snapshot schemas do not export the EntityData supplier values. These are the
// names registered by EntityData/SimpleEntityData and the specialized EntityData
// classes. New snapshots use the SSG `typeLiterals` branch above; the Minecraft
// gate keeps this compatibility path from inventing unavailable entities.
const LEGACY_ENTITY_LITERALS: &[LegacyEntityLiteral] = &[
    entity("player", "players", "org.bukkit.entity.Player"),
    entity("non-op", "non-ops", "org.bukkit.entity.Player"),
    entity("op", "ops", "org.bukkit.entity.Player"),
    entity("arrow", "arrows", "org.bukkit.entity.Arrow"),
    entity_since(
        "spectral arrow",
        "spectral arrows",
        "org.bukkit.entity.SpectralArrow",
        (1, 9, 0),
    ),
    entity_since(
        "tipped arrow",
        "tipped arrows",
        "org.bukkit.entity.TippedArrow",
        (1, 9, 0),
    ),
    entity("boat", "boats", "org.bukkit.entity.Boat"),
    entity("blaze", "blazes", "org.bukkit.entity.Blaze"),
    entity("chicken", "chickens", "org.bukkit.entity.Chicken"),
    entity("mooshroom", "mooshrooms", "org.bukkit.entity.MushroomCow"),
    entity("cow", "cows", "org.bukkit.entity.Cow"),
    entity(
        "cave spider",
        "cave spiders",
        "org.bukkit.entity.CaveSpider",
    ),
    entity_since(
        "dragon fireball",
        "dragon fireballs",
        "org.bukkit.entity.DragonFireball",
        (1, 9, 0),
    ),
    entity("egg", "eggs", "org.bukkit.entity.Egg"),
    entity(
        "ender crystal",
        "ender crystals",
        "org.bukkit.entity.EnderCrystal",
    ),
    entity(
        "ender dragon",
        "ender dragons",
        "org.bukkit.entity.EnderDragon",
    ),
    entity(
        "ender pearl",
        "ender pearls",
        "org.bukkit.entity.EnderPearl",
    ),
    entity(
        "small fireball",
        "small fireballs",
        "org.bukkit.entity.SmallFireball",
    ),
    entity(
        "large fireball",
        "large fireballs",
        "org.bukkit.entity.LargeFireball",
    ),
    entity("fireball", "fireballs", "org.bukkit.entity.Fireball"),
    entity("fish hook", "fish hooks", "org.bukkit.entity.FishHook"),
    entity("ghast", "ghasts", "org.bukkit.entity.Ghast"),
    entity("giant", "giants", "org.bukkit.entity.Giant"),
    entity("iron golem", "iron golems", "org.bukkit.entity.IronGolem"),
    entity("magma cube", "magma cubes", "org.bukkit.entity.MagmaCube"),
    entity("slime", "slimes", "org.bukkit.entity.Slime"),
    entity("painting", "paintings", "org.bukkit.entity.Painting"),
    entity(
        "zombie pigman",
        "zombie pigmen",
        "org.bukkit.entity.PigZombie",
    ),
    entity("silverfish", "silverfish", "org.bukkit.entity.Silverfish"),
    entity("snowball", "snowballs", "org.bukkit.entity.Snowball"),
    entity("snow golem", "snow golems", "org.bukkit.entity.Snowman"),
    entity("spider", "spiders", "org.bukkit.entity.Spider"),
    entity(
        "bottle of enchanting",
        "bottles of enchanting",
        "org.bukkit.entity.ThrownExpBottle",
    ),
    entity("tnt", "tnt", "org.bukkit.entity.TNTPrimed"),
    entity(
        "leash hitch",
        "leash hitches",
        "org.bukkit.entity.LeashHitch",
    ),
    entity("item frame", "item frames", "org.bukkit.entity.ItemFrame"),
    entity("bat", "bats", "org.bukkit.entity.Bat"),
    entity("witch", "witches", "org.bukkit.entity.Witch"),
    entity("wither", "withers", "org.bukkit.entity.Wither"),
    entity(
        "wither skull",
        "wither skulls",
        "org.bukkit.entity.WitherSkull",
    ),
    entity("firework", "fireworks", "org.bukkit.entity.Firework"),
    entity("endermite", "endermites", "org.bukkit.entity.Endermite"),
    entity(
        "armor stand",
        "armor stands",
        "org.bukkit.entity.ArmorStand",
    ),
    entity("shulker", "shulkers", "org.bukkit.entity.Shulker"),
    entity(
        "shulker bullet",
        "shulker bullets",
        "org.bukkit.entity.ShulkerBullet",
    ),
    entity("polar bear", "polar bears", "org.bukkit.entity.PolarBear"),
    entity(
        "area effect cloud",
        "area effect clouds",
        "org.bukkit.entity.AreaEffectCloud",
    ),
    entity("dropped item", "dropped items", "org.bukkit.entity.Item"),
    entity(
        "falling block",
        "falling blocks",
        "org.bukkit.entity.FallingBlock",
    ),
    entity("enderman", "endermen", "org.bukkit.entity.Enderman"),
    entity("cat", "cats", "org.bukkit.entity.Cat"),
    entity("creeper", "creepers", "org.bukkit.entity.Creeper"),
    entity(
        "unpowered creeper",
        "unpowered creepers",
        "org.bukkit.entity.Creeper",
    ),
    entity(
        "powered creeper",
        "powered creepers",
        "org.bukkit.entity.Creeper",
    ),
    entity("pig", "pigs", "org.bukkit.entity.Pig"),
    entity("unsaddled pig", "unsaddled pigs", "org.bukkit.entity.Pig"),
    entity("saddled pig", "saddled pigs", "org.bukkit.entity.Pig"),
    entity("sheep", "sheep", "org.bukkit.entity.Sheep"),
    entity(
        "unsheared sheep",
        "unsheared sheep",
        "org.bukkit.entity.Sheep",
    ),
    entity("sheared sheep", "sheared sheep", "org.bukkit.entity.Sheep"),
    entity("wolf", "wolves", "org.bukkit.entity.Wolf"),
    entity("angry wolf", "angry wolves", "org.bukkit.entity.Wolf"),
    entity("peaceful wolf", "peaceful wolves", "org.bukkit.entity.Wolf"),
    entity("wild wolf", "wild wolves", "org.bukkit.entity.Wolf"),
    entity("tamed wolf", "tamed wolves", "org.bukkit.entity.Wolf"),
    entity("rabbit", "rabbits", "org.bukkit.entity.Rabbit"),
    entity("black rabbit", "black rabbits", "org.bukkit.entity.Rabbit"),
    entity(
        "black and white rabbit",
        "black and white rabbits",
        "org.bukkit.entity.Rabbit",
    ),
    entity("brown rabbit", "brown rabbits", "org.bukkit.entity.Rabbit"),
    entity("gold rabbit", "gold rabbits", "org.bukkit.entity.Rabbit"),
    entity(
        "salt and pepper rabbit",
        "salt and pepper rabbits",
        "org.bukkit.entity.Rabbit",
    ),
    entity(
        "killer rabbit",
        "killer rabbits",
        "org.bukkit.entity.Rabbit",
    ),
    entity("white rabbit", "white rabbits", "org.bukkit.entity.Rabbit"),
    entity("ocelot", "ocelots", "org.bukkit.entity.Ocelot"),
    entity("parrot", "parrots", "org.bukkit.entity.Parrot"),
    entity("panda", "pandas", "org.bukkit.entity.Panda"),
    entity("villager", "villagers", "org.bukkit.entity.Villager"),
    entity(
        "normal villager",
        "normal villagers",
        "org.bukkit.entity.Villager",
    ),
    entity("farmer", "farmers", "org.bukkit.entity.Villager"),
    entity("librarian", "librarians", "org.bukkit.entity.Villager"),
    entity("priest", "priests", "org.bukkit.entity.Villager"),
    entity("blacksmith", "blacksmiths", "org.bukkit.entity.Villager"),
    entity("butcher", "butchers", "org.bukkit.entity.Villager"),
    entity("nitwit", "nitwits", "org.bukkit.entity.Villager"),
    entity(
        "thrown potion",
        "thrown potions",
        "org.bukkit.entity.ThrownPotion",
    ),
    entity("minecart", "minecarts", "org.bukkit.entity.Minecart"),
    entity("xp-orb", "xp-orbs", "org.bukkit.entity.ExperienceOrb"),
    entity("xp orb", "xp orbs", "org.bukkit.entity.ExperienceOrb"),
    entity(
        "zombie villager",
        "zombie villagers",
        "org.bukkit.entity.ZombieVillager",
    ),
    entity_since(
        "wither skeleton",
        "wither skeletons",
        "org.bukkit.entity.WitherSkeleton",
        (1, 11, 0),
    ),
    entity_since("stray", "strays", "org.bukkit.entity.Stray", (1, 11, 0)),
    entity_since("husk", "husks", "org.bukkit.entity.Husk", (1, 11, 0)),
    entity_since(
        "skeleton",
        "skeletons",
        "org.bukkit.entity.Skeleton",
        (1, 11, 0),
    ),
    entity_since(
        "elder guardian",
        "elder guardians",
        "org.bukkit.entity.ElderGuardian",
        (1, 11, 0),
    ),
    entity_since(
        "normal guardian",
        "normal guardians",
        "org.bukkit.entity.Guardian",
        (1, 11, 0),
    ),
    entity_since(
        "guardian",
        "guardians",
        "org.bukkit.entity.Guardian",
        (1, 11, 0),
    ),
    entity_since("donkey", "donkeys", "org.bukkit.entity.Donkey", (1, 11, 0)),
    entity_since("mule", "mules", "org.bukkit.entity.Mule", (1, 11, 0)),
    entity_since("llama", "llamas", "org.bukkit.entity.Llama", (1, 11, 0)),
    entity_since(
        "undead horse",
        "undead horses",
        "org.bukkit.entity.ZombieHorse",
        (1, 11, 0),
    ),
    entity_since(
        "skeleton horse",
        "skeleton horses",
        "org.bukkit.entity.SkeletonHorse",
        (1, 11, 0),
    ),
    entity_since("horse", "horses", "org.bukkit.entity.Horse", (1, 11, 0)),
    entity_since(
        "chested horse",
        "chested horses",
        "org.bukkit.entity.ChestedHorse",
        (1, 11, 0),
    ),
    entity_since(
        "any horse",
        "any horses",
        "org.bukkit.entity.AbstractHorse",
        (1, 11, 0),
    ),
    entity_since(
        "llama spit",
        "llama spits",
        "org.bukkit.entity.LlamaSpit",
        (1, 11, 0),
    ),
    entity_since("evoker", "evokers", "org.bukkit.entity.Evoker", (1, 11, 0)),
    entity_since(
        "evoker fangs",
        "evoker fangs",
        "org.bukkit.entity.EvokerFangs",
        (1, 11, 0),
    ),
    entity_since("vex", "vexes", "org.bukkit.entity.Vex", (1, 11, 0)),
    entity_since(
        "vindicator",
        "vindicators",
        "org.bukkit.entity.Vindicator",
        (1, 11, 0),
    ),
    entity_since(
        "illusioner",
        "illusioners",
        "org.bukkit.entity.Illusioner",
        (1, 12, 0),
    ),
    entity_since(
        "dolphin",
        "dolphins",
        "org.bukkit.entity.Dolphin",
        (1, 13, 0),
    ),
    entity_since(
        "phantom",
        "phantoms",
        "org.bukkit.entity.Phantom",
        (1, 13, 0),
    ),
    entity_since(
        "drowned",
        "drowned",
        "org.bukkit.entity.Drowned",
        (1, 13, 0),
    ),
    entity_since("turtle", "turtles", "org.bukkit.entity.Turtle", (1, 13, 0)),
    entity_since("cod", "cod", "org.bukkit.entity.Cod", (1, 13, 0)),
    entity_since(
        "puffer fish",
        "puffer fish",
        "org.bukkit.entity.PufferFish",
        (1, 13, 0),
    ),
    entity_since("salmon", "salmon", "org.bukkit.entity.Salmon", (1, 13, 0)),
    entity_since(
        "tropical fish",
        "tropical fish",
        "org.bukkit.entity.TropicalFish",
        (1, 13, 0),
    ),
    entity_since(
        "trident",
        "tridents",
        "org.bukkit.entity.Trident",
        (1, 13, 0),
    ),
    entity_since(
        "pillager",
        "pillagers",
        "org.bukkit.entity.Pillager",
        (1, 14, 0),
    ),
    entity_since(
        "ravager",
        "ravagers",
        "org.bukkit.entity.Ravager",
        (1, 14, 0),
    ),
    entity_since(
        "wandering trader",
        "wandering traders",
        "org.bukkit.entity.WanderingTrader",
        (1, 14, 0),
    ),
    entity_since("piglin", "piglins", "org.bukkit.entity.Piglin", (1, 16, 0)),
    entity_since("hoglin", "hoglins", "org.bukkit.entity.Hoglin", (1, 16, 0)),
    entity_since("zoglin", "zoglins", "org.bukkit.entity.Zoglin", (1, 16, 0)),
    entity_since(
        "strider",
        "striders",
        "org.bukkit.entity.Strider",
        (1, 16, 0),
    ),
    entity_since(
        "piglin brute",
        "piglin brutes",
        "org.bukkit.entity.PiglinBrute",
        (1, 16, 2),
    ),
    entity_since(
        "glow squid",
        "glow squids",
        "org.bukkit.entity.GlowSquid",
        (1, 17, 0),
    ),
    entity_since("marker", "markers", "org.bukkit.entity.Marker", (1, 17, 0)),
    entity_since(
        "glow item frame",
        "glow item frames",
        "org.bukkit.entity.GlowItemFrame",
        (1, 17, 0),
    ),
    entity_since("allay", "allays", "org.bukkit.entity.Allay", (1, 19, 0)),
    entity_since(
        "tadpole",
        "tadpoles",
        "org.bukkit.entity.Tadpole",
        (1, 19, 0),
    ),
    entity_since("warden", "wardens", "org.bukkit.entity.Warden", (1, 19, 0)),
    entity("zombie", "zombies", "org.bukkit.entity.Zombie"),
    entity("squid", "squids", "org.bukkit.entity.Squid"),
    entity("human", "humans", "org.bukkit.entity.HumanEntity"),
    entity("damageable", "damageables", "org.bukkit.entity.Damageable"),
    entity("monster", "monsters", "org.bukkit.entity.Monster"),
    entity_since("mob", "mobs", "org.bukkit.entity.Mob", (1, 13, 0)),
    entity("creature", "creatures", "org.bukkit.entity.Creature"),
    entity("animal", "animals", "org.bukkit.entity.Animals"),
    entity("golem", "golems", "org.bukkit.entity.Golem"),
    entity("projectile", "projectiles", "org.bukkit.entity.Projectile"),
    entity(
        "living entity",
        "living entities",
        "org.bukkit.entity.LivingEntity",
    ),
    entity("entity", "entities", "org.bukkit.entity.Entity"),
    entity("water mob", "water mobs", "org.bukkit.entity.WaterMob"),
    entity("fish", "fish", "org.bukkit.entity.Fish"),
    entity(
        "any fireball",
        "any fireballs",
        "org.bukkit.entity.Fireball",
    ),
    entity_since(
        "illager",
        "illagers",
        "org.bukkit.entity.Illager",
        (1, 12, 0),
    ),
    entity_since(
        "spellcaster",
        "spellcasters",
        "org.bukkit.entity.Spellcaster",
        (1, 12, 0),
    ),
    entity_since("raider", "raiders", "org.bukkit.entity.Raider", (1, 14, 0)),
];

#[cfg(test)]
fn legacy_entity_literal(
    text: &str,
    minecraft_version: Option<&str>,
) -> Option<LegacyEntityLiteral> {
    let text = crate::language::strip_indefinite_article(text.trim());
    legacy_entity_literal_without_article(text, minecraft_version)
}

fn legacy_entity_literal_without_article(
    text: &str,
    minecraft_version: Option<&str>,
) -> Option<LegacyEntityLiteral> {
    let minecraft_version = parse_version(minecraft_version?)?;
    LEGACY_ENTITY_LITERALS
        .iter()
        .copied()
        .find(|literal| {
            (text.eq_ignore_ascii_case(literal.singular)
                || text.eq_ignore_ascii_case(literal.plural_name))
                && minecraft_version >= literal.min_minecraft
        })
        .map(|literal| LegacyEntityLiteral {
            is_plural: text.eq_ignore_ascii_case(literal.plural_name),
            ..literal
        })
}

fn parse_version(version: &str) -> Option<(u16, u16, u16)> {
    let mut components = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse::<u16>().ok());
    Some((
        components.next()?,
        components.next()?,
        components.next().unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::legacy_entity_literal;

    #[test]
    fn legacy_fallback_preserves_common_entity_data_and_plurality() {
        let zombie = legacy_entity_literal("zombies", Some("1.12.2")).expect("zombie");
        assert_eq!(zombie.class_name, "org.bukkit.entity.Zombie");
        assert!(zombie.is_plural);

        let player = legacy_entity_literal("a player", Some("1.12.2")).expect("player");
        assert_eq!(player.class_name, "org.bukkit.entity.Player");
        assert!(!player.is_plural);
    }

    #[test]
    fn legacy_fallback_applies_minecraft_version_gates() {
        assert!(legacy_entity_literal("zombie", None).is_none());
        assert!(legacy_entity_literal("zombie", Some("not-a-version")).is_none());
        assert!(legacy_entity_literal("warden", Some("1.12.2")).is_none());
        assert!(legacy_entity_literal("warden", Some("1.19.0")).is_some());
        assert!(legacy_entity_literal("husk", Some("1.11.0")).is_some());
        assert!(legacy_entity_literal("husk", Some("1.10.2")).is_none());
    }
}
