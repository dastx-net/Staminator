/// Current lifecycle state of a typing session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Ready,
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keystroke {
    Correct,
    Incorrect,
    Ignored,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub correct: usize,
    pub errors: usize,
    pub elapsed_ms: u64,
    /// Correct characters per minute, rounded to nearest (half up), like Stamina's CalcSpeed.
    pub cpm: u64,
    /// Errors divided by correct + errors, as a percentage in [0, 100].
    pub error_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub state: SessionState,
    pub position: usize,
    pub correct: usize,
    pub errors: usize,
    pub elapsed_ms: u64,
}

/// A deterministic session engine. `now_ms` values passed to methods must be
/// monotonic milliseconds from any caller-chosen clock.
#[derive(Debug, Clone)]
pub struct Session {
    expected: Vec<char>,
    position: usize,
    correct: usize,
    errors: usize,
    state: SessionState,
    elapsed_before_run_ms: u64,
    run_started_at_ms: Option<u64>,
}

impl Session {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: expected.into().chars().collect(),
            position: 0,
            correct: 0,
            errors: 0,
            state: SessionState::Ready,
            elapsed_before_run_ms: 0,
            run_started_at_ms: None,
        }
    }

    pub fn expected_text(&self) -> String {
        self.expected.iter().collect()
    }

    pub fn next_expected(&self) -> Option<char> {
        self.expected.get(self.position).copied()
    }

    /// Compare a character with the expected character. Wrong input increments
    /// errors but does not advance. The first non-ignored keystroke starts time.
    pub fn type_char(&mut self, ch: char, now_ms: u64) -> Keystroke {
        if !matches!(self.state, SessionState::Ready | SessionState::Running) {
            return Keystroke::Ignored;
        }
        self.start_if_needed(now_ms);
        if self.next_expected() == Some(ch) {
            self.position += 1;
            self.correct += 1;
            if self.position == self.expected.len() {
                self.stop(now_ms);
            }
            Keystroke::Correct
        } else {
            self.errors += 1;
            Keystroke::Incorrect
        }
    }

    pub fn pause(&mut self, now_ms: u64) {
        match self.state {
            SessionState::Running => {
                self.accumulate(now_ms);
                self.state = SessionState::Paused;
            }
            SessionState::Ready => {
                // A pause before the first key has no elapsed run time to save.
                self.state = SessionState::Paused;
            }
            _ => {}
        }
    }

    /// Resume input. As in Stamina, the timer remains idle until the next
    /// keystroke; UI code should consume the key that dismisses the pause.
    pub fn resume(&mut self, _now_ms: u64) {
        if self.state == SessionState::Paused {
            // Stamina clears StartMoment on unpause; its clock restarts when
            // the next character is typed.
            self.state = SessionState::Ready;
            self.run_started_at_ms = None;
        }
    }

    pub fn stop(&mut self, now_ms: u64) -> SessionStats {
        if self.state == SessionState::Running {
            self.accumulate(now_ms);
        }
        self.state = SessionState::Stopped;
        self.run_started_at_ms = None;
        self.stats()
    }

    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        self.elapsed_before_run_ms.saturating_add(
            self.run_started_at_ms
                .map_or(0, |start| now_ms.saturating_sub(start)),
        )
    }

    pub fn snapshot(&self, now_ms: u64) -> SessionSnapshot {
        SessionSnapshot {
            state: self.state,
            position: self.position,
            correct: self.correct,
            errors: self.errors,
            elapsed_ms: self.elapsed_ms(now_ms),
        }
    }

    pub fn stats(&self) -> SessionStats {
        self.stats_for_elapsed(self.elapsed_before_run_ms)
    }

    /// Calculate current metrics using the supplied monotonic clock value.
    /// This is useful before the session has been stopped.
    pub fn stats_at(&self, now_ms: u64) -> SessionStats {
        self.stats_for_elapsed(self.elapsed_ms(now_ms))
    }

    fn stats_for_elapsed(&self, elapsed_ms: u64) -> SessionStats {
        let cpm = if elapsed_ms == 0 {
            0
        } else {
            let numerator = self.correct as u128 * 60_000;
            // Stamina.h::CalcSpeed returns trunc(2*x) - trunc(x), which is
            // nearest-integer rounding with ties rounded upward for x >= 0.
            ((numerator * 2 / elapsed_ms as u128) - (numerator / elapsed_ms as u128))
                .min(u64::MAX as u128) as u64
        };
        let total = self.correct + self.errors;
        SessionStats {
            correct: self.correct,
            errors: self.errors,
            elapsed_ms,
            cpm,
            error_percent: if total == 0 {
                0.0
            } else {
                self.errors as f64 * 100.0 / total as f64
            },
        }
    }

    fn start_if_needed(&mut self, now_ms: u64) {
        if self.state == SessionState::Ready {
            self.state = SessionState::Running;
            self.run_started_at_ms = Some(now_ms);
        }
    }

    fn accumulate(&mut self, now_ms: u64) {
        if let Some(start) = self.run_started_at_ms.take() {
            self.elapsed_before_run_ms = self
                .elapsed_before_run_ms
                .saturating_add(now_ms.saturating_sub(start));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_key_counts_error_without_advancing_and_starts_clock() {
        let mut session = Session::new("ab");
        assert_eq!(session.type_char('x', 100), Keystroke::Incorrect);
        assert_eq!(session.snapshot(350).position, 0);
        assert_eq!(session.snapshot(350).errors, 1);
        assert_eq!(session.snapshot(350).elapsed_ms, 250);
        assert_eq!(session.type_char('a', 400), Keystroke::Correct);
        assert_eq!(session.next_expected(), Some('b'));
    }

    #[test]
    fn pause_and_resume_exclude_paused_time_and_stop_calculates_metrics() {
        let mut session = Session::new("abc");
        session.type_char('a', 10);
        session.pause(1_010);
        assert_eq!(session.snapshot(5_010).elapsed_ms, 1_000);
        session.resume(5_010);
        assert_eq!(session.snapshot(9_010).elapsed_ms, 1_000);
        session.type_char('x', 9_010);
        session.type_char('b', 9_510);
        let stats = session.stop(9_510);
        assert_eq!(stats.elapsed_ms, 1_500);
        assert_eq!(stats.correct, 2);
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.cpm, 80);
        assert!((stats.error_percent - 100.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn pause_before_first_input_ignores_typing_until_resumed() {
        let mut session = Session::new("ab");
        session.pause(100);
        assert_eq!(session.snapshot(500).state, SessionState::Paused);
        assert_eq!(session.snapshot(500).elapsed_ms, 0);
        assert_eq!(session.type_char('a', 500), Keystroke::Ignored);
        assert_eq!(session.snapshot(500).position, 0);
        assert_eq!(session.snapshot(500).elapsed_ms, 0);

        session.resume(1_000);
        assert_eq!(session.snapshot(1_000).state, SessionState::Ready);
        assert_eq!(session.type_char('a', 2_000), Keystroke::Correct);
        assert_eq!(session.snapshot(2_250).elapsed_ms, 250);
    }

    #[test]
    fn completion_stops_automatically_and_ignores_later_input() {
        let mut session = Session::new("a");
        assert_eq!(session.type_char('a', 20), Keystroke::Correct);
        assert_eq!(session.snapshot(100).state, SessionState::Stopped);
        assert_eq!(session.type_char('a', 200), Keystroke::Ignored);
        assert_eq!(session.stats().elapsed_ms, 0);
    }

    #[test]
    fn empty_session_has_zero_metrics() {
        let session = Session::new("");
        assert_eq!(session.stats().cpm, 0);
        assert_eq!(session.stats().error_percent, 0.0);
    }

    #[test]
    fn cpm_matches_stamina_half_up_rounding_and_stats_work_while_running() {
        let mut session = Session::new("xy");
        session.type_char('x', 1_000);
        // One character in 40 seconds is 1.5 CPM, which CalcSpeed returns as 2.
        assert_eq!(session.stats_at(41_000).cpm, 2);
    }
}
