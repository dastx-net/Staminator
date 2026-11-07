use stamina_data::parse_lessons;
use staminator_core::{Keystroke, Session, SessionState, StopReason};

#[test]
fn manual_result_can_continue_without_losing_time_or_errors() {
    let mut session = Session::new("ab");
    session.type_char('x', 100);
    session.type_char('a', 200);
    session.stop(600);
    assert_eq!(session.stop_reason(), Some(StopReason::Manual));
    assert_eq!(session.stats().elapsed_ms, 500);
    assert!(session.continue_after_stop());
    assert_eq!(session.elapsed_ms(5_000), 500);
    session.type_char('b', 5_000);
    assert_eq!(session.stop_reason(), Some(StopReason::Completed));
    assert_eq!(
        (
            session.stats().correct,
            session.stats().errors,
            session.stats().elapsed_ms
        ),
        (2, 1, 500)
    );
    assert!(!session.continue_after_stop());
}

#[test]
fn time_limit_uses_active_time_and_cannot_resume() {
    let mut session = Session::new("abc");
    session.type_char('a', 100);
    session.pause(600);
    assert!(!session.check_time_limit(10_000, 1_000));
    session.resume(10_000);
    session.type_char('b', 11_000);
    assert!(!session.check_time_limit(11_499, 1_000));
    assert!(session.check_time_limit(11_500, 1_000));
    assert_eq!(session.stop_reason(), Some(StopReason::TimeLimit));
    assert_eq!(session.type_char('c', 11_500), Keystroke::Ignored);
    assert!(!session.continue_after_stop());
}

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
