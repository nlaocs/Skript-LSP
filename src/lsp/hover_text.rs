//! Presentation of SSG documentation; decoded text is never reparsed as HTML.
use lsp_types::MarkupKind;
use syntaxes::Documentation;

const LIMIT: usize = 32768;

pub(super) fn render(
    title: &str,
    fields: &[(&str, String)],
    documentation: Option<&Documentation>,
    format: &MarkupKind,
) -> String {
    let markdown = *format == MarkupKind::Markdown;
    let mut output = if markdown {
        format!("### {}\n\n", escape(&bounded(title, 1024)))
    } else {
        format!("{}\n", bounded(title, 1024))
    };
    for (label, value) in fields {
        let value = bounded(value, 8192);
        let field = if markdown {
            format!("- **{}:** {}\n", escape(label), inline_code(&value))
        } else {
            format!("{label}: {value}\n")
        };
        if !append(&mut output, &field) {
            return output;
        }
    }
    if let Some(documentation) = documentation {
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
        if !description.is_empty() && !append(&mut output, &format!("\n{description}\n")) {
            return output;
        }
        if truncated {
            if !append(&mut output, "\n… (description omitted beyond limit)\n") {
                return output;
            }
        }
        let (examples, truncated) = examples(&documentation.examples);
        if !examples.is_empty() {
            if !append(
                &mut output,
                if markdown {
                    "\n### Examples\n"
                } else {
                    "\nExamples:\n"
                },
            ) {
                return output;
            }
            for example in examples {
                let block = if markdown {
                    code_block(&bounded(&example, 16384), "skript")
                } else {
                    example
                };
                if !append(&mut output, &format!("\n{block}\n")) {
                    return output;
                }
            }
        }
        if truncated {
            append(&mut output, "\n… (examples omitted beyond limit)\n");
        }
    }
    output.trim_end().to_owned()
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

fn code_block(text: &str, language: &str) -> String {
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
                            newline(&mut output, 2);
                            output.push_str(&if markdown {
                                code_block(&body, "")
                            } else {
                                body
                            });
                            newline(&mut output, 2);
                        } else {
                            output.push_str(&if markdown { inline_code(&body) } else { body });
                        }
                    }
                    rest = &after[end + close.len()..];
                    continue;
                }
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
                        links.push((output.len(), href(current.attributes)));
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
            output.push_str(&if markdown {
                escape(&literal)
            } else {
                literal.into_owned()
            });
            rest = &rest[current.end..];
            continue;
        }
        let length = if rest.starts_with('<') {
            rest[1..].find('<').map_or(rest.len(), |i| i + 1)
        } else {
            rest.find('<').unwrap_or(rest.len())
        };
        let decoded = html_escape::decode_html_entities(&rest[..length]);
        output.push_str(&if markdown {
            escape(&decoded)
        } else {
            decoded.into_owned()
        });
        rest = &rest[length..];
    }
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
            "**Players:** *custom* \\<red\\> \\<gold\\> \\&lt;x\\&gt; 😀 ©"
        );
    }

    #[test]
    fn unicode_at_scanner_boundaries_is_preserved() {
        for (input, expected) in [
            ("日本😀", "日本😀"),
            ("<em>日本語</em>", "*日本語*"),
            ("<😀", "\\<😀"),
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
        assert_eq!(text.matches("### Examples").count(), 1);
        assert!(text.contains("```skript\non join:\n\tsend \"<red>\"\n```"));
        assert!(text.contains("````skript\nnext ```\n````"));
        assert!(text.contains("fourth") && text.contains("``a.b.C`D``"));
    }
}
