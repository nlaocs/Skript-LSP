mod function_pattern;
mod pattern;

pub mod function {
    use super::function_pattern;
    pub use function_pattern::*;
}

// event, condition, effect, expression, section, structureはここに分類される
// (typeはまた違うので注意)
pub mod syntax {
    use super::pattern;
    pub use pattern::*;
}
