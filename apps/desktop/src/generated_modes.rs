//! Text generation for Stamina's generated training modes.

use crate::content::Exercise;

const TARGET_TEXT_CHARS: usize = 300;

/// Sample printable characters from the phrase corpus, as in Stamina's
/// MODE_MIXED_TEXT. Line breaks and consecutive spaces are omitted.
pub fn letters_from_phrases(phrases: &[Exercise], seed: u64) -> String {
    let corpus: Vec<char> = phrases
        .iter()
        .flat_map(|phrase| phrase.text.chars())
        .filter(|character| {
            !character.is_control() && (!character.is_whitespace() || *character == ' ')
        })
        .collect();
    let fallback: Vec<char> = "asdf jkl;".chars().collect();
    let corpus = if corpus.iter().all(|c| c.is_whitespace()) {
        &fallback
    } else {
        &corpus
    };
    let mut random = Generator::new(seed);
    let mut output = String::new();
    while output.chars().count() < TARGET_TEXT_CHARS {
        let ch = corpus[random.index(corpus.len())];
        if ch == ' ' && (output.is_empty() || output.ends_with(' ')) {
            continue;
        }
        output.push(ch);
    }
    output
}

/// Build a passage by sampling keys from a layout's printable key string.
///
/// Keys are grouped into words of three to eight characters separated by spaces.
/// Control characters and whitespace in `keys` are ignored. An empty usable
/// key set falls back to a simple home-row sequence.
pub fn all_keys(keys: &str, seed: u64) -> String {
    let keys: Vec<char> = keys
        .chars()
        .filter(|character| !character.is_control() && !character.is_whitespace())
        .collect();
    let fallback: Vec<char> = "asdfjkl;".chars().collect();
    let keys = if keys.is_empty() { &fallback } else { &keys };
    let mut random = Generator::new(seed);
    let mut output = String::with_capacity(TARGET_TEXT_CHARS);

    while output.chars().count() < TARGET_TEXT_CHARS {
        if !output.is_empty() {
            output.push(' ');
        }
        let word_len = 3 + random.index(6);
        for _ in 0..word_len {
            output.push(keys[random.index(keys.len())]);
        }
    }

    output
}

struct Generator(u64);

impl Generator {
    fn new(seed: u64) -> Self {
        // Xorshift's all-zero state never changes, so map it to a fixed nonzero seed.
        Self(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(text: &str) -> Exercise {
        Exercise {
            name: "test".into(),
            text: text.into(),
        }
    }

    #[test]
    fn phrase_mode_uses_printable_corpus_and_is_reproducible() {
        let corpus = vec![exercise("Aa Бб, 123!\n")];
        let first = letters_from_phrases(&corpus, 42);
        assert_eq!(first.chars().count(), TARGET_TEXT_CHARS);
        assert!(
            first
                .chars()
                .all(|character| "Aa Бб,123!".contains(character))
        );
        assert!(!first.contains("  "));
        assert_eq!(first, letters_from_phrases(&corpus, 42));
    }

    #[test]
    fn phrase_mode_has_fallback_for_empty_corpus() {
        let generated = letters_from_phrases(&[], 1);
        assert_eq!(generated.chars().count(), TARGET_TEXT_CHARS);
        assert!(
            generated
                .chars()
                .all(|character| "asdf jkl;".contains(character))
        );
    }

    #[test]
    fn all_keys_uses_keys_groups_words_and_is_reproducible() {
        let generated = all_keys("ab\n\t c", 9);
        assert!((250..=400).contains(&generated.chars().count()));
        assert!(
            generated
                .chars()
                .all(|character| "abc ".contains(character))
        );
        assert!(
            generated
                .split(' ')
                .all(|word| (3..=8).contains(&word.len()))
        );
        assert_eq!(generated, all_keys("ab\n\t c", 9));
    }

    #[test]
    fn all_keys_has_fallback_for_empty_layout() {
        let generated = all_keys("\n\t", 0);
        assert!(
            generated
                .chars()
                .all(|character| "asdfjkl; ".contains(character))
        );
    }
}
