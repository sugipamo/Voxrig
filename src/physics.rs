//! Player motion state, controls, collision geometry, and timing metrics.

use crate::Player;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// State and protocol data represented by `ControlState`.
pub struct ControlState {
    /// The `forward` value.
    pub forward: bool,
    /// Latest observation's finite locomotion budget; None retains held-key behavior.
    /// The connection actor alone consumes this duration at physics boundaries.
    pub movement_press_duration: Option<Duration>,
    /// The `back` value.
    pub back: bool,
    /// The `left` value.
    pub left: bool,
    /// The `right` value.
    pub right: bool,
    /// The `jump` value.
    pub jump: bool,
    /// The `sprint` value.
    pub sprint: bool,
    /// The `sneak` value.
    pub sneak: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// State and protocol data represented by `VehicleControl`.
pub struct VehicleControl {
    /// Left/right input in the vanilla range `-1.0..=1.0`.
    pub sideways: f32,
    /// Back/forward input in the vanilla range `-1.0..=1.0`.
    pub forward: f32,
    /// The `jump` value.
    pub jump: bool,
    /// The `dismount` value.
    pub dismount: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// State and protocol data represented by `VehiclePose`.
pub struct VehiclePose {
    /// The `position` value.
    pub position: Vec3,
    /// The `yaw` value.
    pub yaw: f32,
    /// The `pitch` value.
    pub pitch: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// State and protocol data represented by `Vec3`.
pub struct Vec3 {
    /// The `x` value.
    pub x: f64,
    /// The `y` value.
    pub y: f64,
    /// The `z` value.
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// State and protocol data represented by `Aabb`.
pub struct Aabb {
    /// The `min_x` value.
    pub min_x: f64,
    /// The `min_y` value.
    pub min_y: f64,
    /// The `min_z` value.
    pub min_z: f64,
    /// The `max_x` value.
    pub max_x: f64,
    /// The `max_y` value.
    pub max_y: f64,
    /// The `max_z` value.
    pub max_z: f64,
}

impl Aabb {
    /// Performs the `player` operation.
    pub fn player(x: f64, y: f64, z: f64) -> Self {
        Self {
            min_x: x - 0.3,
            min_y: y,
            min_z: z - 0.3,
            max_x: x + 0.3,
            max_y: y + 1.8,
            max_z: z + 0.3,
        }
    }
    /// Performs the `block` operation.
    pub fn block(x: i32, y: i32, z: i32) -> Self {
        Self {
            min_x: f64::from(x),
            min_y: f64::from(y),
            min_z: f64::from(z),
            max_x: f64::from(x + 1),
            max_y: f64::from(y + 1),
            max_z: f64::from(z + 1),
        }
    }
    /// Performs the `expand` operation.
    pub fn expand(self, movement: Vec3) -> Self {
        Self {
            min_x: self.min_x + movement.x.min(0.0),
            min_y: self.min_y + movement.y.min(0.0),
            min_z: self.min_z + movement.z.min(0.0),
            max_x: self.max_x + movement.x.max(0.0),
            max_y: self.max_y + movement.y.max(0.0),
            max_z: self.max_z + movement.z.max(0.0),
        }
    }
    /// Performs the `offset` operation.
    pub fn offset(self, movement: Vec3) -> Self {
        Self {
            min_x: self.min_x + movement.x,
            min_y: self.min_y + movement.y,
            min_z: self.min_z + movement.z,
            max_x: self.max_x + movement.x,
            max_y: self.max_y + movement.y,
            max_z: self.max_z + movement.z,
        }
    }
    pub(crate) fn clip_y(self, obstacle: Self, mut dy: f64) -> f64 {
        if obstacle.max_x > self.min_x
            && obstacle.min_x < self.max_x
            && obstacle.max_z > self.min_z
            && obstacle.min_z < self.max_z
        {
            if dy > 0.0 && self.max_y <= obstacle.min_y {
                dy = dy.min(obstacle.min_y - self.max_y);
            } else if dy < 0.0 && self.min_y >= obstacle.max_y {
                let gap = obstacle.max_y - self.min_y;
                dy = dy.max(if gap.abs() < 1.0e-4 { 0.0 } else { gap });
            }
        }
        dy
    }
    pub(crate) fn clip_x(self, obstacle: Self, mut dx: f64) -> f64 {
        if obstacle.max_y > self.min_y
            && obstacle.min_y < self.max_y
            && obstacle.max_z > self.min_z
            && obstacle.min_z < self.max_z
        {
            if dx > 0.0 && self.max_x <= obstacle.min_x {
                dx = dx.min(obstacle.min_x - self.max_x);
            } else if dx < 0.0 && self.min_x >= obstacle.max_x {
                dx = dx.max(obstacle.max_x - self.min_x);
            }
        }
        dx
    }
    pub(crate) fn clip_z(self, obstacle: Self, mut dz: f64) -> f64 {
        if obstacle.max_x > self.min_x
            && obstacle.min_x < self.max_x
            && obstacle.max_y > self.min_y
            && obstacle.min_y < self.max_y
        {
            if dz > 0.0 && self.max_z <= obstacle.min_z {
                dz = dz.min(obstacle.min_z - self.max_z);
            } else if dz < 0.0 && self.min_z >= obstacle.max_z {
                dz = dz.max(obstacle.max_z - self.min_z);
            }
        }
        dz
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// State and protocol data represented by `MotionState`.
pub struct MotionState {
    /// The `velocity` value.
    pub velocity: Vec3,
    /// The `fall_distance` value.
    pub fall_distance: f32,
    /// The `collided_horizontal` value.
    pub collided_horizontal: bool,
    /// The `collided_vertical` value.
    pub collided_vertical: bool,
    /// The `ticks` value.
    pub ticks: u64,
}

impl ControlState {
    /// Performs the `is_moving` operation.
    pub fn is_moving(self) -> bool {
        self.forward || self.back || self.left || self.right
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Possible values represented by `CorrectionReason`.
pub enum CorrectionReason {
    /// A server position arrived shortly after a movement packet and differed
    /// from the position proposed by the client.
    MovementRejectedOrAdjusted,
}

#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `PositionCorrection`.
pub struct PositionCorrection {
    /// The `expected` value.
    pub expected: Player,
    /// The `received` value.
    pub received: Player,
    /// The `distance` value.
    pub distance: f64,
    /// The `reason` value.
    pub reason: CorrectionReason,
}

#[derive(Clone, Debug, Default, PartialEq)]
/// State and protocol data represented by `PhysicsMetrics`.
pub struct PhysicsMetrics {
    /// The `movement_packets` value.
    pub movement_packets: u64,
    /// Authoritative position packets received after the initial spawn position.
    pub server_position_packets: u64,
    /// The `position_corrections` value.
    pub position_corrections: u64,
    /// The `total_correction_distance` value.
    pub total_correction_distance: f64,
    /// The `largest_correction_distance` value.
    pub largest_correction_distance: f64,
    /// The `disconnects` value.
    pub disconnects: u64,
    /// The `connected_for` value.
    pub connected_for: Duration,
    /// The `physics_ticks` value.
    pub physics_ticks: u64,
    /// The `physics_tick_p99` value.
    pub physics_tick_p99: Duration,
    /// The `physics_tick_max` value.
    pub physics_tick_max: Duration,
    /// The `physics_tick_lag_p99` value.
    pub physics_tick_lag_p99: Duration,
    /// The `physics_tick_lag_max` value.
    pub physics_tick_lag_max: Duration,
    /// Total number of tick duration samples represented by the bounded
    /// histogram (not merely the retained diagnostic window).
    pub physics_tick_samples: u64,
    /// True only if the bounded accumulator could not represent another
    /// sample. A gate must fail closed when this is set.
    pub physics_tick_histogram_overflowed: bool,
    /// Total number of scheduler-lag samples represented by the histogram.
    pub physics_tick_lag_samples: u64,
    /// True if the scheduler-lag histogram could not represent a sample.
    pub physics_tick_lag_histogram_overflowed: bool,
}

/// A boundary token for an instrumentation measurement window.
///
/// Establishing an epoch resets only the diagnostic accumulators returned by
/// [`crate::Bot::begin_measurement_epoch`]. It never rewinds physics state,
/// packet caches, or the last movement proposal used to classify a server
/// correction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicsMeasurementEpoch {
    generation: crate::ConnectionGeneration,
}

impl PhysicsMeasurementEpoch {
    pub(crate) const fn new(generation: crate::ConnectionGeneration) -> Self {
        Self { generation }
    }

    /// Returns the client connection generation owning this epoch.
    #[must_use]
    pub const fn generation(self) -> crate::ConnectionGeneration {
        self.generation
    }
}

pub(crate) struct PhysicsTracker {
    connected_at: Instant,
    last_sent: Option<(Player, Instant)>,
    metrics: PhysicsMetrics,
    tick_durations: DurationHistogram,
    tick_lag: DurationHistogram,
}

/// A fixed-size, mergeable duration summary. Bucket boundaries deliberately
/// include the approved R9 limits so a p99 below a limit is never rounded up
/// merely because it fell into a broad logarithmic bucket. `max` remains
/// exact, while p99 is the conservative upper bound of its bucket.
#[derive(Clone, Debug)]
struct DurationHistogram {
    counts: [u64; 24],
    count: u64,
    max: u64,
    overflowed: bool,
}

const HISTOGRAM_UPPER_BOUNDS: [u64; 24] = [
    0,
    1,
    2,
    4,
    8,
    16,
    32,
    64,
    128,
    256,
    512,
    1024,
    2048,
    4095,
    4999,
    5000,
    9999,
    19999,
    39999,
    49999,
    50000,
    65535,
    1_000_000,
    u64::MAX,
];

impl Default for DurationHistogram {
    fn default() -> Self {
        Self {
            counts: [0; 24],
            count: 0,
            max: 0,
            overflowed: false,
        }
    }
}

impl DurationHistogram {
    fn record(&mut self, value: Duration) {
        let micros = value.as_micros().min(u128::from(u64::MAX)) as u64;
        let Some(index) = HISTOGRAM_UPPER_BOUNDS
            .iter()
            .position(|upper| micros <= *upper)
        else {
            self.overflowed = true;
            return;
        };
        if self.count == u64::MAX || self.counts[index] == u64::MAX {
            self.overflowed = true;
            return;
        }
        self.count += 1;
        self.counts[index] += 1;
        self.max = self.max.max(micros);
    }

    fn p99(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let rank = self.count.saturating_mul(99).div_ceil(100).max(1);
        let mut cumulative = 0_u64;
        for (index, count) in self.counts.iter().copied().enumerate() {
            cumulative = cumulative.saturating_add(count);
            if cumulative >= rank {
                return HISTOGRAM_UPPER_BOUNDS[index];
            }
        }
        u64::MAX
    }
}

impl PhysicsTracker {
    pub fn new() -> Self {
        Self {
            connected_at: Instant::now(),
            last_sent: None,
            metrics: PhysicsMetrics::default(),
            tick_durations: DurationHistogram::default(),
            tick_lag: DurationHistogram::default(),
        }
    }

    pub fn record_movement(&mut self, player: Player) {
        self.metrics.movement_packets += 1;
        self.last_sent = Some((player, Instant::now()));
    }

    pub fn record_server_position(&mut self, received: &Player) -> Option<PositionCorrection> {
        self.metrics.server_position_packets += 1;
        let (expected, sent_at) = self.last_sent.as_ref()?;
        if sent_at.elapsed() > Duration::from_secs(2) {
            return None;
        }
        let dx = expected.x - received.x;
        let dy = expected.y - received.y;
        let dz = expected.z - received.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        if distance <= 1.0e-4 {
            return None;
        }
        self.metrics.position_corrections += 1;
        self.metrics.total_correction_distance += distance;
        self.metrics.largest_correction_distance =
            self.metrics.largest_correction_distance.max(distance);
        Some(PositionCorrection {
            expected: expected.clone(),
            received: received.clone(),
            distance,
            reason: CorrectionReason::MovementRejectedOrAdjusted,
        })
    }

    pub fn record_disconnect(&mut self) {
        self.metrics.disconnects += 1;
    }

    pub fn record_tick(&mut self, duration: Duration, lag: Duration) {
        self.metrics.physics_ticks += 1;
        self.tick_durations.record(duration);
        self.tick_lag.record(lag);
        self.metrics.physics_tick_max = Duration::from_micros(self.tick_durations.max);
        self.metrics.physics_tick_lag_max = Duration::from_micros(self.tick_lag.max);
    }

    pub fn snapshot(&self) -> PhysicsMetrics {
        let mut metrics = self.metrics.clone();
        metrics.connected_for = self.connected_at.elapsed();
        metrics.physics_tick_p99 = Duration::from_micros(self.tick_durations.p99());
        metrics.physics_tick_lag_p99 = Duration::from_micros(self.tick_lag.p99());
        metrics.physics_tick_samples = self.tick_durations.count;
        metrics.physics_tick_histogram_overflowed = self.tick_durations.overflowed;
        metrics.physics_tick_lag_samples = self.tick_lag.count;
        metrics.physics_tick_lag_histogram_overflowed = self.tick_lag.overflowed;
        metrics
    }

    /// Starts a new diagnostic measurement window without changing physics
    /// state. In particular, `last_sent` is retained so a server position
    /// packet received after the boundary is classified against the exact
    /// movement proposal that produced it.
    pub fn begin_measurement_epoch(&mut self) {
        self.connected_at = Instant::now();
        self.metrics = PhysicsMetrics::default();
        self.tick_durations = DurationHistogram::default();
        self.tick_lag = DurationHistogram::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct FixtureFile {
        generator: String,
        minecraft: String,
        fixtures: Vec<Fixture>,
    }

    #[derive(Deserialize)]
    struct Fixture {
        name: String,
        trajectory: Vec<FixtureTick>,
    }

    #[derive(Deserialize)]
    struct FixtureTick {
        position: [f64; 3],
        velocity: [f64; 3],
        on_ground: bool,
        fall_distance: f64,
        sprinting: bool,
    }

    #[test]
    fn correction_updates_metrics() {
        let mut tracker = PhysicsTracker::new();
        let mut expected = Player::offline("MetricBot");
        expected.x = 10.0;
        tracker.record_movement(expected);
        let mut received = Player::offline("MetricBot");
        received.x = 9.5;
        let correction = tracker.record_server_position(&received).unwrap();
        assert_eq!(correction.distance, 0.5);
        let metrics = tracker.snapshot();
        assert_eq!(metrics.movement_packets, 1);
        assert_eq!(metrics.server_position_packets, 1);
        assert_eq!(metrics.position_corrections, 1);
        assert_eq!(metrics.total_correction_distance, 0.5);
    }

    #[test]
    fn tick_metrics_report_p99_max_and_queue_lag() {
        let mut tracker = PhysicsTracker::new();
        for micros in 1..=100 {
            tracker.record_tick(
                Duration::from_micros(micros),
                Duration::from_micros(micros * 2),
            );
        }
        let metrics = tracker.snapshot();
        assert_eq!(metrics.physics_ticks, 100);
        assert_eq!(metrics.physics_tick_p99, Duration::from_micros(128));
        assert_eq!(metrics.physics_tick_max, Duration::from_micros(100));
        assert_eq!(metrics.physics_tick_lag_p99, Duration::from_micros(256));
        assert_eq!(metrics.physics_tick_lag_max, Duration::from_micros(200));
        assert_eq!(metrics.physics_tick_samples, 100);
        assert!(!metrics.physics_tick_histogram_overflowed);
    }

    #[test]
    fn tick_metric_histogram_is_bounded_and_covers_the_full_run() {
        let mut tracker = PhysicsTracker::new();
        for micros in 0..100_000 {
            tracker.record_tick(
                Duration::from_micros(micros as u64),
                Duration::from_micros(micros as u64),
            );
        }
        let metrics = tracker.snapshot();
        assert_eq!(metrics.physics_ticks, 100_000);
        assert_eq!(metrics.physics_tick_samples, 100_000);
        assert_eq!(metrics.physics_tick_lag_samples, 100_000);
        assert!(!metrics.physics_tick_histogram_overflowed);
        assert!(!metrics.physics_tick_lag_histogram_overflowed);
    }

    #[test]
    fn flat_trajectories_match_prismarine_physics_fixtures() {
        let file: FixtureFile =
            serde_json::from_str(include_str!("../data/prismarine_physics_fixtures.json")).unwrap();
        assert_eq!(file.generator, "prismarine-physics@1.11.1");
        assert_eq!(file.minecraft, "1.16.1");
        for fixture in file.fixtures {
            if !matches!(
                fixture.name.as_str(),
                "idle" | "walk" | "jump" | "sprint" | "fall"
            ) {
                continue;
            }
            let mut position = Vec3 {
                x: 0.5,
                y: if fixture.name == "fall" { 8.0 } else { 1.0 },
                z: 0.5,
            };
            let mut velocity = Vec3::default();
            let mut on_ground = fixture.name != "fall";
            let mut fall_distance = 0.0;
            for (index, expected) in fixture.trajectory.iter().enumerate() {
                let was_on_ground = on_ground;
                let forward = matches!(fixture.name.as_str(), "walk" | "sprint");
                let sprint = fixture.name == "sprint";
                let jump = fixture.name == "jump" && index == 0;
                let friction: f64 = 0.91 * 0.6;
                let speed = if sprint { 0.13 } else { 0.1 };
                let acceleration = if was_on_ground {
                    speed * (0.162_771_36 / friction.powi(3))
                } else if sprint {
                    0.026
                } else {
                    0.02
                };
                if forward {
                    velocity.z += acceleration * 0.98;
                }
                if jump && was_on_ground {
                    velocity.y = f64::from(0.42_f32);
                }
                position.x += velocity.x;
                let moved_y = velocity.y;
                position.y += moved_y;
                position.z += velocity.z;
                let collided = position.y < 1.0;
                if collided {
                    position.y = 1.0;
                    velocity.y = 0.0;
                }
                on_ground = collided;
                if on_ground {
                    fall_distance = 0.0;
                } else if moved_y < 0.0 {
                    fall_distance += -moved_y;
                }
                velocity.y = (velocity.y - 0.08) * 0.98;
                let drag = if was_on_ground { friction } else { 0.91 };
                velocity.x *= drag;
                velocity.z *= drag;
                let epsilon = 2.0e-5;
                for (actual, reference) in [position.x, position.y, position.z]
                    .into_iter()
                    .zip(expected.position)
                {
                    assert!(
                        (actual - reference).abs() < epsilon,
                        "{} tick {} position: {actual} != {reference}",
                        fixture.name,
                        index + 1
                    );
                }
                for (actual, reference) in [velocity.x, velocity.y, velocity.z]
                    .into_iter()
                    .zip(expected.velocity)
                {
                    assert!(
                        (actual - reference).abs() < epsilon,
                        "{} tick {} velocity: {actual} != {reference}",
                        fixture.name,
                        index + 1
                    );
                }
                assert_eq!(
                    on_ground,
                    expected.on_ground,
                    "{} tick {}",
                    fixture.name,
                    index + 1
                );
                assert!((fall_distance - expected.fall_distance).abs() < epsilon);
                assert_eq!(sprint, expected.sprinting);
            }
        }
    }

    #[test]
    fn matching_position_is_not_a_correction() {
        let mut tracker = PhysicsTracker::new();
        let player = Player::offline("MetricBot");
        tracker.record_movement(player.clone());
        assert!(tracker.record_server_position(&player).is_none());
        assert_eq!(tracker.snapshot().position_corrections, 0);
    }

    #[test]
    fn measurement_epoch_resets_diagnostics_without_replacing_movement_proposal() {
        let mut tracker = PhysicsTracker::new();
        let mut expected = Player::offline("MetricBot");
        expected.x = 10.0;
        tracker.record_movement(expected);
        tracker.record_tick(Duration::from_micros(3), Duration::from_micros(4));
        tracker.begin_measurement_epoch();

        let mut received = Player::offline("MetricBot");
        received.x = 9.5;
        let correction = tracker
            .record_server_position(&received)
            .expect("epoch must retain the movement proposal");
        assert_eq!(correction.distance, 0.5);
        let metrics = tracker.snapshot();
        assert_eq!(metrics.movement_packets, 0);
        assert_eq!(metrics.position_corrections, 1);
        assert_eq!(metrics.physics_ticks, 0);
    }

    #[test]
    fn aabb_clips_motion_against_full_block() {
        let player = Aabb::player(0.5, 1.0, 0.5);
        let floor = Aabb::block(0, 0, 0);
        assert_eq!(player.clip_y(floor, -0.2), 0.0);
        let wall = Aabb::block(1, 1, 0);
        assert!((player.clip_x(wall, 1.0) - 0.2).abs() < 1.0e-12);
    }
}
