//! Bevy integration for f13-core: components wrap the engine-independent rules, systems tick them on
//! `FixedUpdate`, and results come out as messages for HUD, audio and networking to react to.

use bevy::prelude::*;
use f13_core::fear::{Fear, FearContext, FearEventData};
use f13_core::killer_abilities::{AbilityEvent, Blockers, KillerAbilities};
use f13_core::stats::{StatModifiedValue, Stats};

/// Runs on the server (authority) or a client. Set by the netcode layer; defaults to server for offline play.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Authority(pub bool);

impl Default for Authority {
    fn default() -> Self {
        Authority(true)
    }
}

#[derive(Component, Debug)]
pub struct KillerAbilitiesComp(pub KillerAbilities);

/// Use-blockers for a killer this frame, written by movement/combat systems.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct KillerBlockers(pub Blockers);

#[derive(Message, Debug, Clone, Copy)]
pub struct KillerAbilityMessage {
    pub killer: Entity,
    pub event: AbilityEvent,
}

/// Fear event identity: the asset name of the fear event (e.g. "JasonNearFearEvent").
pub type FearEventId = String;

#[derive(Component, Debug)]
pub struct CounselorFear(pub Fear<FearEventId>);

#[derive(Component, Debug, Clone)]
pub struct CounselorStats {
    pub stats: Stats,
    pub fear_modifier: StatModifiedValue,
    /// Sum of perk fear-rate modifiers (every event), and those for an active event only.
    pub perk_fear_rate: f32,
    pub perk_fear_active: f32,
}

/// Fired by the server when a counselor's fear reaches 100.
#[derive(Message, Debug, Clone, Copy)]
pub struct MaxFearReached(pub Entity);

/// Push or pop a fear event on a counselor (sent by perception/environment systems).
#[derive(Message, Debug, Clone)]
pub enum FearEventMessage {
    Push { counselor: Entity, id: FearEventId, data: FearEventData, distance: f32 },
    Pop { counselor: Entity, id: FearEventId, data: FearEventData },
}

pub struct F13GameplayPlugin;

impl Plugin for F13GameplayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Authority>()
            .add_message::<KillerAbilityMessage>()
            .add_message::<MaxFearReached>()
            .add_message::<FearEventMessage>()
            .add_systems(FixedUpdate, (tick_killer_abilities, (apply_fear_events, tick_fear).chain()));
    }
}

fn tick_killer_abilities(
    time: Res<Time>,
    auth: Res<Authority>,
    mut q: Query<(Entity, &mut KillerAbilitiesComp, Option<&KillerBlockers>)>,
    mut out: MessageWriter<KillerAbilityMessage>,
) {
    let dt = time.delta_secs();
    let mut events = Vec::new();
    for (e, mut k, blockers) in &mut q {
        let b = blockers.map(|b| b.0).unwrap_or_default();
        k.0.update(dt, auth.0, &b, &mut events);
        for ev in events.drain(..) {
            out.write(KillerAbilityMessage { killer: e, event: ev });
        }
    }
}

fn apply_fear_events(mut msgs: MessageReader<FearEventMessage>, mut q: Query<&mut CounselorFear>) {
    for m in msgs.read() {
        match m {
            FearEventMessage::Push { counselor, id, data, distance } => {
                if let Ok(mut f) = q.get_mut(*counselor) {
                    f.0.push(id.clone(), data, *distance);
                }
            }
            FearEventMessage::Pop { counselor, id, data } => {
                if let Ok(mut f) = q.get_mut(*counselor) {
                    f.0.pop(id.clone(), data);
                }
            }
        }
    }
}

fn tick_fear(
    time: Res<Time>,
    auth: Res<Authority>,
    mut q: Query<(Entity, &mut CounselorFear, &CounselorStats)>,
    mut out: MessageWriter<MaxFearReached>,
) {
    let dt = time.delta_secs();
    // Perk cap bonuses for specific events are not modelled yet: cap = max_fear_cap.
    let cap = |_: &FearEventId, d: &FearEventData| d.max_fear_cap;
    for (e, mut fear, s) in &mut q {
        let rate = s.fear_modifier.value(&s.stats, true) + s.perk_fear_rate;
        let ctx = FearContext { rate, rate_active: s.perk_fear_active, cap: &cap, has_authority: auth.0 };
        if fear.0.tick(dt, &ctx) {
            out.write(MaxFearReached(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use f13_core::killer_abilities::{Ability, KillerTuning};
    use std::time::Duration;

    #[test]
    fn plugin_unlocks_morph_after_one_second() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, F13GameplayPlugin));
        let tuning = KillerTuning {
            unlock_time: [1.0, 45.0, 90.0, 120.0],
            recharge_time: [30.0, 15.0, 30.0, 30.0],
            raged_recharge_modifier: 1.5,
            rage_unlock_time: 900.0,
        };
        let jason = app.world_mut().spawn(KillerAbilitiesComp(KillerAbilities::new(tuning, None))).id();
        // Drive the schedule by hand for 1.1 s. Outside the fixed main loop, systems read the generic `Time`,
        // so advance that (normally Bevy copies `Time<Fixed>` into it before each FixedUpdate run).
        for _ in 0..66 {
            app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f64(1.0 / 60.0));
            app.world_mut().run_schedule(FixedUpdate);
        }
        let k = app.world().get::<KillerAbilitiesComp>(jason).unwrap();
        assert!(k.0.is_unlocked(Ability::Morph));
        assert!(!k.0.is_unlocked(Ability::Sense));
    }
}
