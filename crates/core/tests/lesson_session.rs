use stamina_data::parse_lessons;
use staminator_core::{Keystroke, Session, SessionState};

#[test]
fn parsed_lesson_drives_a_session_through_pause_and_completion() {
    let lessons = parse_lessons("[#]>Section\r\nDescription\r\n[#] Lesson\r\nаb\r\n");
    let lesson = lessons.iter().find(|lesson| lesson.kind == ' ').unwrap();
    let mut session = Session::new(lesson.text.as_ref().unwrap());

    assert_eq!(session.type_char('x', 100), Keystroke::Incorrect);
    assert_eq!(session.type_char('а', 200), Keystroke::Correct);
    session.pause(600);
    assert_eq!(session.type_char('b', 1_000), Keystroke::Ignored);
    session.resume(2_000);
    assert_eq!(session.type_char('b', 3_000), Keystroke::Correct);

    assert_eq!(session.snapshot(5_000).state, SessionState::Stopped);
    let result = session.stats();
    assert_eq!(
        (result.correct, result.errors, result.elapsed_ms),
        (2, 1, 500)
    );
    assert_eq!(result.cpm, 240);
    assert!((result.error_percent - 100.0 / 3.0).abs() < 0.001);
    assert_eq!(session.type_char('b', 6_000), Keystroke::Ignored);
    assert_eq!(session.stop(7_000), result);
}
