use crate::addon_syntax_list::entity::internal_utils::{
    Entries, define_syntax_struct, entries_parser,
};

define_syntax_struct!(Structure {
    entries: Option<Entries> = entries_parser,
});

// 構造がSectionと全く同じため、テストは省略
