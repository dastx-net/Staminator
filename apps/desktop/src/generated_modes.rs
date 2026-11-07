//! Text generation for Stamina's generated training modes.

use crate::content::Exercise;

const TARGET_TEXT_CHARS: usize = 300;

/// Randomized phrase order. The chosen phrase starts the first cycle; each
/// following cycle gets a fresh permutation from the same seeded generator.
pub struct PhraseSequence {
    order: Vec<usize>,
    cursor: usize,
    random: Generator,
}

impl PhraseSequence {
    pub fn new(count: usize, first: usize, seed: u64) -> Self {
        let mut sequence = Self {
            order: Vec::new(),
            cursor: 0,
            random: Generator::new(seed),
        };
        sequence.order = (0..count).collect();
        if count > 0 {
            let first = first.min(count - 1);
            sequence.order.swap(0, first);
            sequence.shuffle_from(1);
        }
        sequence
    }

    #[cfg(test)]
    pub fn first(&self) -> Option<usize> {
        self.order.first().copied()
    }

    pub fn next(&mut self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        self.cursor += 1;
        if self.cursor == self.order.len() {
            self.order = (0..self.order.len()).collect();
            self.shuffle_from(0);
            self.cursor = 0;
        }
        self.order.get(self.cursor).copied()
    }

    fn shuffle_from(&mut self, start: usize) {
        for index in (start + 1..self.order.len()).rev() {
            let other = start + self.random.index(index - start + 1);
            self.order.swap(index, other);
        }
    }
}

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

/// Build a passage by shuffling the layout's printable keys and emitting each
/// key once per cycle. Keys are grouped into words of three to eight characters.
/// Control characters and whitespace are ignored.
pub fn all_keys(keys: &str, seed: u64) -> String {
    let keys: Vec<char> = keys
        .chars()
        .filter(|character| !character.is_control() && !character.is_whitespace())
        .collect();
    let fallback: Vec<char> = "asdfjkl;".chars().collect();
    let key_set = if keys.is_empty() { fallback } else { keys };
    let mut random = Generator::new(seed);
    let mut output = String::with_capacity(TARGET_TEXT_CHARS);
    let mut output_len = 0;

    let mut bag = Vec::new();
    while output_len < TARGET_TEXT_CHARS {
        if bag.is_empty() {
            bag.clone_from(&key_set);
            for index in (1..bag.len()).rev() {
                let other = random.index(index + 1);
                bag.swap(index, other);
            }
        }
        if !output.is_empty() && output_len + 1 < TARGET_TEXT_CHARS {
            output.push(' ');
            output_len += 1;
        }
        let remaining = TARGET_TEXT_CHARS - output_len;
        let word_len = if remaining <= 8 {
            remaining
        } else {
            (3 + random.index(6)).min(remaining - 4)
        };
        for _ in 0..word_len {
            if bag.is_empty() {
                bag.clone_from(&key_set);
                for index in (1..bag.len()).rev() {
                    let other = random.index(index + 1);
                    bag.swap(index, other);
                }
            }
            output.push(bag.pop().expect("key set is nonempty"));
            output_len += 1;
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
    fn phrase_sequence_visits_each_phrase_then_repeats_with_seeded_cycles() {
        let mut first = PhraseSequence::new(5, 2, 42);
        let mut same = PhraseSequence::new(5, 2, 42);
        let mut initial = vec![first.first().unwrap()];
        let mut matching = vec![same.first().unwrap()];
        initial.extend((0..4).map(|_| first.next().unwrap()));
        matching.extend((0..4).map(|_| same.next().unwrap()));
        assert_eq!(initial, matching);
        let mut sorted = initial.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..5).collect::<Vec<_>>());
        let mut second_cycle = vec![first.next().unwrap()];
        let mut matching_second = vec![same.next().unwrap()];
        second_cycle.extend((0..4).map(|_| first.next().unwrap()));
        matching_second.extend((0..4).map(|_| same.next().unwrap()));
        let mut sorted_second = second_cycle.clone();
        sorted_second.sort_unstable();
        assert_eq!(sorted_second, (0..5).collect::<Vec<_>>());
        assert_eq!(second_cycle, matching_second);
    }

    #[test]
    fn phrase_sequence_handles_empty_and_single_phrase_catalogs() {
        let mut empty = PhraseSequence::new(0, 0, 3);
        assert_eq!(empty.first(), None);
        assert_eq!(empty.next(), None);
        let mut single = PhraseSequence::new(1, 9, 3);
        assert_eq!(single.first(), Some(0));
        assert_eq!(single.next(), Some(0));
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
        assert_eq!(generated.chars().count(), TARGET_TEXT_CHARS);
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

        let unicode = all_keys("абв", 9);
        assert_eq!(unicode.chars().count(), TARGET_TEXT_CHARS);
        assert!(unicode.split(' ').all(|word| !word.is_empty()));

        let generated = all_keys("abcdef", 9);
        let cycle = generated
            .chars()
            .filter(|character| *character != ' ')
            .take(6);
        let mut cycle: Vec<_> = cycle.collect();
        cycle.sort_unstable();
        assert_eq!(cycle, "abcdef".chars().collect::<Vec<_>>());
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
