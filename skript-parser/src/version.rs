//! Release numbers used to select parser capabilities from runtime metadata.

/// A known Skript release, without prerelease/build labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkriptVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SkriptVersion {
    /// Reads `major.minor[.patch]` with an optional `-label` or `+build` suffix.
    /// Missing, overflowing, or misplaced numeric components are not guessed.
    pub fn parse(value: &str) -> Option<Self> {
        fn number(value: &str) -> Option<(u32, &str)> {
            let end = value
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(value.len());
            Some((value.get(..end)?.parse().ok()?, &value[end..]))
        }
        let (major, rest) = number(value.trim())?;
        let (minor, rest) = number(rest.strip_prefix('.')?)?;
        let (patch, rest) = match rest.strip_prefix('.') {
            Some(rest) => number(rest)?,
            None => (0, rest),
        };
        if !rest.is_empty() {
            let label = rest.strip_prefix('-').or_else(|| rest.strip_prefix('+'))?;
            if label.is_empty()
                || !label.starts_with(|c: char| c.is_ascii_alphanumeric())
                || !label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
            {
                return None;
            }
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_components_are_anchored_and_never_skipped() {
        for (text, expected) in [
            ("2.8", (2, 8, 0)),
            ("2.9.0", (2, 9, 0)),
            ("2.15.4-beta1+build.9", (2, 15, 4)),
            (" 2.2-dev36 ", (2, 2, 0)),
            ("3.0.1", (3, 0, 1)),
        ] {
            let version = SkriptVersion::parse(text).unwrap();
            assert_eq!((version.major, version.minor, version.patch), expected);
        }
        for text in [
            "",
            "unknown",
            "2",
            "v2.8",
            "build7-2.8",
            "2.x.8",
            "2.8.bad",
            "2.8.",
            "2.8-",
            "2.8 bad",
            "4294967296.2.8",
            "2.4294967296.8",
            "2.8.4294967296",
        ] {
            assert_eq!(SkriptVersion::parse(text), None, "{text:?}");
        }
    }
}
