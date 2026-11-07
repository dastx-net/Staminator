//! Deterministic lesson text generation matching Stamina's lesson editor.
//!
//! The source algorithm is `CLessons::GenerateSeq` and `CLessons::seq` in
//! `src/stamina/Lessons.cpp`; the editor inserts its result at the caret in
//! `CLessonsDlg::OnGenerate` (`src/stamina/DlgLessons.cpp`).

const PAIR_PATTERN: &str = "111222 11212 21122 12121 22121 12112 21212";
const TRIPLE_PATTERN: &str = "11231 21322 31332 13212 13231 32312 123213 132312 231321";
const QUAD_PATTERN: &str = "13124 32441 31342 23114 24213 41332 42431 14223";
const SHORT_PAIR_PATTERN: &str = "11212 21122 12121 22121";

/// Generates the text Stamina inserts for one through four selected symbols.
/// Unsupported lengths return an empty string, as in the original editor.
pub fn generate_lesson_text(symbols: &str) -> String {
    let chars: Vec<char> = symbols.chars().collect();
    match chars.len() {
        0 => String::new(),
        1 => std::iter::repeat_n(chars[0], 5).chain([' ']).collect(),
        2 => sequence(&chars, false),
        3 => {
            let mut result = String::new();
            result.push_str(&sequence(&[chars[0], chars[1]], false));
            result.push_str(&sequence(&[chars[0], chars[2]], false));
            result.push_str(&sequence(&[chars[1], chars[2]], false));
            result.push_str(&sequence(&chars, false));
            result
        }
        4 => {
            let mut result = String::new();
            for pair in [
                [chars[1], chars[2]],
                [chars[0], chars[2]],
                [chars[3], chars[0]],
                [chars[1], chars[3]],
            ] {
                result.push_str(&sequence(&pair, false));
            }
            result.push_str(&sequence(&[chars[0], chars[1]], true));
            result.push_str(&sequence(&[chars[2], chars[3]], true));
            result.push_str(&sequence(&chars, false));
            result
        }
        _ => String::new(),
    }
}

fn sequence(symbols: &[char], short: bool) -> String {
    let pattern = match (symbols.len(), short) {
        (2, false) => PAIR_PATTERN,
        (2, true) => SHORT_PAIR_PATTERN,
        (3, false) => TRIPLE_PATTERN,
        (4, false) => QUAD_PATTERN,
        _ => return String::new(),
    };
    let mut result: String = pattern
        .chars()
        .map(|ch| match ch {
            ' ' => ' ',
            digit @ '1'..='4' => symbols[(digit as usize) - ('1' as usize)],
            _ => unreachable!("patterns contain only spaces and indices 1 through 4"),
        })
        .collect();
    result.push(' ');
    result
}

#[cfg(test)]
mod tests {
    use super::generate_lesson_text;

    #[test]
    fn one_symbol_is_repeated_five_times() {
        assert_eq!(generate_lesson_text("x"), "xxxxx ");
    }

    #[test]
    fn two_symbols_follow_original_pair_pattern() {
        assert_eq!(
            generate_lesson_text("ab"),
            "aaabbb aabab baabb ababa bbaba abaab babab "
        );
    }

    #[test]
    fn three_symbols_emit_each_pair_then_triple_pattern() {
        let text = generate_lesson_text("abc");
        assert!(text.starts_with("aaabbb aabab baabb ababa bbaba abaab babab "));
        assert!(text.contains("aabca bacbb caccb acbab"));
        assert!(text.ends_with("bcacba "));
    }

    #[test]
    fn four_symbols_use_short_pair_patterns_in_the_middle() {
        let text = generate_lesson_text("abcd");
        assert!(text.contains("acabd cbdda cacdb bcaad"));
        assert!(text.contains("aabab baabb ababa bbaba "));
        assert!(text.contains("ccdcd dccdd cdcdc ddcdc "));
        assert!(text.ends_with("dbdca adbbc "));
    }

    #[test]
    fn unsupported_lengths_return_empty_text_and_unicode_is_counted_by_character() {
        assert_eq!(generate_lesson_text(""), "");
        assert_eq!(generate_lesson_text("abcde"), "");
        assert_eq!(
            generate_lesson_text("ёж"),
            "ёёёжжж ёёжёж жёёжж ёжёжё жжёжё ёжёёж жёжёж "
        );
    }
}
