//! `--perf`: where a frame's time goes. Bevy's own diagnostics (frame time, entity count, GPU pass times) plus the
//! time each top-level schedule of the main world takes (an empty marker schedule between each pair stamps the time),
//! and the render world's system sets (a stamp system in each). Logged every few seconds as averages. Built with
//! `--features profile`, also the costliest systems (Bevy's tracing spans).

use bevy::app::{First, Last, MainScheduleOrder, PostUpdate, PreUpdate, RunFixedMainLoop, SpawnScene, Update};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};
use std::time::{Duration, Instant};

const LOG_EVERY: Duration = Duration::from_secs(5);

/// Marks the end of the main schedule it follows (and the start of the frame, for 0).
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Stamp(usize);

/// The main world's schedules, in the order they run; each is timed from the stamp before it to the one after.
const MAIN: [&str; 7] = ["First", "PreUpdate", "FixedUpdate (RunFixedMainLoop)", "Update", "SpawnScene", "PostUpdate", "Last"];
/// The render world's system sets stamped, in order: each section runs from its stamp to the next.
const RENDER: [&str; 6] = ["ExtractCommands..CreateViews (prepare assets, meshes)", "Specialize..PrepareViews", "Queue", "PhaseSort", "Prepare", "Render (encode, submit)"];

#[derive(Resource)]
struct Times {
    names: &'static [&'static str],
    last: Option<Instant>,
    at: usize,
    sums: Vec<f64>,
    frames: u32,
    since: Instant,
}

impl Times {
    fn new(names: &'static [&'static str]) -> Self {
        Self { names, last: None, at: 0, sums: vec![0.0; names.len()], frames: 0, since: Instant::now() }
    }

    /// Stamp `i`: the time since stamp `i - 1` goes to section `i - 1`; stamp 0 starts a frame.
    fn stamp(&mut self, i: usize, what: &str) {
        let now = Instant::now();
        if i == 0 {
            self.frames += 1;
        } else if let Some(last) = self.last.filter(|_| self.at == i - 1) {
            self.sums[i - 1] += (now - last).as_secs_f64() * 1000.0;
        }
        self.last = Some(now);
        self.at = i;
        if i == 0 && self.since.elapsed() >= LOG_EVERY && self.frames > 1 {
            let n = (self.frames - 1) as f64;
            let total: f64 = self.sums.iter().sum::<f64>() / n;
            let parts: Vec<String> = self.names.iter().zip(&self.sums).map(|(name, s)| format!("{name} {:.2}", s / n)).collect();
            info!("perf: {what} {total:.2} ms per frame: {}", parts.join(", "));
            self.sums.iter_mut().for_each(|s| *s = 0.0);
            self.frames = 0;
            self.since = Instant::now();
        }
    }
}

#[derive(Resource)]
struct MainTimes(Times);
#[derive(Resource)]
struct RenderTimes(Times);

pub struct PerfPlugin;

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            bevy::diagnostic::LogDiagnosticsPlugin { wait_duration: LOG_EVERY, ..default() },
            bevy::diagnostic::EntityCountDiagnosticsPlugin::default(),
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
        ));
        app.insert_resource(MainTimes(Times::new(&MAIN)));
        #[cfg(feature = "profile")]
        app.add_systems(Last, systems::log_top);
        let labels: [bevy::ecs::intern::Interned<dyn ScheduleLabel>; 7] =
            [First.intern(), PreUpdate.intern(), RunFixedMainLoop.intern(), Update.intern(), SpawnScene.intern(), PostUpdate.intern(), Last.intern()];
        {
            let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
            order.insert_before(First, Stamp(0));
            for (i, label) in labels.iter().enumerate() {
                let at = order.labels.iter().position(|l| l == label).expect("a main schedule");
                order.labels.insert(at + 1, Stamp(i + 1).intern());
            }
        }
        for i in 0..=labels.len() {
            app.add_systems(Stamp(i), move |mut t: ResMut<MainTimes>| t.0.stamp(i, "main world"));
        }
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render.insert_resource(RenderTimes(Times::new(&RENDER)));
        let sets = [RenderSystems::ExtractCommands, RenderSystems::Specialize, RenderSystems::Queue, RenderSystems::PhaseSort, RenderSystems::Prepare, RenderSystems::Render, RenderSystems::Cleanup];
        for (i, set) in sets.into_iter().enumerate() {
            // Stamp 0 starts a frame; the last one ends its last section.
            render.add_systems(Render, (move |mut t: ResMut<RenderTimes>| t.0.stamp(i, "render world")).in_set(set));
        }
    }
}

/// With the `profile` feature: Bevy's per-system tracing spans, totalled by system name (main and render world alike)
/// and logged as the costliest systems per frame.
#[cfg(feature = "profile")]
pub mod systems {
    use bevy::log::tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};
    use bevy::log::BoxedLayer;
    use bevy::log::tracing::{field::Field, field::Visit, span, Subscriber};
    use bevy::prelude::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Instant;

    /// System name -> (total seconds, runs) since the last log.
    static TOTALS: Mutex<Option<HashMap<String, (f64, u32)>>> = Mutex::new(None);

    struct Name(String);
    struct Entered(Instant);

    struct NameVisitor(Option<String>);
    impl Visit for NameVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "name" {
                self.0 = Some(format!("{value:?}").trim_matches('"').to_string());
            }
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "name" {
                self.0 = Some(value.to_string());
            }
        }
    }

    struct SystemTimes;

    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SystemTimes {
        fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
            if attrs.metadata().name() != "system" {
                return;
            }
            let mut v = NameVisitor(None);
            attrs.record(&mut v);
            if let (Some(name), Some(span)) = (v.0, ctx.span(id)) {
                span.extensions_mut().insert(Name(name));
            }
        }
        fn on_enter(&self, id: &span::Id, ctx: Context<'_, S>) {
            if let Some(span) = ctx.span(id) {
                let mut ext = span.extensions_mut();
                if ext.get_mut::<Name>().is_some() {
                    ext.replace(Entered(Instant::now()));
                }
            }
        }
        fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
            let Some(span) = ctx.span(id) else { return };
            let ext = span.extensions();
            let (Some(name), Some(at)) = (ext.get::<Name>(), ext.get::<Entered>()) else { return };
            let dt = at.0.elapsed().as_secs_f64();
            let mut totals = TOTALS.lock().unwrap();
            let e = totals.get_or_insert_with(HashMap::new).entry(name.0.clone()).or_default();
            e.0 += dt;
            e.1 += 1;
        }
    }

    /// LogPlugin's custom layer: timing systems only with --perf.
    pub fn layer(_app: &mut App) -> Option<BoxedLayer> {
        std::env::args().any(|a| a == "--perf").then(|| Box::new(SystemTimes) as BoxedLayer)
    }

    /// Logs the costliest systems (ms per frame) every few seconds.
    pub fn log_top(mut frames: Local<u32>, mut since: Local<Option<Instant>>) {
        *frames += 1;
        let since = since.get_or_insert_with(Instant::now);
        if since.elapsed() < super::LOG_EVERY {
            return;
        }
        let Some(totals) = TOTALS.lock().unwrap().take() else { return };
        let mut v: Vec<_> = totals.into_iter().collect();
        v.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        let n = (*frames).max(1) as f64;
        let lines: Vec<String> = v.iter().take(25).map(|(name, (s, runs))| format!("  {:6.3} ms  {:4.1}x  {name}", s * 1000.0 / n, *runs as f64 / n)).collect();
        info!("perf: costliest systems per frame (both worlds; parallel, so they overlap):\n{}", lines.join("\n"));
        *frames = 0;
        *since = Instant::now();
    }
}
