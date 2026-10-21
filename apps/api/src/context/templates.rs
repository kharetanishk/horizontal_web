//! System prompts per mode and the quick-action templates (FR-8), versioned in `apps/api/prompts/`.

use super::Mode;
use crate::types::DoubtType;

const SYSTEM_MAIN: &str = include_str!("../../prompts/system_main.md");
const SYSTEM_DOUBT: &str = include_str!("../../prompts/system_doubt.md");
const SYSTEM_SUMMARY: &str = include_str!("../../prompts/system_summary.md");

const SIMPLER: &str = include_str!("../../prompts/simpler.md");
const EXAMPLE: &str = include_str!("../../prompts/example.md");
const WHY: &str = include_str!("../../prompts/why.md");
const DEEPER: &str = include_str!("../../prompts/deeper.md");
const WRONG: &str = include_str!("../../prompts/wrong.md");
const FREE: &str = include_str!("../../prompts/free.md");

pub fn system_prompt(mode: Mode) -> &'static str {
    match mode {
        Mode::Main => SYSTEM_MAIN,
        Mode::Doubt => SYSTEM_DOUBT,
        Mode::Summary => SYSTEM_SUMMARY,
    }
    .trim_end()
}

/// AC-8.1: each quick action has its own template; `free` is the free-text question.
pub fn template(doubt: DoubtType) -> &'static str {
    match doubt {
        DoubtType::Simpler => SIMPLER,
        DoubtType::Example => EXAMPLE,
        DoubtType::Why => WHY,
        DoubtType::Deeper => DEEPER,
        DoubtType::Wrong => WRONG,
        DoubtType::Free => FREE,
    }
    .trim_end()
}

/// The user turn as sent: a reply is its content unchanged; a branch is its template, the
/// rendered anchor block (`anchor::render`) and the question. Pieces are joined in fixed
/// order, never substituted into the template, so user text can't inject placeholders.
pub fn user_message(doubt: Option<DoubtType>, anchor_block: &str, question: &str) -> String {
    let Some(doubt) = doubt else {
        return question.to_string();
    };
    let question = (!question.is_empty()).then(|| format!("<question>\n{question}\n</question>"));
    [
        Some(template(doubt).to_string()),
        (!anchor_block.is_empty()).then(|| anchor_block.to_string()),
        question,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [DoubtType; 6] = [
        DoubtType::Simpler,
        DoubtType::Example,
        DoubtType::Why,
        DoubtType::Deeper,
        DoubtType::Wrong,
        DoubtType::Free,
    ];

    #[test]
    fn each_doubt_type_maps_to_its_own_template() {
        let files = [SIMPLER, EXAMPLE, WHY, DEEPER, WRONG, FREE];
        for (d, file) in ALL.iter().zip(files) {
            assert_eq!(template(*d), file.trim_end());
            assert!(!template(*d).is_empty());
        }
        let mut seen: Vec<&str> = ALL.iter().map(|d| template(*d)).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ALL.len());
        assert_eq!(template(DoubtType::Free), FREE.trim_end());
    }

    #[test]
    fn wrong_template_asks_to_recheck_the_anchored_part() {
        let t = template(DoubtType::Wrong).to_lowercase();
        assert!(t.contains("re-check") && t.contains("part being asked about"));
    }

    #[test]
    fn system_prompts_per_mode_are_distinct() {
        let all = [Mode::Main, Mode::Doubt, Mode::Summary].map(system_prompt);
        assert!(all.iter().all(|p| !p.is_empty()));
        assert!(all[0] != all[1] && all[1] != all[2] && all[0] != all[2]);
    }

    #[test]
    fn user_message_layout_is_fixed_and_deterministic() {
        let anchor = "<anchor>\nx <selected>y</selected>\n</anchor>";
        let out = user_message(Some(DoubtType::Wrong), anchor, "is this {{anchor}} right?");
        assert_eq!(
            out,
            format!(
                "{}\n\n{anchor}\n\n<question>\nis this {{{{anchor}}}} right?\n</question>",
                template(DoubtType::Wrong)
            )
        );
        assert_eq!(
            out,
            user_message(Some(DoubtType::Wrong), anchor, "is this {{anchor}} right?")
        );
        // Quick action with no typed question, whole-node anchor.
        assert_eq!(
            user_message(Some(DoubtType::Simpler), "", ""),
            template(DoubtType::Simpler)
        );
        // Replies are not wrapped.
        assert_eq!(user_message(None, "", "hello"), "hello");
    }
}
