//! The match: its states and one-second timer, when it ends, and the XP (score events) it awards.
//!
//! Reimplemented from the game mode (SCGameMode and the offline one, SCGameMode_OfflineBots):
//! - **States**: Unreal's WaitingToStart, InProgress, WaitingPostMatch and LeavingMap, plus the game's own
//!   WaitingPreMatch, PreMatchIntro (the level intro), WaitingPostMatchOutro and PostMatchOutro (the end-of-match
//!   HUD outro).
//! - **Timer** (DefaultTimer, once a second of game time), by state:
//!   - WaitingToStart: counts `remaining_time` down, then moves to WaitingPreMatch, or straight to PreMatchIntro
//!     when PreMatchWaitTime is 0. It is 0 in the game: no constructor or ini sets it.
//!   - PreMatchIntro and PostMatchOutro: the timer doesn't count; their sequences end them.
//!   - Otherwise it counts down, and then:
//!     - InProgress counts `elapsed_in_progress_time` up and ends the match at 0;
//!     - WaitingPostMatchOutro ends it again (on to PostMatchOutro);
//!     - WaitingPostMatch restarts the game (back to the lobby; here, leaving the map).
//! - **Entering a state** sets `remaining_time`:
//!   - WaitingToStart: the hold-for-players time, which is 0, so 1 s.
//!   - WaitingPreMatch: PreMatchWaitTime.
//!   - InProgress: RoundTime.
//!   - WaitingPostMatchOutro: PostMatchOutroWaitTime.
//!   - WaitingPostMatch: PostMatchWaitTime.
//! - **EndMatch**:
//!   - InProgress goes to WaitingPostMatchOutro (each player gets the HUD outro);
//!   - WaitingPostMatchOutro goes to PostMatchOutro;
//!   - PostMatchOutro goes to WaitingPostMatch.
//! - **Ending early** (the offline mode's timer): once the match has run more than 10 s, it ends when no counselor
//!   is left alive and unescaped, or when Jason is dead.
//!   - Quick Play (SCGameMode_Hunt) counts dead + escaped against the spawned counselors instead, and waits for a
//!     pending Tommy Jarvis. For one counselor without Tommy, that is the same rule.
//! - **XP** (SCPlayerState's score event handler):
//!   - an event marked only-once scores once per match;
//!   - XP = Experience × scale, truncated toward zero (perks would multiply it; none are modelled);
//!   - the time bonus's scale is the match's elapsed seconds × (1/60), so its XP is per minute.
//!
//! What the match awards a counselor:
//! - Escaping (the game mode's escape handler): Escape, Unscathed if never damaged, and the time bonus.
//! - When the match ends (Quick Play's WaitingPostMatchOutro handler):
//!   - the time bonus if not dead;
//!   - Unscathed if never damaged;
//!   - CompletedMatch;
//!   - TeamEscaped to everyone if every counselor escaped.
//!   - The offline mode gives no XP at the end. This port follows Quick Play here, so the results have something
//!     to show.
//!   - Only-once events that she already scored on escaping don't score again.

/// SCGameMode's DefaultTimer scale for the time bonus (1/60 as a float).
const PER_MINUTE: f32 = 1.0 / 60.0;
/// The offline mode ends the match early only after this many seconds in progress (`elapsed > 10`).
const END_CHECK_AFTER: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchState {
    WaitingToStart,
    WaitingPreMatch,
    PreMatchIntro,
    InProgress,
    WaitingPostMatchOutro,
    PostMatchOutro,
    WaitingPostMatch,
    LeavingMap,
}

/// Match timing, in whole seconds as the game keeps them.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchRules {
    /// RoundTime (DefaultGame.ini).
    pub round_time: i32,
    /// PreMatchWaitTime: 0 in the game.
    pub pre_match_wait_time: i32,
    /// PostMatchOutroWaitTime: the end-of-match outro's wait.
    pub post_match_outro_wait_time: i32,
    /// PostMatchWaitTime: the results, before the game returns to the lobby.
    pub post_match_wait_time: i32,
}

/// Who is still playing, for the early end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roster {
    /// Counselors alive and not escaped.
    pub counselors_left: u32,
    /// Whether Jason is alive.
    /// - `None`: there is no Jason (a test run without him). The game would end such a match at once, so this port
    ///   doesn't end it.
    pub killer_alive: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub state: MatchState,
    /// SCGameState RemainingTime: the current state's countdown, s.
    pub remaining_time: i32,
    /// SCGameState ElapsedInProgressTime, s.
    pub elapsed_in_progress_time: i32,
    /// Game time since the timer last fired, s.
    clock: f32,
}

impl Default for Match {
    fn default() -> Self {
        Self::new()
    }
}

impl Match {
    /// A match waiting to start (HandleMatchIsWaitingToStart: the hold time is 0, so 1 s).
    pub fn new() -> Self {
        Self { state: MatchState::WaitingToStart, remaining_time: 1, elapsed_in_progress_time: 0, clock: 0.0 }
    }

    /// Advances `dt` seconds of game time; the timer fires once per whole second. Returns the states entered, in
    /// order.
    pub fn advance(&mut self, dt: f32, rules: &MatchRules, roster: &Roster) -> Vec<MatchState> {
        let mut entered = Vec::new();
        self.clock += dt;
        while self.clock >= 1.0 {
            self.clock -= 1.0;
            self.timer(rules, roster, &mut entered);
        }
        entered
    }

    /// DefaultTimer, then the offline mode's early end.
    fn timer(&mut self, rules: &MatchRules, roster: &Roster, entered: &mut Vec<MatchState>) {
        match self.state {
            MatchState::WaitingToStart => {
                if self.remaining_time < 1 {
                    return;
                }
                self.remaining_time -= 1;
                if self.remaining_time == 0 {
                    let next = if rules.pre_match_wait_time < 1 { MatchState::PreMatchIntro } else { MatchState::WaitingPreMatch };
                    self.set_state(next, rules, entered);
                }
                return;
            }
            MatchState::PreMatchIntro | MatchState::PostMatchOutro | MatchState::LeavingMap => return,
            _ => {}
        }
        if self.remaining_time > 0 {
            self.remaining_time -= 1;
        }
        match self.state {
            MatchState::WaitingPreMatch if self.remaining_time == 0 => self.set_state(MatchState::PreMatchIntro, rules, entered),
            MatchState::InProgress => {
                self.elapsed_in_progress_time += 1;
                if self.remaining_time == 0 {
                    self.end_match(rules, entered);
                }
            }
            MatchState::WaitingPostMatchOutro if self.remaining_time == 0 => self.end_match(rules, entered),
            MatchState::WaitingPostMatch if self.remaining_time == 0 => self.set_state(MatchState::LeavingMap, rules, entered),
            _ => {}
        }
        if self.state == MatchState::InProgress
            && self.elapsed_in_progress_time > END_CHECK_AFTER
            && (roster.counselors_left == 0 || roster.killer_alive == Some(false))
        {
            self.end_match(rules, entered);
        }
    }

    /// StartMatch, when the level intro is over (this port plays none).
    pub fn start(&mut self, rules: &MatchRules) -> Vec<MatchState> {
        let mut entered = Vec::new();
        if matches!(self.state, MatchState::WaitingToStart | MatchState::WaitingPreMatch | MatchState::PreMatchIntro) {
            self.set_state(MatchState::InProgress, rules, &mut entered);
        }
        entered
    }

    /// EndMatch: from the match to the outro's wait, from there to the outro, and from the outro to the results.
    pub fn end(&mut self, rules: &MatchRules) -> Vec<MatchState> {
        let mut entered = Vec::new();
        self.end_match(rules, &mut entered);
        entered
    }

    fn end_match(&mut self, rules: &MatchRules, entered: &mut Vec<MatchState>) {
        let next = match self.state {
            MatchState::InProgress => MatchState::WaitingPostMatchOutro,
            MatchState::WaitingPostMatchOutro => MatchState::PostMatchOutro,
            MatchState::PostMatchOutro => MatchState::WaitingPostMatch,
            _ => return,
        };
        self.set_state(next, rules, entered);
    }

    fn set_state(&mut self, state: MatchState, rules: &MatchRules, entered: &mut Vec<MatchState>) {
        self.state = state;
        match state {
            MatchState::WaitingPreMatch => self.remaining_time = rules.pre_match_wait_time,
            MatchState::InProgress => self.remaining_time = rules.round_time,
            MatchState::WaitingPostMatchOutro => self.remaining_time = rules.post_match_outro_wait_time,
            MatchState::WaitingPostMatch => self.remaining_time = rules.post_match_wait_time,
            _ => {}
        }
        entered.push(state);
    }

    /// The match is over for scoring: the outro, the results, or leaving.
    pub fn ended(&self) -> bool {
        matches!(
            self.state,
            MatchState::WaitingPostMatchOutro | MatchState::PostMatchOutro | MatchState::WaitingPostMatch | MatchState::LeavingMap
        )
    }
}

/// A score event class (Blueprints/ScoreEvents).
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreEvent {
    pub experience: i32,
    /// EScoreEventCategory, without the enum prefix.
    pub category: String,
    /// bOnlyScoreOnce.
    pub only_once: bool,
    /// bAllCounselors: scores for every counselor.
    pub all_counselors: bool,
    /// EventDescription: shown with "<exp>" replaced by the XP.
    pub description: String,
}

impl ScoreEvent {
    pub fn text(&self, xp: i32) -> String {
        self.description.replace("<exp>", &xp.to_string())
    }
}

/// One score event a player scored.
#[derive(Clone, Debug, PartialEq)]
pub struct Award {
    pub event: String,
    pub xp: i32,
}

/// A player's score events this match (SCPlayerState PlayerScoringEvents).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerScore {
    pub awards: Vec<Award>,
}

impl PlayerScore {
    /// Scores `event` (named `name`) at `scale`. Returns the XP, or `None` for an only-once event already scored.
    pub fn award(&mut self, name: &str, event: &ScoreEvent, scale: f32) -> Option<i32> {
        if event.only_once && self.has(name) {
            return None;
        }
        let xp = (event.experience as f32 * scale) as i32;
        self.awards.push(Award { event: name.to_string(), xp });
        Some(xp)
    }

    pub fn has(&self, name: &str) -> bool {
        self.awards.iter().any(|a| a.event == name)
    }

    pub fn total(&self) -> i32 {
        self.awards.iter().map(|a| a.xp).sum()
    }
}

/// What a counselor did this match, for the match's own XP.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CounselorRecord {
    pub dead: bool,
    pub escaped: bool,
    /// bHasTakenDamage.
    pub taken_damage: bool,
}

/// The time bonus's scale after `elapsed` seconds in progress.
fn minutes(elapsed: i32) -> f32 {
    elapsed as f32 * PER_MINUTE
}

/// The score events (name, scale) a counselor gets for escaping, `elapsed` seconds into the match.
pub fn escape_awards(record: &CounselorRecord, elapsed: i32) -> Vec<(&'static str, f32)> {
    let mut out = vec![("Escape", 1.0)];
    if !record.taken_damage {
        out.push(("Unscathed", 1.0));
    }
    out.push(("CounselorTimeBonus", minutes(elapsed)));
    out
}

/// The score events (name, scale) a counselor gets when the match ends (Quick Play's rules). `all_escaped`: every
/// counselor escaped.
pub fn end_of_match_awards(record: &CounselorRecord, all_escaped: bool, elapsed: i32) -> Vec<(&'static str, f32)> {
    let mut out = Vec::new();
    if !record.dead {
        out.push(("CounselorTimeBonus", minutes(elapsed)));
    }
    if !record.taken_damage {
        out.push(("Unscathed", 1.0));
    }
    out.push(("CompletedMatch", 1.0));
    if all_escaped {
        out.push(("TeamEscaped", 1.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic timings (not the game's): exercises the rules only.
    fn rules() -> MatchRules {
        MatchRules { round_time: 30, pre_match_wait_time: 0, post_match_outro_wait_time: 3, post_match_wait_time: 4 }
    }
    const PLAYING: Roster = Roster { counselors_left: 1, killer_alive: Some(true) };

    fn run(m: &mut Match, seconds: i32, roster: &Roster) -> Vec<MatchState> {
        (0..seconds).flat_map(|_| m.advance(1.0, &rules(), roster)).collect()
    }

    #[test]
    fn waits_a_second_then_the_intro_then_runs_the_round_out() {
        let mut m = Match::new();
        assert_eq!(m.advance(0.5, &rules(), &PLAYING), vec![]);
        assert_eq!(m.advance(0.5, &rules(), &PLAYING), vec![MatchState::PreMatchIntro]);
        assert_eq!(m.start(&rules()), vec![MatchState::InProgress]);
        assert_eq!(m.remaining_time, 30);
        assert_eq!(run(&mut m, 29, &PLAYING), vec![]);
        assert_eq!(run(&mut m, 1, &PLAYING), vec![MatchState::WaitingPostMatchOutro]);
        assert_eq!(m.elapsed_in_progress_time, 30);
        // The outro's wait, then the outro (ended by its sequence), the results and leaving.
        assert_eq!(run(&mut m, 3, &PLAYING), vec![MatchState::PostMatchOutro]);
        assert_eq!(run(&mut m, 5, &PLAYING), vec![]);
        assert_eq!(m.end(&rules()), vec![MatchState::WaitingPostMatch]);
        assert_eq!(run(&mut m, 4, &PLAYING), vec![MatchState::LeavingMap]);
    }

    #[test]
    fn ends_early_once_no_counselor_is_left_but_not_in_the_first_ten_seconds() {
        let gone = Roster { counselors_left: 0, killer_alive: Some(true) };
        let mut m = Match::new();
        run(&mut m, 1, &PLAYING);
        m.start(&rules());
        assert_eq!(run(&mut m, 10, &gone), vec![]);
        assert_eq!(run(&mut m, 1, &gone), vec![MatchState::WaitingPostMatchOutro]);
        assert_eq!(m.elapsed_in_progress_time, 11);
        assert_eq!(m.remaining_time, 3);
    }

    #[test]
    fn jason_dead_ends_it_and_no_jason_does_not() {
        let mut m = Match::new();
        run(&mut m, 1, &PLAYING);
        m.start(&rules());
        assert_eq!(run(&mut m, 15, &Roster { counselors_left: 1, killer_alive: None }), vec![]);
        assert_eq!(run(&mut m, 1, &Roster { counselors_left: 1, killer_alive: Some(false) }), vec![MatchState::WaitingPostMatchOutro]);
    }

    fn event(xp: i32, once: bool) -> ScoreEvent {
        ScoreEvent { experience: xp, category: "Defense".into(), only_once: once, all_counselors: false, description: "Ok +<exp>".into() }
    }

    #[test]
    fn once_events_score_once_and_scale_truncates() {
        let mut s = PlayerScore::default();
        assert_eq!(s.award("A", &event(50, true), 1.0), Some(50));
        assert_eq!(s.award("A", &event(50, true), 1.0), None);
        assert_eq!(s.award("B", &event(10, false), 1.0), Some(10));
        assert_eq!(s.award("B", &event(10, false), 1.0), Some(10));
        // 10 XP a minute: 59 s is 9, a minute is 10.
        assert_eq!(s.award("T", &event(10, false), minutes(59)), Some(9));
        assert_eq!(s.award("T", &event(10, false), minutes(60)), Some(10));
        assert_eq!(s.total(), 89);
        assert_eq!(event(5, true).text(5), "Ok +5");
    }

    #[test]
    fn escaping_then_the_end_awards() {
        let r = CounselorRecord { dead: false, escaped: true, taken_damage: false };
        assert_eq!(escape_awards(&r, 120), vec![("Escape", 1.0), ("Unscathed", 1.0), ("CounselorTimeBonus", minutes(120))]);
        let dead = CounselorRecord { dead: true, escaped: false, taken_damage: true };
        assert_eq!(end_of_match_awards(&dead, false, 300), vec![("CompletedMatch", 1.0)]);
        assert_eq!(end_of_match_awards(&r, true, 300).last(), Some(&("TeamEscaped", 1.0)));
    }
}
