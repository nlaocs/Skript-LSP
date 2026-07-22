mod pattern;
mod plural;

// event, condition, effect, expression, section, structureはここに分類される
// (typeはまた違うので注意)
pub mod syntax {
    use super::{pattern, plural};
    pub use pattern::*;
    pub use plural::*;
}
