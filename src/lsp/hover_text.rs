//! Presentation of SSG documentation; decoded text is never reparsed as HTML.
use lsp_types::MarkupKind;
use syntaxes::{Catalog, Documentation};

const LIMIT: usize = 32768;

/// Display lookup only: no inheritance, conversion or inferred type selection.
pub(super) fn type_name(catalog: &Catalog, class: &str) -> String {
    catalog
        .type_by_class_name(class)
        .map_or_else(|| class.to_owned(), |ty| ty.code_name.as_str().to_owned())
}

pub(super) fn render(
    title: &str,
    fields: &[(&str, String)],
    documentation: Option<&Documentation>,
    format: &MarkupKind,
) -> String {
    let body = documentation.map(|doc| documentation_body(doc, format));
    render_cached(title, fields, body.as_ref(), format)
}

/// Compose cached, already sanitized body text without decoding it again.
pub(super) fn render_cached(
    title: &str,
    fields: &[(&str, String)],
    body: Option<&DocumentationBody>,
    format: &MarkupKind,
) -> String {
    let markdown = *format == MarkupKind::Markdown;
    let mut output = if markdown {
        format!("**{}**\n\n", escape(&bounded(title, 1024)))
    } else {
        format!("{}\n", bounded(title, 1024))
    };
    // Implementation identities are useful, but should not precede user facts.
    for definition in [false, true] {
        if definition {
            if let Some(body) = body {
                for block in &body.blocks {
                    if !append(&mut output, block) {
                        return output;
                    }
                }
            }
            if fields.iter().any(|(label, _)| definition_field(label)) {
                if !append(&mut output, &section("Definition", markdown)) {
                    return output;
                }
            }
        }
        for (label, value) in fields {
            if definition_field(label) != definition {
                continue;
            }
            let value = bounded(value, 8192);
            let field = if markdown && matches!(*label, "Pattern" | "Template") {
                format!(
                    "\n**{}:**\n\n{}\n\n",
                    label,
                    code_block(&value, "skript-pattern")
                )
            } else if markdown {
                format!("- **{}:** {}\n", escape(label), inline_code(&value))
            } else {
                format!("{label}: {value}\n")
            };
            if !append(&mut output, &field) {
                return output;
            }
        }
    }
    output.trim_end().to_owned()
}

fn definition_field(label: &str) -> bool {
    matches!(
        label,
        "Addon"
            | "Java class"
            | "Qualified Java class"
            | "Value class"
            | "Snapshot value classes"
            | "Registration"
            | "Entry class"
            | "Structure registration"
            | "Declaration index"
            | "Since"
            | "Requires"
    )
}

fn section(label: &str, markdown: bool) -> String {
    if markdown {
        format!("\n\n**{label}**\n\n")
    } else {
        format!("\n{label}:\n")
    }
}

/// Immutable catalog bodies are prepared once, not once per proposal.
#[derive(Default)]
pub(super) struct DocumentationBody {
    blocks: Vec<String>,
    bytes: usize,
}
impl DocumentationBody {
    fn append(&mut self, block: &str) -> bool {
        if self.bytes + block.len() > LIMIT - 64 {
            self.blocks
                .push("\n\n… (additional content omitted)\n".into());
            return false;
        }
        self.bytes += block.len();
        self.blocks.push(block.into());
        true
    }
    #[cfg(test)]
    pub(super) fn text(text: &str) -> Self {
        let mut body = Self::default();
        body.append(text);
        body
    }
}

pub(super) fn documentation_body(
    documentation: &Documentation,
    format: &MarkupKind,
) -> DocumentationBody {
    let markdown = *format == MarkupKind::Markdown;
    let mut output = DocumentationBody::default();
    let mut raw = String::new();
    let mut truncated = false;
    for line in &documentation.description {
        if raw.len() + line.len() + 1 > LIMIT {
            truncated = true;
            break;
        }
        if !raw.is_empty() {
            raw.push('\n');
        }
        raw.push_str(line);
    }
    let description = html(&raw, markdown);
    if !description.is_empty()
        && !output.append(&format!(
            "{}{description}\n",
            section("Description", markdown)
        ))
    {
        return output;
    }
    if truncated && !output.append("\n… (description omitted beyond limit)\n") {
        return output;
    }
    let (examples, truncated) = examples(&documentation.examples);
    if !examples.is_empty() {
        if !output.append(&section("Examples", markdown)) {
            return output;
        }
        for example in examples {
            let block = if markdown {
                code_block(&bounded(&example, 16384), "skript")
            } else {
                example
            };
            if !output.append(&format!("\n{block}\n")) {
                return output;
            }
        }
    }
    if truncated {
        output.append("\n… (examples omitted beyond limit)\n");
    }
    output
}

/// Completion previews are pattern templates; only published examples are code.
pub(super) fn completion(
    title: &str,
    syntax: &str,
    template: Option<&str>,
    fields: &[(&str, String)],
    identity: &str,
    documentation: Option<&DocumentationBody>,
    format: &MarkupKind,
) -> lsp_types::Documentation {
    let mut fields = fields.to_vec();
    // Syntax and preview can be identical; show each distinct grammar once.
    if let Some(template) = template.filter(|template| *template != syntax) {
        fields.push(("Template", template.into()));
    }
    fields.push(("Pattern", syntax.into()));
    fields.push((
        "Validation",
        "Editing proposal; expected types and addon guards are not fully checked.".into(),
    ));
    fields.push(("Registration", identity.into()));
    markup(render_cached(title, &fields, documentation, format), format)
}

pub(super) fn markup(value: String, format: &MarkupKind) -> lsp_types::Documentation {
    if *format == MarkupKind::Markdown {
        lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        })
    } else {
        lsp_types::Documentation::String(value)
    }
}

fn append(output: &mut String, block: &str) -> bool {
    if output.len() + block.len() > LIMIT - 64 {
        output.push_str("\n\n… (additional content omitted)\n");
        false
    } else {
        output.push_str(block);
        true
    }
}

fn bounded(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn escape(text: &str) -> String {
    let mut output = String::new();
    for c in text.chars() {
        if "\\`*_{}[]<>#!|&".contains(c) {
            output.push('\\');
        }
        output.push(c);
    }
    output
}

fn longest_run(text: &str, character: char) -> usize {
    text.split(|c| c != character)
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn inline_code(text: &str) -> String {
    let text = text.replace(['\n', '\r'], " ");
    let fence = "`".repeat(longest_run(&text, '`') + 1);
    let padding = text.starts_with(['`', ' ']) || text.ends_with(['`', ' ']);
    if padding {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

/// Keep continuous visible text together even when the HTML scanner splits it
/// at unknown tags. Literal Markdown syntax uses code spans, since Neovim's
/// standard highlighter displays backslash escapes rather than concealing them.
#[derive(Default)]
struct Text {
    value: String,
    code: Vec<std::ops::Range<usize>>,
}

impl Text {
    fn push(&mut self, text: &str) {
        self.value.push_str(text);
    }

    fn push_code(&mut self, text: &str) {
        let start = self.value.len();
        self.value.push_str(text);
        if start != self.value.len() {
            self.code.push(start..self.value.len());
        }
    }

    fn flush(&mut self, output: &mut String, markdown: bool) {
        if !markdown {
            output.push_str(&self.value);
        } else {
            let mut cursor = 0;
            while cursor < self.value.len() {
                let start = cursor;
                let mut literal = false;
                while cursor < self.value.len() {
                    if let Some(code) = self.code.iter().find(|range| range.contains(&cursor)) {
                        cursor = code.end;
                        literal = true;
                        continue;
                    }
                    let rest = &self.value[cursor..];
                    let c = rest.chars().next().unwrap();
                    if c.is_whitespace() {
                        break;
                    }
                    if "\\`*_{}[]<>#!|&".contains(c) {
                        literal = true;
                    }
                    // Placeholders may contain spaces, but cannot swallow the
                    // next line or a known HTML structure (flush boundaries).
                    let close = match c {
                        '<' => Some('>'),
                        '[' => Some(']'),
                        _ => None,
                    };
                    if let Some(close) = close
                        && let Some(end) = rest.find(close)
                        && !rest[..end].contains(['\n', '\r'])
                    {
                        cursor += end + close.len_utf8();
                    } else {
                        cursor += c.len_utf8();
                    }
                }
                if cursor != start {
                    let text = &self.value[start..cursor];
                    output.push_str(&if literal {
                        inline_code(text)
                    } else {
                        text.to_owned()
                    });
                } else {
                    let c = self.value[cursor..].chars().next().unwrap();
                    output.push(c);
                    cursor += c.len_utf8();
                }
            }
        }
        self.value.clear();
        self.code.clear();
    }
}

pub(super) fn code_block(text: &str, language: &str) -> String {
    let fence = "`".repeat((longest_run(text, '`') + 1).max(3));
    format!("{fence}{language}\n{text}\n{fence}")
}

/// Single-line continuation items can extend a preceding section example.
/// A multiline array item is always its own example; top-level items and blank
/// separators retain their boundaries. No common indentation is stripped.
fn examples(items: &[String]) -> (Vec<String>, bool) {
    let mut result: Vec<String> = Vec::new();
    let mut pending = String::new();
    let mut processed = 0;
    let mut truncated = false;
    for item in items {
        if processed + item.len() + 1 > LIMIT {
            truncated = true;
            break;
        }
        processed += item.len() + 1;
        let decoded = html_escape::decode_html_entities(item);
        let text = decoded.replace("\r\n", "\n");
        let text = text.trim_end_matches(['\r', '\n']);
        let continuation = !text.contains('\n')
            && indentation(text) > indentation(&pending)
            && pending.lines().any(|line| line.trim_end().ends_with(':'));
        if continuation {
            pending.push('\n');
            pending.push_str(text);
            continue;
        }
        if !pending.is_empty() {
            result.push(std::mem::take(&mut pending));
        }
        if text.trim().is_empty() {
            continue;
        }
        if text.contains('\n') {
            result.push(text.to_owned());
        } else {
            pending.push_str(text);
        }
    }
    if !pending.is_empty() {
        result.push(pending);
    }
    (result, truncated)
}

fn indentation(text: &str) -> usize {
    text.bytes()
        .take_while(|c| matches!(c, b' ' | b'\t'))
        .fold(0, |column, c| {
            if c == b'\t' {
                (column / 4 + 1) * 4
            } else {
                column + 1
            }
        })
}

struct Tag<'a> {
    name: String,
    closing: bool,
    attributes: &'a str,
    end: usize,
}

fn tag(input: &str) -> Option<Tag<'_>> {
    if !input.starts_with('<') {
        return None;
    }
    let mut quote = None;
    let mut end = None;
    for (i, c) in input.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '>') => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    let mut inside = input[1..end].trim();
    let closing = inside.starts_with('/');
    if closing {
        inside = inside[1..].trim_start();
    }
    let length = inside.bytes().take_while(u8::is_ascii_alphanumeric).count();
    if length == 0 {
        return None;
    }
    let rest = &inside[length..];
    if rest.starts_with(|c: char| !c.is_ascii_whitespace() && c != '/') {
        return None;
    }
    Some(Tag {
        name: inside[..length].to_ascii_lowercase(),
        closing,
        attributes: rest,
        end: end + 1,
    })
}

fn href(mut attributes: &str) -> Option<String> {
    while !attributes.trim_start().is_empty() {
        attributes = attributes.trim_start();
        let length = attributes
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'-')
            .count();
        if length == 0 {
            break;
        }
        let key = &attributes[..length];
        attributes = attributes[length..].trim_start();
        if !attributes.starts_with('=') {
            continue;
        }
        attributes = attributes[1..].trim_start();
        let (value, rest) = if attributes.starts_with(['\'', '"']) {
            let quote = attributes.as_bytes()[0] as char;
            let end = attributes[1..].find(quote)? + 1;
            (&attributes[1..end], &attributes[end + 1..])
        } else {
            let end = attributes
                .find(char::is_whitespace)
                .unwrap_or(attributes.len());
            (&attributes[..end], &attributes[end..])
        };
        attributes = rest;
        if key.eq_ignore_ascii_case("href") {
            let decoded = html_escape::decode_html_entities(value);
            if decoded.chars().any(char::is_control) {
                return None;
            }
            let url = url::Url::parse(decoded.trim()).ok()?;
            if !matches!(url.scheme(), "http" | "https" | "mailto") {
                return None;
            }
            return Some(url.to_string().replace('<', "%3C").replace('>', "%3E"));
        }
    }
    None
}

fn newline(output: &mut String, count: usize) {
    if output.is_empty() {
        return;
    }
    let existing = output.chars().rev().take_while(|c| *c == '\n').count();
    for _ in existing..count {
        output.push('\n');
    }
}

// SSG embeds documentation cross-reference anchors inside code. Remove those
// wrappers before one decode, while keeping other tags as literal code (Skript
// itself uses angle-bracket formatting tags, including HTML-like spellings).
fn code_text(input: &str) -> String {
    let mut raw = String::new();
    let mut rest = input;
    while let Some(index) = rest.find('<') {
        raw.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(current) = tag(rest)
            && current.name == "a"
        {
            rest = &rest[current.end..];
        } else {
            raw.push('<');
            rest = &rest[1..];
        }
    }
    raw.push_str(rest);
    html_escape::decode_html_entities(&raw).into_owned()
}

/// Interpret only known documentation tags. Unknown tags remain literal text,
/// including Skript color tags. Entity-decoded text never enters this scanner.
fn html(input: &str, markdown: bool) -> String {
    let mut output = String::new();
    let mut text = Text::default();
    let mut rest = input;
    let mut lists: Vec<Option<usize>> = Vec::new();
    let mut links = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with("<!--") {
            if let Some(end) = rest.find("-->") {
                rest = &rest[end + 3..];
                continue;
            }
        }
        if rest.starts_with('<')
            && let Some(current) = tag(rest)
        {
            let name = current.name.as_str();
            if !current.closing && matches!(name, "code" | "pre" | "script" | "style" | "iframe") {
                let close = format!("</{name}>");
                let after = &rest[current.end..];
                if let Some(end) = after.to_ascii_lowercase().find(&close) {
                    if matches!(name, "code" | "pre") {
                        let mut body = &after[..end];
                        if name == "pre"
                            && let Some(code) = tag(body)
                            && code.name == "code"
                            && !code.closing
                            && body
                                .get(body.len().saturating_sub(7)..)
                                .is_some_and(|end| end.eq_ignore_ascii_case("</code>"))
                        {
                            body = &body[code.end..body.len() - 7];
                        }
                        let body = code_text(body);
                        if name == "pre" {
                            text.flush(&mut output, markdown);
                            newline(&mut output, 2);
                            output.push_str(&if markdown {
                                code_block(&body, "")
                            } else {
                                body
                            });
                            newline(&mut output, 2);
                        } else {
                            text.push_code(&body);
                        }
                    }
                    rest = &after[end + close.len()..];
                    continue;
                }
            }
            let url = (name == "a" && !current.closing)
                .then(|| href(current.attributes))
                .flatten();
            let linked = if current.closing {
                links
                    .last()
                    .is_some_and(|(_, url): &(usize, Option<String>)| url.is_some())
            } else {
                url.is_some()
            };
            if matches!(
                name,
                "strong" | "b" | "em" | "i" | "p" | "div" | "br" | "ul" | "ol" | "li"
            ) || name == "a" && linked
            {
                text.flush(&mut output, markdown);
            }
            let recognized = match name {
                "strong" | "b" => {
                    if markdown {
                        output.push_str("**");
                    }
                    true
                }
                "em" | "i" => {
                    if markdown {
                        output.push('*');
                    }
                    true
                }
                "p" | "div" => {
                    newline(&mut output, 2);
                    true
                }
                "br" => {
                    if markdown {
                        output.push_str("  ");
                    }
                    output.push('\n');
                    true
                }
                "ul" | "ol" => {
                    newline(&mut output, 1);
                    if current.closing {
                        lists.pop();
                    } else if lists.len() < 32 {
                        lists.push((name == "ol").then_some(1));
                    }
                    true
                }
                "li" => {
                    newline(&mut output, 1);
                    if !current.closing {
                        output.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                        match lists.last_mut() {
                            Some(Some(number)) => {
                                output.push_str(&format!("{number}. "));
                                *number += 1;
                            }
                            _ => output.push_str("- "),
                        }
                    }
                    true
                }
                "a" => {
                    if current.closing {
                        if let Some((start, Some(url))) = links.pop() {
                            let label = output.split_off(start);
                            output.push_str(&if markdown {
                                format!("[{label}](<{url}>)")
                            } else {
                                format!("{label} ({url})")
                            });
                        }
                    } else {
                        links.push((output.len(), url));
                    }
                    true
                }
                "span" => true,
                _ => false,
            };
            if recognized {
                rest = &rest[current.end..];
                continue;
            }
            let literal = html_escape::decode_html_entities(&rest[..current.end]);
            text.push(&literal);
            rest = &rest[current.end..];
            continue;
        }
        let length = if rest.starts_with('<') {
            rest[1..].find('<').map_or(rest.len(), |i| i + 1)
        } else {
            rest.find('<').unwrap_or(rest.len())
        };
        let decoded = html_escape::decode_html_entities(&rest[..length]);
        text.push(&decoded);
        rest = &rest[length..];
    }
    text.flush(&mut output, markdown);
    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_is_tokenized_before_one_entity_decode_and_literal_tags_survive() {
        let input = "<p><strong>Players:</strong> <em>custom</em> &lt;red&gt; <gold> &amp;lt;x&amp;gt; &#x1F600; &copy;</p>";
        assert_eq!(
            html(input, false),
            "Players: custom <red> <gold> &lt;x&gt; 😀 ©"
        );
        assert_eq!(
            html(input, true),
            "**Players:** *custom* `<red>` `<gold>` `&lt;x&gt;` 😀 ©"
        );
    }

    #[test]
    fn unicode_at_scanner_boundaries_is_preserved() {
        for (input, expected) in [
            ("日本😀", "日本😀"),
            ("<em>日本語</em>", "*日本語*"),
            ("<😀", "`<😀`"),
        ] {
            assert_eq!(html(input, true), expected);
        }
        assert_eq!(
            html("<pre>日本語<red>😀</pre>", true),
            "```\n日本語<red>😀\n```"
        );
        assert_eq!(
            html(
                "<code>set <a href='#name'>name of &lt;item&gt;</a> to &lt;text&gt;</code>",
                true
            ),
            "`set name of <item> to <text>`"
        );
        assert_eq!(
            html("<code>send \"<b><i><red> &amp;lt;\"</code>", true),
            "`send \"<b><i><red> &lt;\"`"
        );
    }

    #[test]
    fn prose_preserves_literal_backslashes_placeholders_and_entities_without_escape_slashes() {
        let input = r#"Path C:\Skript\scripts\*.sk \\server\share \n &lt;item type&gt; [&lt;script&gt;.sk] &amp;lt;x&amp;gt; 日本😀"#;
        assert_eq!(
            html(input, true),
            r#"Path `C:\Skript\scripts\*.sk` `\\server\share` `\n` `<item type>` `[<script>.sk]` `&lt;x&gt;` 日本😀"#
        );
        assert_eq!(
            html(input, false),
            r#"Path C:\Skript\scripts\*.sk \\server\share \n <item type> [<script>.sk] &lt;x&gt; 日本😀"#
        );
        assert_eq!(
            html("[<script>.sk] <x><y> <code>a`b</code><z>", true),
            "`[<script>.sk]` `<x><y>` ``a`b<z>``"
        );
        assert_eq!(
            html("[<span><x></span>.sk] <x><a href='#ref'><y></a><z>", true),
            "`[<x>.sk]` `<x><y><z>`"
        );
        assert_eq!(
            html("before <code>a b</code><code>c</code> after", true),
            "before `a bc` after"
        );
        assert_eq!(
            html("<%color of ...%>this text is colored!", true),
            "`<%color of ...%>this` text is `colored!`"
        );
        assert_eq!(html("<x\nnext paragraph", true), "`<x`\nnext paragraph");
        assert_eq!(
            html(
                "<strong>&lt;player&gt;</strong> <em>C:\\path</em> <a href='https://example.test'>&lt;item type&gt;</a>",
                true
            ),
            "**`<player>`** *`C:\\path`* [`<item type>`](<https://example.test/>)"
        );
    }

    #[test]
    fn link_boundaries_remain_valid_across_paragraphs_and_nested_links() {
        for input in [
            "x          <a href='https://example.test'><p></p></a>",
            "日本  <a href='https://example.test'><p>😀</p><pre>x</pre></a>",
            "<a href='https://example.test'>outer<a href='https://nested.test'>inner</a></a>",
        ] {
            assert!(!html(input, true).is_empty());
        }
    }

    #[test]
    fn paragraphs_lists_links_and_code_are_safe_and_readable() {
        let input = "<p>Intro<br>next</p><ol><li>one</li><li><b>two</b><ul><li>nested</li></ul></li></ol><a href='https://example.test/?a=1&amp;b=2'>safe</a> <a href='javascript:alert(1)'>bad</a> <a href='#local'>local</a><pre><code>send `x` &lt;red&gt;\n    # ```</code></pre><script>evil()</script>";
        let text = html(input, true);
        assert!(text.contains("Intro  \nnext\n\n1. one\n2. **two**\n  - nested"));
        assert!(text.contains("[safe](<https://example.test/?a=1&b=2>) bad local"));
        assert!(text.contains("````\nsend `x` <red>\n    # ```\n````"));
        assert!(
            !text.contains("javascript:") && !text.contains("evil()") && !text.contains("<code>")
        );
        assert_eq!(html("<code>a`b &lt;red&gt;</code>", true), "``a`b <red>``");
    }

    #[test]
    fn examples_keep_array_boundaries_and_join_indented_section_lines_only() {
        let items = [
            "on join:",
            "\tpermission",
            "\tmessage \"&lt;red&gt;\"",
            "separate",
            "  unrelated",
            "",
            "  first\n    second\n",
            "next\n\tchild",
        ];
        assert_eq!(
            examples(&items.map(str::to_owned)).0,
            vec![
                "on join:\n\tpermission\n\tmessage \"<red>\"",
                "separate",
                "  unrelated",
                "  first\n    second",
                "next\n\tchild"
            ]
        );
        assert_eq!(
            examples(&["\ton load:".into(), "\tset {_x} to 1".into()]).0,
            vec!["\ton load:", "\tset {_x} to 1"]
        );
    }

    #[test]
    fn oversized_documentation_is_bounded_before_decode_and_never_splits_fences() {
        let doc = Documentation {
            description: vec!["x".repeat(LIMIT + 1)],
            examples: vec!["on load:\n\tsend \"😀\"".into(), "x".repeat(LIMIT + 1)],
            ..Default::default()
        };
        let text = render("Title", &[], Some(&doc), &MarkupKind::Markdown);
        assert!(
            text.len() < LIMIT
                && text.contains("description omitted")
                && text.contains("examples omitted")
        );
        assert!(text.contains("```skript\non load:\n\tsend \"😀\"\n```"));
    }

    #[test]
    fn patterns_and_examples_have_distinct_safe_code_fences() {
        let documentation = Documentation {
            examples: vec!["send \"%{_name}%\" # example".into()],
            ..Default::default()
        };
        let pattern = "[send] %strings% [to %-players%] \x60\x60\x60";
        let text = render(
            "Effect",
            &[("Pattern", pattern.into()), ("Addon", "Skript".into())],
            Some(&documentation),
            &MarkupKind::Markdown,
        );
        assert!(text.contains("**Pattern:**\n\n\x60\x60\x60\x60skript-pattern\n[send] %strings% [to %-players%] \x60\x60\x60\n\x60\x60\x60\x60"));
        assert!(text.contains("\x60\x60\x60skript\nsend \"%{_name}%\" # example\n\x60\x60\x60"));
        assert!(text.contains("- **Addon:** \x60Skript\x60"));
        let plain = render(
            "Effect",
            &[("Pattern", pattern.into())],
            Some(&documentation),
            &MarkupKind::PlainText,
        );
        assert!(plain.contains(&format!("Pattern: {pattern}")));
        assert!(!plain.contains("skript-pattern"));
    }

    #[test]
    fn completion_markup_preserves_literal_grammar_and_plaintext_information() {
        let grammar = "[go] %entities% (to|towards) %locations%";
        let lsp_types::Documentation::MarkupContent(markdown) = completion(
            "Effect / [Addon]",
            grammar,
            Some("go %entities% to %locations%"),
            &[],
            "reg\x60id",
            None,
            &MarkupKind::Markdown,
        ) else {
            panic!("negotiated markdown");
        };
        assert_eq!(markdown.kind, MarkupKind::Markdown);
        assert!(
            markdown
                .value
                .contains(&code_block(grammar, "skript-pattern"))
        );
        assert!(markdown.value.contains("**Template:**\n\n\x60\x60\x60skript-pattern\ngo %entities% to %locations%\n\x60\x60\x60"));
        assert!(markdown.value.contains("**Effect / \\[Addon\\]**"));
        assert!(
            markdown
                .value
                .contains("**Registration:** \x60\x60reg\x60id\x60\x60")
        );
        let lsp_types::Documentation::String(plain) = completion(
            "Effect",
            grammar,
            None,
            &[],
            "reg",
            Some(&DocumentationBody::text("Description")),
            &MarkupKind::PlainText,
        ) else {
            panic!("plain negotiation")
        };
        for value in [
            grammar,
            "Effect",
            "Description",
            "Validation:",
            "Registration: reg",
        ] {
            assert!(plain.contains(value), "{plain}");
        }
        assert!(!plain.contains("**") && !plain.contains("skript-pattern"));
    }

    #[test]
    fn all_examples_share_one_heading_and_fences_cannot_be_closed_by_code() {
        let doc = Documentation {
            examples: vec![
                "on join:".into(),
                "\tsend \"<red>\"".into(),
                "next ```".into(),
                "fourth".into(),
            ],
            ..Default::default()
        };
        let text = render(
            "Title [x]",
            &[("Java class", "a.b.C`D".into())],
            Some(&doc),
            &MarkupKind::Markdown,
        );
        assert_eq!(text.matches("**Examples**").count(), 1);
        assert!(text.contains("```skript\non join:\n\tsend \"<red>\"\n```"));
        assert!(text.contains("````skript\nnext ```\n````"));
        assert!(text.contains("fourth") && text.contains("``a.b.C`D``"));
    }

    #[test]
    fn shared_hierarchy_keeps_cached_body_once_and_definition_last() {
        let doc = Documentation {
            description: vec!["A &lt;value&gt;".into()],
            examples: vec!["send 1".into()],
            ..Default::default()
        };
        let body = documentation_body(&doc, &MarkupKind::Markdown);
        let fields = [
            ("Return type", "dynamic (snapshot Partial: number)".into()),
            ("Registration", "a.b.C:0".into()),
        ];
        let text = render_cached(
            "Expression: Example",
            &fields,
            Some(&body),
            &MarkupKind::Markdown,
        );
        assert!(text.starts_with("**Expression: Example**"));
        assert!(text.contains("dynamic (snapshot Partial: number)"));
        assert_eq!(text.matches("**Description**").count(), 1);
        assert_eq!(text.matches("**Examples**").count(), 1);
        assert!(text.contains("A `<value>`"));
        assert!(text.find("**Examples**").unwrap() < text.find("**Definition**").unwrap());
        assert!(text.contains(&code_block("send 1", "skript")));
        let lsp_types::Documentation::MarkupContent(completion) = completion(
            "Expression: Example",
            "value",
            Some("value"),
            &fields,
            "registration",
            Some(&body),
            &MarkupKind::Markdown,
        ) else {
            panic!()
        };
        assert!(!completion.value.contains("**Template:**"));
        assert_eq!(completion.value.matches("**Examples**").count(), 1);
        assert!(completion.value.len() < LIMIT);
    }

    #[test]
    fn cached_body_keeps_earlier_blocks_when_composed_limit_is_reached() {
        let doc = Documentation {
            description: vec!["kept description".into()],
            examples: vec!["send 1".into(), "x".repeat(16384)],
            ..Default::default()
        };
        let body = documentation_body(&doc, &MarkupKind::Markdown);
        let text = render_cached(
            "Title",
            &[
                ("One", "a".repeat(8192)),
                ("Two", "b".repeat(8192)),
                ("Three", "c".repeat(8192)),
            ],
            Some(&body),
            &MarkupKind::Markdown,
        );
        assert!(text.contains("kept description"));
        assert!(text.contains(&code_block("send 1", "skript")));
        assert!(!text.contains(&"x".repeat(100)));
        assert!(text.contains("additional content omitted") && text.len() < LIMIT);
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("\x60\x60\x60"))
                .count(),
            2
        );
    }
}
