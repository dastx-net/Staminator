use stamina_data::load_lesson_catalog;
use staminator_core::{Keystroke, Session, StopReason};
use std::path::PathBuf;

#[test]
fn bundled_courses_load_in_order_and_can_be_completed() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lessons");
    let catalog = load_lesson_catalog(&[root]);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    for (language, prefix, count) in [("en", "ktouch.us.", 15), ("ru", "ktouch.ru1.", 16)] {
        let lessons: Vec<_> = catalog
            .lessons
            .iter()
            .filter(|entry| entry.lesson.language == language)
            .collect();
        assert_eq!(lessons.len(), count);
        for (index, entry) in lessons.iter().enumerate() {
            assert_eq!(entry.lesson.id, format!("{prefix}{:03}", index + 1));
            let mut session = Session::new(&entry.lesson.text);
            for (time, ch) in entry.lesson.text.chars().enumerate() {
                assert_eq!(session.type_char(ch, time as u64 + 1), Keystroke::Correct);
            }
            assert_eq!(session.stop_reason(), Some(StopReason::Completed));
        }
    }
}
