//! The match (f13-core::game_mode): its timer from when the counselor is in the world, the end once she is dead
//! or escaped (or the round runs out), the end-of-match outro and the results, then leaving.
//!
//! - **Not played**: the level intro and the outro sequence, so both states pass at once.
//! - **Leaving**: when the results' wait is over, the game returns to the lobby; this port closes.
//! - **XP**: score events come in as [`Score`] and [`ClassScore`] triggers from where the game scores them:
//!   - picking up an item: its PickupScoreEvent;
//!   - installing a car part, and the car's last part: the car's PartRepairedScoreEvent and
//!     VehicleRepairedScoreEvent;
//!   - first aid used, a door barricaded, firecrackers thrown, a bear trap placed: the item's own event;
//!   - Jason hit with a weapon: DamageJason;
//!   - Jason stunned: StunJason, or the item's own (shotgun, flare, firecrackers, bear trap);
//!   - breaking his grab: EscapeJasonGrab;
//!   - a flare hitting him: SpotJasonFlare;
//!   - escaping and the match's end: f13-core::game_mode.
//! - **HUD**: each award shows as its description with "<exp>" replaced by the XP ("Arm Yourself +10") and its
//!   category, as the counselor's ClientDisplayScoreEvent (FUN_1403e9070, its message FUN_1403ee250) hands them to the
//!   HUD's OnShowScoreEvent: XPDisplayWidget's lines (ui.rs). Without the game's HUD, a plain text list for 4 s.

use bevy::prelude::*;
pub use f13_core::game_mode::MatchState;
use f13_core::game_mode::{end_of_match_awards, escape_awards, killer_end_of_match_awards, CounselorRecord, Match, MatchRules, PlayerScore, Roster, ScoreEvent};
use f13_data::classes::ClassDb;
use std::collections::BTreeMap;

use crate::jason::Jason;
use crate::player::{Player, Unsettled};
use crate::vehicle::Escaped;

/// How long a score event's text stays on the HUD, s.
const POPUP_SECONDS: f32 = 4.0;

/// The match rules and score events from tuning.json.
#[derive(Resource, Clone)]
pub struct MatchTuning {
    pub rules: MatchRules,
    pub events: BTreeMap<String, ScoreEvent>,
}

/// A score event for a counselor: its class name without `_C` (e.g. "StunJason"), at scale 1.
#[derive(Event, Clone, Debug)]
pub struct Score {
    pub who: Entity,
    pub event: &'static str,
}

/// A score event for Jason (e.g. "KillCounselor").
#[derive(Event, Clone, Debug)]
pub struct KillerScore(pub &'static str);

/// The score event a class names in one of its properties (e.g. an item's "PickupScoreEvent"), for a counselor or
/// (`who` None) for Jason.
#[derive(Event, Clone, Debug)]
pub struct ClassScore {
    pub class: String,
    pub property: &'static str,
    pub who: Option<Entity>,
}

impl ClassScore {
    pub fn new(who: Entity, class: &str, property: &'static str) -> Self {
        Self { class: class.to_string(), property, who: Some(who) }
    }

    /// Jason's (a fuse box's DestroyScoreEvent, a kill's KillInfo ScoreEvent).
    pub fn killer(class: &str, property: &'static str) -> Self {
        Self { class: class.to_string(), property, who: None }
    }
}

/// A context kill by Jason (SCGameMode's kill handler FUN_140376a60: the kill's damage type, its KillInfo, scores
/// its ScoreEvent for him: Resourceful, Versatile or Seeker): the kill actor's class and its kill component.
#[derive(Event, Clone, Debug)]
pub struct ContextKillScore {
    pub class: String,
    pub component: &'static str,
}

/// Jason took damage (bHasTakenDamage: his Unscathed).
#[derive(Event, Clone, Copy, Debug)]
pub struct JasonDamaged;

/// One counselor's match: what she scored, what became of her, and her score texts (with their category and the
/// game time they were scored).
#[derive(Clone, Debug, Default)]
pub struct Tally {
    pub score: PlayerScore,
    pub record: CounselorRecord,
    last_health: Option<f32>,
    pub popups: Vec<(String, String, f32)>,
    /// The badges she earned (her score events' CounselorBadge, once each: FUN_1403aca40).
    pub badges: Vec<String>,
}

impl Tally {
    fn award(&mut self, who: Entity, name: &str, scale: f32, events: &BTreeMap<String, ScoreEvent>, now: f32) {
        let Some(event) = events.get(name) else {
            warn!("match: no score event {name}");
            return;
        };
        if let Some(xp) = self.score.award(name, event, scale) {
            let text = event.text(xp);
            info!("match: {who} scored {name}: {text}");
            self.popups.push((text, event.category.clone(), now));
            if let Some(b) = event.counselor_badge.as_ref().filter(|b| !self.badges.contains(b)) {
                info!("match: {who} earned the {b} badge");
                self.badges.push(b.clone());
            }
        }
    }
}

/// The match, and what each counselor and Jason scored in it.
#[derive(Resource, Default)]
pub struct GameMatch {
    pub state: Match,
    /// Each counselor's (by her entity; a client's by her twin's).
    pub counselors: BTreeMap<Entity, Tally>,
    /// The counselor this machine plays.
    pub local: Option<Entity>,
    /// Jason's: his score, his score texts, and whether he took damage.
    pub killer_score: PlayerScore,
    pub killer_popups: Vec<(String, String, f32)>,
    /// His badges (his score events' JasonBadge).
    pub killer_badges: Vec<String>,
    killer_damaged: bool,
}

impl GameMatch {
    fn award(&mut self, who: Entity, name: &str, scale: f32, events: &BTreeMap<String, ScoreEvent>, now: f32) {
        self.counselors.entry(who).or_default().award(who, name, scale, events, now);
    }

    /// The local counselor's.
    pub fn mine(&self) -> Option<&Tally> {
        self.counselors.get(&self.local?)
    }

    fn award_killer(&mut self, name: &str, scale: f32, events: &BTreeMap<String, ScoreEvent>, now: f32) {
        let Some(event) = events.get(name) else {
            warn!("match: no score event {name}");
            return;
        };
        if let Some(xp) = self.killer_score.award(name, event, scale) {
            let text = event.text(xp);
            info!("match: Jason scored {name}: {text}");
            self.killer_popups.push((text, event.category.clone(), now));
            if let Some(b) = event.jason_badge.as_ref().filter(|b| !self.killer_badges.contains(b)) {
                info!("match: Jason earned the {b} badge");
                self.killer_badges.push(b.clone());
            }
        }
    }

    /// The local counselor's score texts so far: (text, EScoreEventCategory name, game time).
    pub fn score_lines(&self) -> &[(String, String, f32)] {
        self.mine().map_or(&[], |t| &t.popups)
    }

    /// Jason's.
    pub fn killer_score_lines(&self) -> &[(String, String, f32)] {
        &self.killer_popups
    }

    /// The results' headline: hers, or (Jason played) what became of the counselors.
    fn outcome_for(&self, killer: bool) -> String {
        if killer {
            let n = self.counselors.len();
            let (dead, escaped) = (self.counselors.values().filter(|t| t.record.dead).count(), self.counselors.values().filter(|t| t.record.escaped).count());
            return match n {
                1 if escaped == 1 => "VANESSA ESCAPED".into(),
                1 if dead == 1 => "VANESSA IS DEAD".into(),
                1 => "VANESSA SURVIVED".into(),
                _ => format!("{dead} OF {n} DEAD, {escaped} ESCAPED"),
            };
        }
        self.outcome().into()
    }

    fn outcome(&self) -> &'static str {
        let record = self.mine().map(|t| t.record).unwrap_or_default();
        if record.escaped {
            "YOU ESCAPED"
        } else if record.dead {
            "YOU DIED"
        } else {
            "YOU SURVIVED"
        }
    }
}

/// The classes' properties, for [`ClassScore`].
#[derive(Resource, Default)]
struct ScoreClasses(Option<ClassDb>);

pub struct MatchPlugin {
    /// Close when the results are over (not under a bot, which decides when the run ends).
    pub leave: bool,
}

#[derive(Resource)]
pub(crate) struct Leave(pub bool);

impl Plugin for MatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMatch>()
            .insert_resource(Leave(self.leave))
            .add_systems(Startup, (load_classes, spawn_hud))
            .add_systems(FixedUpdate, run_match.run_if(crate::net::simulating))
            .add_systems(Update, (update_hud, hold_pawns))
            .add_observer(on_score)
            .add_observer(on_class_score)
            .add_observer(on_killer_score)
            .add_observer(on_context_kill_score)
            .add_observer(|_: On<JasonDamaged>, mut game: ResMut<GameMatch>| game.killer_damaged = true);
    }
}

/// The match is in progress (or past it): the game spawns the pawns as it starts (HandleMatchHasStarted, after the
/// PreMatchIntro), so until then Jason's tick (his ability timers) and their input don't run here and they're hidden
/// ([`hold_pawns`]). True without match rules.
pub fn in_progress(game: Option<Res<GameMatch>>, tuning: Option<Res<MatchTuning>>) -> bool {
    tuning.is_none() || game.is_none_or(|g| !matches!(g.state.state, MatchState::WaitingToStart | MatchState::WaitingPreMatch | MatchState::PreMatchIntro))
}

/// Hidden until the match starts (see [`in_progress`]).
#[derive(Component)]
struct HeldForMatch;

fn hold_pawns(
    mut commands: Commands,
    game: Option<Res<GameMatch>>,
    tuning: Option<Res<MatchTuning>>,
    mut pawns: Query<(Entity, &mut Visibility, Has<HeldForMatch>), Or<(With<Player>, With<Jason>)>>,
) {
    let started = in_progress(game, tuning);
    for (e, mut v, held) in &mut pawns {
        if !started && !held {
            *v = Visibility::Hidden;
            commands.entity(e).insert(HeldForMatch);
        } else if started && held {
            *v = Visibility::Inherited;
            commands.entity(e).remove::<HeldForMatch>();
        }
    }
}

fn load_classes(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let classes = ClassDb::load(&export.0.join("classes.json")).map_err(|e| warn!("match: no classes.json ({e}): no XP for items")).ok();
    commands.insert_resource(ScoreClasses(classes));
}

fn on_score(score: On<Score>, tuning: Option<Res<MatchTuning>>, mut game: ResMut<GameMatch>, time: Res<Time>) {
    let Some(t) = tuning else { return };
    game.award(score.who, score.event, 1.0, &t.events, time.elapsed_secs());
}

fn on_killer_score(score: On<KillerScore>, tuning: Option<Res<MatchTuning>>, mut game: ResMut<GameMatch>, time: Res<Time>) {
    let Some(t) = tuning else { return };
    game.award_killer(score.0, 1.0, &t.events, time.elapsed_secs());
}

fn on_context_kill_score(score: On<ContextKillScore>, tuning: Option<Res<MatchTuning>>, classes: Res<ScoreClasses>, mut game: ResMut<GameMatch>, time: Res<Time>) {
    let (Some(t), Some(db)) = (tuning, &classes.0) else { return };
    let info = db.get(&score.class).and_then(|c| c.components.iter().find(|c| c.name == score.component)).and_then(|c| c.props["KillInfo"].as_str());
    match info.and_then(|i| db.prop(i, "ScoreEvent").as_str()) {
        Some(event) => game.award_killer(event.trim_end_matches("_C"), 1.0, &t.events, time.elapsed_secs()),
        None => warn!("match: no KillInfo ScoreEvent for {}'s {} (re-export: f13-export levels)", score.class, score.component),
    }
}

fn on_class_score(score: On<ClassScore>, tuning: Option<Res<MatchTuning>>, classes: Res<ScoreClasses>, mut game: ResMut<GameMatch>, time: Res<Time>) {
    let (Some(t), Some(db)) = (tuning, &classes.0) else { return };
    if let Some(event) = db.prop(&score.class, score.property).as_str() {
        let name = event.trim_end_matches("_C");
        match score.who {
            Some(who) => game.award(who, name, 1.0, &t.events, time.elapsed_secs()),
            None => game.award_killer(name, 1.0, &t.events, time.elapsed_secs()),
        }
    }
}

/// Runs the match timer once the counselors are in the world, keeps each one's record, and scores the escapes and
/// the match's end.
#[allow(clippy::too_many_arguments)]
fn run_match(
    mut commands: Commands,
    (intro, mut intro_done): (Option<Res<crate::cinematic::IntroEnabled>>, ResMut<crate::cinematic::CinematicDone>),
    time: Res<Time>,
    tuning: Option<Res<MatchTuning>>,
    mut game: ResMut<GameMatch>,
    players: Query<(Entity, &Player, Has<Escaped>, Has<crate::controls::LocalPlayer>), Without<Unsettled>>,
    jason: Query<(), With<Jason>>,
    leave: Res<Leave>,
    host: Option<ResMut<crate::host::HostRequests>>,
    mut exit: MessageWriter<AppExit>,
    (rematch, lobby): (Option<Res<crate::net::Rematch>>, Query<&crate::lobby::NetLobby>),
) {
    let Some(t) = tuning else { return };
    if players.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let g = &mut *game;
    let elapsed = g.state.elapsed_in_progress_time;
    let mut left = 0;
    for (e, p, escaped, local) in &players {
        if local {
            g.local = Some(e);
        }
        let tally = g.counselors.entry(e).or_default();
        let health = p.counselor.health;
        if tally.last_health.is_some_and(|h| health < h) {
            tally.record.taken_damage = true;
        }
        tally.last_health = Some(health);
        if !p.counselor.alive() && !tally.record.dead {
            tally.record.dead = true;
            info!("match: counselor {e} died, {elapsed} s in");
        }
        if escaped && !tally.record.escaped && !tally.record.dead {
            tally.record.escaped = true;
            info!("match: counselor {e} escaped, {elapsed} s in");
            for (name, scale) in escape_awards(&tally.record, elapsed) {
                tally.award(e, name, scale, &t.events, now);
            }
        }
        left += u32::from(p.counselor.alive() && !tally.record.escaped);
    }
    let roster = Roster {
        counselors_left: left,
        // Jason can't die yet.
        killer_alive: (!jason.is_empty()).then_some(true),
    };
    let mut entered = g.state.advance(time.delta_secs(), &t.rules, &roster);
    // The intro's IntroSequenceCompleted: on to the match; the outro's end: on to the results.
    if g.state.state == MatchState::PreMatchIntro && std::mem::take(&mut intro_done.0) {
        entered.extend(g.state.start(&t.rules));
    }
    if g.state.state == MatchState::PostMatchOutro && std::mem::take(&mut intro_done.0) {
        entered.extend(g.state.end(&t.rules));
    }
    // The host's start and end (host.rs).
    if let Some(mut h) = host {
        if std::mem::take(&mut h.start) {
            entered.extend(g.state.start(&t.rules));
        }
        if std::mem::take(&mut h.end) {
            entered.extend(g.state.end(&t.rules));
        }
    }
    let mut i = 0;
    while i < entered.len() {
        match entered[i] {
            // The level intro (cinematic.rs) when there is one to play; else straight on.
            MatchState::PreMatchIntro if intro.is_some() => commands.insert_resource(crate::cinematic::PlayCinematic(crate::cinematic::INTRO)),
            MatchState::PreMatchIntro => {
                let started = g.state.start(&t.rules);
                entered.extend(started);
            }
            MatchState::InProgress => info!("match: started ({} s round)", t.rules.round_time),
            MatchState::WaitingPostMatchOutro => {
                info!("match: over after {} s: {}", g.state.elapsed_in_progress_time, g.outcome());
                let all_escaped = g.counselors.values().all(|t| t.record.escaped);
                let elapsed = g.state.elapsed_in_progress_time;
                for (&e, tally) in g.counselors.iter_mut() {
                    for (name, scale) in end_of_match_awards(&tally.record, all_escaped, elapsed) {
                        tally.award(e, name, scale, &t.events, now);
                    }
                }
                if !jason.is_empty() {
                    let (spawned, dead) = (g.counselors.len() as u32, g.counselors.values().filter(|t| t.record.dead).count() as u32);
                    for (name, scale) in killer_end_of_match_awards(spawned, dead, g.killer_damaged) {
                        g.award_killer(name, scale, &t.events, now);
                    }
                }
            }
            // The outro (cinematic.rs), with Jason alive (he can't die yet: there whenever he's in the match).
            MatchState::PostMatchOutro if intro.is_some() && !jason.is_empty() => {
                commands.insert_resource(crate::cinematic::PlayCinematic(crate::cinematic::OUTRO))
            }
            MatchState::PostMatchOutro => {
                let ended = g.state.end(&t.rules);
                entered.extend(ended);
            }
            MatchState::WaitingPostMatch => {
                for (e, tally) in &g.counselors {
                    let list: Vec<String> = tally.score.awards.iter().map(|a| format!("{} {}", a.event, a.xp)).collect();
                    info!("match: results: counselor {e}: {} XP ({})", tally.score.total(), list.join(", "));
                }
                if !g.killer_score.awards.is_empty() {
                    let list: Vec<String> = g.killer_score.awards.iter().map(|a| format!("{} {}", a.event, a.xp)).collect();
                    info!("match: Jason's results: {} XP ({})", g.killer_score.total(), list.join(", "));
                }
            }
            MatchState::LeavingMap if leave.0 => {
                // A lobby's server: a new lobby (a new world: no --seed), last match's Jason passed on.
                if rematch.is_some() {
                    let args: Vec<String> = std::env::args().skip(1).collect();
                    let last = lobby.iter().next().and_then(|l| l.jason);
                    match crate::net::relaunch(&args, &["--seed", "--last-jason"], &last.map(|j| ("--last-jason", j.to_string())).into_iter().collect::<Vec<_>>()) {
                        Ok(_) => info!("match: back to the lobby"),
                        Err(e) => warn!("match: can't start the lobby again: {e}"),
                    }
                }
                info!("match: leaving");
                exit.write(AppExit::Success);
            }
            _ => {}
        }
        i += 1;
    }
}

#[derive(Component)]
struct MatchClock;

#[derive(Component)]
struct ScorePopups;

#[derive(Component)]
struct MatchOverlay;

#[derive(Component)]
struct MatchOverlayText;

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(26.0), ..default() },
        TextLayout::justify(Justify::Right),
        // Top right: the debug HUD takes the top left.
        Node { position_type: PositionType::Absolute, top: Val::Px(10.0), right: Val::Px(24.0), ..default() },
        MatchClock,
    ));
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(22.0), ..default() },
        TextColor(Color::srgb(1.0, 0.85, 0.4)),
        TextLayout::justify(Justify::Right),
        Node { position_type: PositionType::Absolute, top: Val::Percent(30.0), right: Val::Px(24.0), ..default() },
        ScorePopups,
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
            GlobalZIndex(10),
            Visibility::Hidden,
            MatchOverlay,
        ))
        .with_child((Text::new(""), TextFont { font_size: FontSize::Px(24.0), ..default() }, TextLayout::justify(Justify::Center), MatchOverlayText));
}

fn update_hud(
    time: Res<Time>,
    game: Res<GameMatch>,
    tuning: Option<Res<MatchTuning>>,
    mut clock: Single<&mut Text, (With<MatchClock>, Without<ScorePopups>, Without<MatchOverlayText>)>,
    mut popups: Single<&mut Text, (With<ScorePopups>, Without<MatchClock>, Without<MatchOverlayText>)>,
    mut overlay: Single<(&mut Visibility, &mut BackgroundColor), With<MatchOverlay>>,
    mut overlay_text: Single<&mut Text, (With<MatchOverlayText>, Without<MatchClock>, Without<ScorePopups>)>,
    game_hud: Option<Res<crate::ui::Hud>>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
    names: Query<&Name>,
) {
    let m = &game.state;
    // The played character's results: Jason's or hers.
    let killer = play_jason.is_some();
    let empty = PlayerScore::default();
    let score = if killer { &game.killer_score } else { game.mine().map_or(&empty, |t| &t.score) };
    // The game's HUD has its own timer (ui.rs).
    let clock_text = if m.state == MatchState::InProgress && game_hud.is_none() { format!("{}:{:02}", m.remaining_time / 60, m.remaining_time % 60) } else { String::new() };
    if clock.0 != clock_text {
        clock.0 = clock_text;
    }
    let now = time.elapsed_secs();
    let shown: Vec<&str> = game.score_lines().iter().filter(|(_, _, at)| game_hud.is_none() && now - at < POPUP_SECONDS).map(|(t, _, _)| t.as_str()).collect();
    let popup_text = shown.join("\n");
    if popups.0 != popup_text {
        popups.0 = popup_text;
    }
    let (ref mut vis, ref mut bg) = *overlay;
    let text = match m.state {
        MatchState::WaitingPostMatchOutro | MatchState::PostMatchOutro => {
            bg.0 = Color::srgba(0.0, 0.0, 0.0, 0.25);
            format!("MATCH OVER\n{}", game.outcome_for(killer))
        }
        MatchState::WaitingPostMatch | MatchState::LeavingMap => {
            bg.0 = Color::srgba(0.0, 0.0, 0.0, 0.92);
            let mut s = format!("{}\nmatch time {}:{:02}\n\n", game.outcome_for(killer), m.elapsed_in_progress_time / 60, m.elapsed_in_progress_time % 60);
            for a in &score.awards {
                let line = tuning.as_ref().and_then(|t| t.events.get(&a.event)).map_or_else(|| format!("{} +{}", a.event, a.xp), |e| e.text(a.xp));
                s.push_str(&line);
                s.push('\n');
            }
            s.push_str(&format!("\nTOTAL XP  {}\n", score.total()));
            let badges = if killer { game.killer_badges.clone() } else { game.mine().map_or(vec![], |t| t.badges.clone()) };
            if !badges.is_empty() {
                s.push_str(&format!("BADGES  {}\n", badges.join(", ")));
            }
            // Everyone's (SCMatchEndLobbyWidget's players), with more than one counselor or as Jason.
            if game.counselors.len() > 1 || (killer && !game.counselors.is_empty()) {
                s.push('\n');
                for (e, t) in &game.counselors {
                    let name = names.get(*e).map_or("counselor".to_string(), |n| n.to_string());
                    let fate = if t.record.escaped { "escaped" } else if t.record.dead { "dead" } else { "survived" };
                    s.push_str(&format!("{name} ({fate})  {} XP  {}\n", t.score.total(), t.badges.join(", ")));
                }
                s.push_str(&format!("Jason  {} XP  {}\n", game.killer_score.total(), game.killer_badges.join(", ")));
            }
            s.push_str(&format!("\nclosing in {} s", m.remaining_time));
            s
        }
        _ => String::new(),
    };
    let visible = if text.is_empty() { Visibility::Hidden } else { Visibility::Inherited };
    if **vis != visible {
        **vis = visible;
    }
    if overlay_text.0 != text {
        overlay_text.0 = text;
    }
}
