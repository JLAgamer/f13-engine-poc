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
//! - **HUD**: each award shows as its description ("Arm Yourself +10") for a few seconds, as the game's
//!   ClientDisplayScoreEvent does. The display time is this port's (4 s), not the widget's.

use bevy::prelude::*;
pub use f13_core::game_mode::MatchState;
use f13_core::game_mode::{end_of_match_awards, escape_awards, CounselorRecord, Match, MatchRules, PlayerScore, Roster, ScoreEvent};
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

/// A score event for the counselor: its class name without `_C` (e.g. "StunJason"), at scale 1.
#[derive(Event, Clone, Debug)]
pub struct Score(pub &'static str);

/// The score event a class names in one of its properties (e.g. an item's "PickupScoreEvent").
#[derive(Event, Clone, Debug)]
pub struct ClassScore {
    pub class: String,
    pub property: &'static str,
}

impl ClassScore {
    pub fn new(class: &str, property: &'static str) -> Self {
        Self { class: class.to_string(), property }
    }
}

/// The match, and what the counselor scored in it.
#[derive(Resource, Default)]
pub struct GameMatch {
    pub state: Match,
    pub score: PlayerScore,
    pub record: CounselorRecord,
    last_health: Option<f32>,
    /// Score texts on the HUD, with the game time they were scored.
    popups: Vec<(String, f32)>,
}

impl GameMatch {
    fn award(&mut self, name: &str, scale: f32, events: &BTreeMap<String, ScoreEvent>, now: f32) {
        let Some(event) = events.get(name) else {
            warn!("match: no score event {name}");
            return;
        };
        if let Some(xp) = self.score.award(name, event, scale) {
            let text = event.text(xp);
            info!("match: scored {name}: {text}");
            self.popups.push((text, now));
        }
    }

    fn outcome(&self) -> &'static str {
        if self.record.escaped {
            "YOU ESCAPED"
        } else if self.record.dead {
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
struct Leave(bool);

impl Plugin for MatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMatch>()
            .insert_resource(Leave(self.leave))
            .add_systems(Startup, (load_classes, spawn_hud))
            .add_systems(FixedUpdate, run_match)
            .add_systems(Update, update_hud)
            .add_observer(on_score)
            .add_observer(on_class_score);
    }
}

fn load_classes(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let classes = ClassDb::load(&export.0.join("classes.json")).map_err(|e| warn!("match: no classes.json ({e}): no XP for items")).ok();
    commands.insert_resource(ScoreClasses(classes));
}

fn on_score(score: On<Score>, tuning: Option<Res<MatchTuning>>, mut game: ResMut<GameMatch>, time: Res<Time>) {
    let Some(t) = tuning else { return };
    game.award(score.0, 1.0, &t.events, time.elapsed_secs());
}

fn on_class_score(score: On<ClassScore>, tuning: Option<Res<MatchTuning>>, classes: Res<ScoreClasses>, mut game: ResMut<GameMatch>, time: Res<Time>) {
    let (Some(t), Some(db)) = (tuning, &classes.0) else { return };
    if let Some(event) = db.prop(&score.class, score.property).as_str() {
        game.award(event.trim_end_matches("_C"), 1.0, &t.events, time.elapsed_secs());
    }
}

/// Runs the match timer once she's in the world, keeps her record, and scores her escape and the match's end.
fn run_match(
    time: Res<Time>,
    tuning: Option<Res<MatchTuning>>,
    mut game: ResMut<GameMatch>,
    player: Query<&Player, Without<Unsettled>>,
    jason: Query<(), With<Jason>>,
    escaped: Option<Res<Escaped>>,
    leave: Res<Leave>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(t) = tuning else { return };
    let Ok(p) = player.single() else { return };
    let now = time.elapsed_secs();
    let g = &mut *game;
    let health = p.counselor.health;
    if g.last_health.is_some_and(|h| health < h) {
        g.record.taken_damage = true;
    }
    g.last_health = Some(health);
    if !p.counselor.alive() && !g.record.dead {
        g.record.dead = true;
        info!("match: Vanessa died, {} s in", g.state.elapsed_in_progress_time);
    }
    if escaped.is_some() && !g.record.escaped && !g.record.dead {
        g.record.escaped = true;
        info!("match: Vanessa escaped, {} s in", g.state.elapsed_in_progress_time);
        for (name, scale) in escape_awards(&g.record, g.state.elapsed_in_progress_time) {
            g.award(name, scale, &t.events, now);
        }
    }
    let roster = Roster {
        counselors_left: u32::from(p.counselor.alive() && !g.record.escaped),
        // Jason can't die yet.
        killer_alive: (!jason.is_empty()).then_some(true),
    };
    let mut entered = g.state.advance(time.delta_secs(), &t.rules, &roster);
    let mut i = 0;
    while i < entered.len() {
        match entered[i] {
            MatchState::PreMatchIntro => {
                let started = g.state.start(&t.rules);
                entered.extend(started);
            }
            MatchState::InProgress => info!("match: started ({} s round)", t.rules.round_time),
            MatchState::WaitingPostMatchOutro => {
                info!("match: over after {} s: {}", g.state.elapsed_in_progress_time, g.outcome());
                // One counselor: every counselor escaped if she did.
                for (name, scale) in end_of_match_awards(&g.record, g.record.escaped, g.state.elapsed_in_progress_time) {
                    g.award(name, scale, &t.events, now);
                }
            }
            MatchState::PostMatchOutro => {
                let ended = g.state.end(&t.rules);
                entered.extend(ended);
            }
            MatchState::WaitingPostMatch => {
                let list: Vec<String> = g.score.awards.iter().map(|a| format!("{} {}", a.event, a.xp)).collect();
                info!("match: results: {} XP ({})", g.score.total(), list.join(", "));
            }
            MatchState::LeavingMap if leave.0 => {
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
) {
    let m = &game.state;
    // The game's HUD has its own timer (ui.rs).
    let clock_text = if m.state == MatchState::InProgress && game_hud.is_none() { format!("{}:{:02}", m.remaining_time / 60, m.remaining_time % 60) } else { String::new() };
    if clock.0 != clock_text {
        clock.0 = clock_text;
    }
    let now = time.elapsed_secs();
    let shown: Vec<&str> = game.popups.iter().filter(|(_, at)| now - at < POPUP_SECONDS).map(|(t, _)| t.as_str()).collect();
    let popup_text = shown.join("\n");
    if popups.0 != popup_text {
        popups.0 = popup_text;
    }
    let (ref mut vis, ref mut bg) = *overlay;
    let text = match m.state {
        MatchState::WaitingPostMatchOutro | MatchState::PostMatchOutro => {
            bg.0 = Color::srgba(0.0, 0.0, 0.0, 0.25);
            format!("MATCH OVER\n{}", game.outcome())
        }
        MatchState::WaitingPostMatch | MatchState::LeavingMap => {
            bg.0 = Color::srgba(0.0, 0.0, 0.0, 0.92);
            let mut s = format!("{}\nmatch time {}:{:02}\n\n", game.outcome(), m.elapsed_in_progress_time / 60, m.elapsed_in_progress_time % 60);
            for a in &game.score.awards {
                let line = tuning.as_ref().and_then(|t| t.events.get(&a.event)).map_or_else(|| format!("{} +{}", a.event, a.xp), |e| e.text(a.xp));
                s.push_str(&line);
                s.push('\n');
            }
            s.push_str(&format!("\nTOTAL XP  {}\n\nclosing in {} s", game.score.total(), m.remaining_time));
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
