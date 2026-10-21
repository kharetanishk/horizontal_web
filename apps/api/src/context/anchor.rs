//! Branch anchors: validation against the parent's raw markdown and the prompt block
//! rendered from them (docs/ARCHITECTURE.md, Context builder step 6; AC-6.1, AC-7.1).
//!
//! Offsets are UTF-16 code units (JS string indexing). They only ever index the
//! `encode_utf16()` buffer, never a Rust `str`.
//!
//! A `code` anchor's `start..end` covers the whole block, opening fence first. Line
//! numbers count from that fence: line 1 is the first line inside the block.

use crate::http::error::ApiError;
use crate::types::{Anchor, ErrorCode};

/// The selection itself is never truncated, so it must fit well inside the slot.
pub const SELECTION_MAX_TOKENS: usize = 1_000;
/// Anchor slot in the prompt budget, wrapper tags included.
pub const ANCHOR_SLOT_TOKENS: usize = 1_500;

const TRIMMED: &str = "…";

fn invalid() -> ApiError {
    ApiError::Code(ErrorCode::InvalidAnchor)
}

/// `content` split at the UTF-16 range into (before, selection, after).
fn split_utf16(content: &str, start: u32, end: u32) -> Result<(String, String, String), ApiError> {
    let u: Vec<u16> = content.encode_utf16().collect();
    let (start, end) = (start as usize, end as usize);
    if start > end || end > u.len() {
        return Err(invalid());
    }
    // A range that splits a surrogate pair fails here, so before/after are whole too.
    let selection = String::from_utf16(&u[start..end]).map_err(|_| invalid())?;
    Ok((
        String::from_utf16_lossy(&u[..start]),
        selection,
        String::from_utf16_lossy(&u[end..]),
    ))
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// A code block's lines: opening fence, body, and the closing fence if the slice has one.
struct Block<'a> {
    fence: &'a str,
    body: Vec<&'a str>,
    closing: Option<&'a str>,
}

fn parse_block(block: &str) -> Option<Block<'_>> {
    let mut lines: Vec<&str> = block.lines().collect();
    let fence = *lines.first().filter(|l| is_fence(l))?;
    let closing =
        (lines.len() > 1 && is_fence(lines[lines.len() - 1])).then(|| lines.pop().unwrap());
    lines.remove(0);
    Some(Block {
        fence,
        body: lines,
        closing,
    })
}

/// Checks `anchor` against the parent's `content`; any mismatch is 422 `invalid_anchor`.
/// `count` is the mode tokenizer.
pub fn validate(
    anchor: &Anchor,
    content: &str,
    count: impl Fn(&str) -> usize,
) -> Result<(), ApiError> {
    match anchor {
        Anchor::None => Ok(()),
        Anchor::Text { start, end, text } => {
            let (_, selection, _) = split_utf16(content, *start, *end)?;
            if selection.is_empty() || selection != *text || count(text) > SELECTION_MAX_TOKENS {
                return Err(invalid());
            }
            Ok(())
        }
        Anchor::Code {
            start,
            end,
            start_line,
            end_line,
            text,
        } => {
            let (before, block, _) = split_utf16(content, *start, *end)?;
            if !(before.is_empty() || before.ends_with('\n')) {
                return Err(invalid());
            }
            let b = parse_block(&block).ok_or_else(invalid)?;
            let (s, e) = (*start_line as usize, *end_line as usize);
            if s == 0 || s > e || e > b.body.len() || b.body[s - 1..e].join("\n") != *text {
                return Err(invalid());
            }
            if count(text) > SELECTION_MAX_TOKENS {
                return Err(invalid());
            }
            Ok(())
        }
    }
}

/// Grows context outward from the selection, nearest unit first on each side, while the
/// rendered block fits the slot. The selection is always kept. `before` is nearest-last.
/// ponytail: re-counts the whole block per added unit (quadratic in kept units); only
/// reached when the full context is over the slot. Sum per-unit counts if it shows up in profiles.
fn fit(
    before: &[&str],
    after: &[&str],
    render: impl Fn(&[&str], &[&str]) -> String,
    count: impl Fn(&str) -> usize,
) -> String {
    let full = render(before, after);
    if count(&full) <= ANCHOR_SLOT_TOKENS {
        return full;
    }
    let (mut b, mut a) = (0, 0);
    let (mut grow_b, mut grow_a) = (true, true);
    while grow_b || grow_a {
        if grow_b {
            grow_b = b < before.len()
                && count(&render(&before[before.len() - b - 1..], &after[..a]))
                    <= ANCHOR_SLOT_TOKENS;
            b += grow_b as usize;
        }
        if grow_a {
            grow_a = a < after.len()
                && count(&render(&before[before.len() - b..], &after[..a + 1]))
                    <= ANCHOR_SLOT_TOKENS;
            a += grow_a as usize;
        }
    }
    render(&before[before.len() - b..], &after[..a])
}

fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '\n' | '。' | '！' | '？')
}

/// The anchor block for the prompt: `text` is the highlight inside its surrounding
/// sentence, `code` is the block with the selected lines marked; empty for `none`.
/// The selection is marked with `<selected>` tags. Deterministic: same input, same bytes.
pub fn render(
    anchor: &Anchor,
    content: &str,
    count: impl Fn(&str) -> usize,
) -> Result<String, ApiError> {
    validate(anchor, content, &count)?;
    match anchor {
        Anchor::None => Ok(String::new()),
        Anchor::Text { start, end, .. } => {
            let (before, selection, after) = split_utf16(content, *start, *end)?;
            let lead = before.rfind(is_sentence_end).map_or(0, |i| {
                i + before[i..].chars().next().map_or(0, char::len_utf8)
            });
            let lead = before[lead..].trim_start();
            let tail = after.find(is_sentence_end).map_or(after.len(), |i| {
                i + after[i..].chars().next().map_or(0, char::len_utf8)
            });
            let tail = after[..tail].trim_end();
            let lead: Vec<&str> = lead.split_inclusive(char::is_whitespace).collect();
            let tail: Vec<&str> = tail.split_inclusive(char::is_whitespace).collect();
            Ok(fit(
                &lead,
                &tail,
                |l, t| {
                    format!(
                        "<anchor>\n{}{}<selected>{selection}</selected>{}{}\n</anchor>",
                        if l.len() < lead.len() { TRIMMED } else { "" },
                        l.concat(),
                        t.concat(),
                        if t.len() < tail.len() { TRIMMED } else { "" },
                    )
                },
                &count,
            ))
        }
        Anchor::Code {
            start,
            end,
            start_line,
            end_line,
            text,
        } => {
            let (_, block, _) = split_utf16(content, *start, *end)?;
            let b = parse_block(&block).ok_or_else(invalid)?;
            let (s, e) = (*start_line as usize, *end_line as usize);
            let (above, below) = (&b.body[..s - 1], &b.body[e..]);
            Ok(fit(
                above,
                below,
                |l, t| {
                    let mut out = format!("<anchor>\n{}\n", b.fence);
                    if l.len() < above.len() {
                        out += &format!("{TRIMMED}\n");
                    }
                    for line in l {
                        out += &format!("{line}\n");
                    }
                    out += &format!("<selected>\n{text}\n</selected>\n");
                    for line in t {
                        out += &format!("{line}\n");
                    }
                    if t.len() < below.len() {
                        out += &format!("{TRIMMED}\n");
                    }
                    if let Some(closing) = b.closing {
                        out += &format!("{closing}\n");
                    }
                    out + "</anchor>"
                },
                &count,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Test tokenizer: one token per whitespace-separated word.
    fn words(s: &str) -> usize {
        s.split_whitespace().count()
    }

    fn u16_len(s: &str) -> u32 {
        s.encode_utf16().count() as u32
    }

    fn text_anchor(content: &str, sel: &str) -> Anchor {
        let start = u16_len(&content[..content.find(sel).unwrap()]);
        Anchor::Text {
            start,
            end: start + u16_len(sel),
            text: sel.into(),
        }
    }

    fn is_invalid(r: Result<(), ApiError>) -> bool {
        matches!(r, Err(ApiError::Code(ErrorCode::InvalidAnchor)))
    }

    proptest! {
        /// AC-6.1: offsets taken JS-style from any Unicode string round-trip; any other text is rejected.
        #[test]
        fn text_anchor_round_trip(content in "\\PC{0,40}", a in 0usize..=40, b in 0usize..=40, other in "\\PC{1,5}") {
            let chars: Vec<char> = content.chars().collect();
            let (a, b) = (a.min(chars.len()), b.min(chars.len()));
            let (a, b) = (a.min(b), a.max(b));
            let prefix: String = chars[..a].iter().collect();
            let text: String = chars[a..b].iter().collect();
            let start = u16_len(&prefix);
            let end = start + u16_len(&text);
            let ok = validate(&Anchor::Text { start, end, text: text.clone() }, &content, words);
            prop_assert_eq!(ok.is_ok(), !text.is_empty());
            let wrong = format!("{text}{other}");
            let rejected = is_invalid(validate(&Anchor::Text { start, end, text: wrong }, &content, words));
            prop_assert!(rejected);
        }

        /// Arbitrary offsets are accepted exactly when the UTF-16 slice decodes to `text`.
        #[test]
        fn text_anchor_arbitrary_offsets(content in "\\PC{0,20}", start in 0u32..50, end in 0u32..50, text in "\\PC{1,5}") {
            let u: Vec<u16> = content.encode_utf16().collect();
            let expect = start <= end
                && (end as usize) <= u.len()
                && String::from_utf16(&u[start as usize..end as usize]).ok().as_deref() == Some(text.as_str());
            let got = validate(&Anchor::Text { start, end, text }, &content, words).is_ok();
            prop_assert_eq!(got, expect);
        }
    }

    #[test]
    fn text_anchor_rejects_split_surrogate_and_out_of_range() {
        let content = "a😀b";
        // 😀 is two UTF-16 units at 1..3; 1..2 is half a pair.
        assert!(
            validate(
                &Anchor::Text {
                    start: 1,
                    end: 3,
                    text: "😀".into()
                },
                content,
                words
            )
            .is_ok()
        );
        assert!(is_invalid(validate(
            &Anchor::Text {
                start: 1,
                end: 2,
                text: "\u{FFFD}".into()
            },
            content,
            words
        )));
        assert!(is_invalid(validate(
            &Anchor::Text {
                start: 3,
                end: 5,
                text: "b".into()
            },
            content,
            words
        )));
        assert!(is_invalid(validate(
            &Anchor::Text {
                start: 2,
                end: 1,
                text: "".into()
            },
            content,
            words
        )));
    }

    #[test]
    fn selection_over_limit_is_invalid() {
        let sel = "w ".repeat(SELECTION_MAX_TOKENS + 1);
        let content = format!("Intro. {sel}end.");
        assert!(is_invalid(validate(
            &text_anchor(&content, &sel),
            &content,
            words
        )));
        let sel = "w ".repeat(SELECTION_MAX_TOKENS);
        assert!(validate(&text_anchor(&content, &sel), &content, words).is_ok());

        let body = (0..SELECTION_MAX_TOKENS + 1)
            .map(|_| "x")
            .collect::<Vec<_>>()
            .join("\n");
        let block = format!("```\n{body}\n```");
        let a = Anchor::Code {
            start: 0,
            end: u16_len(&block),
            start_line: 1,
            end_line: SELECTION_MAX_TOKENS as u32 + 1,
            text: body.clone(),
        };
        assert!(is_invalid(validate(&a, &block, words)));
    }

    const CODE_DOC: &str = "Here is the code:\n\n```rust\nfn main() {\n    let a = 1;\n    let b = 2;\n    println!(\"{}\", a + b);\n}\n```\n\nDone.";

    fn code_anchor(start_line: u32, end_line: u32, text: &str) -> Anchor {
        let start = CODE_DOC.find("```rust").unwrap();
        let end = CODE_DOC.rfind("```").unwrap() + 3;
        Anchor::Code {
            start: u16_len(&CODE_DOC[..start]),
            end: u16_len(&CODE_DOC[..end]),
            start_line,
            end_line,
            text: text.into(),
        }
    }

    #[test]
    fn code_anchor_validation() {
        let sel = "    let a = 1;\n    let b = 2;";
        assert!(validate(&code_anchor(2, 3, sel), CODE_DOC, words).is_ok());
        // Wrong lines, empty or out-of-body ranges.
        assert!(is_invalid(validate(
            &code_anchor(1, 2, sel),
            CODE_DOC,
            words
        )));
        assert!(is_invalid(validate(
            &code_anchor(0, 1, "fn main() {"),
            CODE_DOC,
            words
        )));
        assert!(is_invalid(validate(
            &code_anchor(3, 2, sel),
            CODE_DOC,
            words
        )));
        assert!(is_invalid(validate(
            &code_anchor(5, 6, "}\n```"),
            CODE_DOC,
            words
        )));
        // Slice not starting at a fence line.
        let Anchor::Code { start, end, .. } = code_anchor(2, 3, sel) else {
            unreachable!()
        };
        let shifted = Anchor::Code {
            start: start + 3,
            end,
            start_line: 1,
            end_line: 2,
            text: "fn main() {\n    let a = 1;".into(),
        };
        assert!(is_invalid(validate(&shifted, CODE_DOC, words)));
        // Fence-looking text that is not at a line start.
        let doc = "say ```\nx\n```";
        let a = Anchor::Code {
            start: 4,
            end: u16_len(doc),
            start_line: 1,
            end_line: 1,
            text: "x".into(),
        };
        assert!(is_invalid(validate(&a, doc, words)));
    }

    #[test]
    fn code_render_marks_selection_and_keeps_block() {
        let sel = "    let a = 1;\n    let b = 2;";
        let out = render(&code_anchor(2, 3, sel), CODE_DOC, words).unwrap();
        assert_eq!(
            out,
            "<anchor>\n```rust\nfn main() {\n<selected>\n    let a = 1;\n    let b = 2;\n</selected>\n    println!(\"{}\", a + b);\n}\n```\n</anchor>"
        );
        assert_eq!(
            out,
            render(&code_anchor(2, 3, sel), CODE_DOC, words).unwrap()
        );
    }

    #[test]
    fn code_render_trims_nearest_first_never_the_selection() {
        let body: Vec<String> = (1..=2000).map(|i| format!("line{i}")).collect();
        let doc = format!("```\n{}\n```", body.join("\n"));
        let sel = body[999..1100].join("\n");
        let a = Anchor::Code {
            start: 0,
            end: u16_len(&doc),
            start_line: 1000,
            end_line: 1100,
            text: sel.clone(),
        };
        let out = render(&a, &doc, words).unwrap();
        assert!(words(&out) <= ANCHOR_SLOT_TOKENS);
        assert!(out.contains(&format!("<selected>\n{sel}\n</selected>")));
        // Nearest neighbours survive, the far ends are trimmed.
        assert!(out.contains("line999\n<selected>") && out.contains("</selected>\nline1101\n"));
        assert!(!out.contains("line1\n") && !out.contains("line2000"));
        assert!(out.starts_with("<anchor>\n```\n…\n") && out.ends_with("…\n```\n</anchor>"));
    }

    #[test]
    fn text_render_uses_surrounding_sentence() {
        let content = "First sentence. The quick brown fox jumps over the dog! Last one.";
        let out = render(&text_anchor(content, "brown fox"), content, words).unwrap();
        assert_eq!(
            out,
            "<anchor>\nThe quick <selected>brown fox</selected> jumps over the dog!\n</anchor>"
        );
        assert!(render(&Anchor::None, content, words).unwrap().is_empty());
    }

    #[test]
    fn text_render_trims_long_sentence_keeping_selection() {
        let lead = (0..1000)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let tail = (0..1000)
            .map(|i| format!("z{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let sel = "w ".repeat(900).trim_end().to_string();
        let content = format!("{lead} {sel} {tail}.");
        let out = render(&text_anchor(&content, &sel), &content, words).unwrap();
        assert!(words(&out) <= ANCHOR_SLOT_TOKENS);
        assert!(out.contains(&format!("a999 <selected>{sel}</selected> z0 ")));
        assert!(!out.contains("a0 ") && !out.contains("z999"));
    }
}
