use std::time::{Duration, Instant};

use ard_ecs::prelude::*;

use crate::prelude::{App, AppBuilder, Plugin};

const DEFAULT_FIXED_TICK_RATE: Duration = Duration::from_millis(33);

/// Propogated once at first dispatch when using `ArdCore`.
#[derive(Debug, Default, Event, Copy, Clone)]
pub struct Start;

/// Signals to `ArdCore` that ticks should stop being propogated.
///
/// # Note
/// This is NOT the last event to be sent. If you want to handle the last event of the engine
/// then handle the `Stopping` event.
#[derive(Debug, Event, Copy, Clone)]
pub struct Stop;

/// Signal from `ArdCore` that the engine is stopping. When using `ArdCore`, this is the last
/// event to be submitted.
#[derive(Debug, Event, Copy, Clone)]
pub struct Stopping;

/// Signals one iteration of the game loop. The duration should be how much time has elapsed since
/// the last tick.
#[derive(Debug, Default, Event, Copy, Clone)]
pub struct Tick(pub Duration);

/// Occurs every iteration after `Tick`.
#[derive(Debug, Default, Event, Copy, Clone)]
pub struct PostTick(pub Duration);

/// Propogated at a fixed rate set during `ArdCore` creation. The default value is
/// `DEFAULT_FIXED_TICK_RATE`. The duration is the fixed time between propogations.
///
/// # Dispatch Slower than Fixed Rate
/// If the dispatcher runs slower than the fixed time, additionally propogations will not be sent.
/// For example, if your dispatcher is iterating half as fast as the fixed rate, only one event
/// will be sent per iteration; NOT two.
#[derive(Debug, Default, Event, Copy, Clone)]
pub struct FixedTick(pub Duration);

/// Current state of the core.
#[derive(Debug, Resource)]
pub struct ArdCoreState {
    stopping: bool,
}

/// The base engine plugin.
///
/// A default runner is used which, every iteration of dispatch, generates new `Tick` and
/// `PostTick` events until the `Stop` event is received. Additionally, a fixed rate
/// `FixedTick` event is propogated. The order of propogation is as follows:
///
/// `Tick` → `PostTick` → `FixedTick`
pub struct ArdCorePlugin;

pub struct ArdCore {
    fixed_rate: Duration,
    fixed_timer: Duration,
}

impl SystemState for ArdCore {
    type Data = ();
    type Resources = (Write<ArdCoreState>,);
}

impl Default for ArdCore {
    fn default() -> Self {
        ArdCore {
            fixed_rate: DEFAULT_FIXED_TICK_RATE,
            fixed_timer: Duration::ZERO,
        }
    }
}

impl ArdCoreState {
    #[inline]
    pub fn stopping(&self) -> bool {
        self.stopping
    }
}

impl ArdCore {
    pub fn new(fixed_rate: Duration) -> Self {
        ArdCore {
            fixed_rate,
            fixed_timer: Duration::ZERO,
        }
    }

    pub fn tick(&mut self, ctx: Context<Self>, tick: Tick) {
        let duration = tick.0;

        if !ctx.resources.0.unwrap().stopping {
            // Post tick
            ctx.events.submit(PostTick(duration));

            // Check for fixed tick
            self.fixed_timer += duration;
            if self.fixed_timer >= self.fixed_rate {
                self.fixed_timer = Duration::ZERO;
                ctx.events.submit(FixedTick(self.fixed_rate));
            }
        }
    }

    pub fn stop(&mut self, ctx: Context<Self>, _: Stop) {
        ctx.resources.0.unwrap().stopping = true;
        ctx.events.submit(Stopping);
    }
}

#[allow(clippy::from_over_into)]
impl Into<System> for ArdCore {
    fn into(self) -> System {
        SystemBuilder::new(self)
            .with_handler(ArdCore::tick)
            .with_handler(ArdCore::stop)
            .build()
    }
}

impl Plugin for ArdCorePlugin {
    fn build(&mut self, app: &mut AppBuilder) {
        // Use half the number of threads on the system so we don't pin the CPU to 100%
        rayon::ThreadPoolBuilder::new()
            .num_threads((num_cpus::get() / 2).max(1))
            .build_global()
            .unwrap();

        app.add_system(ArdCore::default());
        app.add_resource(ArdCoreState { stopping: false });
        app.add_event(Start);
        app.with_runner(default_core_runner);
    }
}

fn default_core_runner(mut app: App) {
    app.run_startups();

    let mut last = Instant::now();
    while !app.resources.get::<ArdCoreState>().unwrap().stopping {
        // Submit tick event
        let now = Instant::now();
        app.dispatcher.submit(Tick(now.duration_since(last)));
        last = now;

        // Dispatch
        app.dispatcher.run(&mut app.world, &app.resources);
    }

    // Handle `Stopping` event
    app.dispatcher.run(&mut app.world, &app.resources);
}
