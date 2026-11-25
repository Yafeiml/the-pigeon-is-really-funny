use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Rest,
    Walk,
    Look,
    Peck,
}

#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub head_x: f32,
    pub head_y: f32,
    pub body_y: f32,
    pub body_pitch: f32,
    pub near_foot_x: f32,
    pub near_foot_y: f32,
    pub far_foot_x: f32,
    pub far_foot_y: f32,
    pub look: f32,
    pub peck: f32,
    pub facing: f32,
}

impl Pose {
    fn standing(facing: f32) -> Self {
        let (near_foot_x, far_foot_x) = if cfg!(feature = "painted") {
            (
                foot_position(STANDING_PHASE).0,
                foot_position(STANDING_PHASE + 0.5).0,
            )
        } else {
            (7.0, -9.0)
        };
        Self {
            head_x: 48.0,
            head_y: -61.0,
            body_y: 0.0,
            body_pitch: 0.0,
            near_foot_x,
            near_foot_y: 72.0,
            far_foot_x,
            far_foot_y: 72.0,
            look: 0.0,
            peck: 0.0,
            facing,
        }
    }

    fn blend(self, to: Self, amount: f32) -> Self {
        let mix = |from, to| from + (to - from) * amount;
        Self {
            head_x: mix(self.head_x, to.head_x),
            head_y: mix(self.head_y, to.head_y),
            body_y: mix(self.body_y, to.body_y),
            body_pitch: mix(self.body_pitch, to.body_pitch),
            near_foot_x: mix(self.near_foot_x, to.near_foot_x),
            near_foot_y: mix(self.near_foot_y, to.near_foot_y),
            far_foot_x: mix(self.far_foot_x, to.far_foot_x),
            far_foot_y: mix(self.far_foot_y, to.far_foot_y),
            look: mix(self.look, to.look),
            peck: mix(self.peck, to.peck),
            facing: to.facing,
        }
    }
}

// Painted legs have a shorter reach than the archived 3D gait.
const STRIDE: f32 = if cfg!(feature = "painted") {
    48.0
} else {
    60.0
};
const WALK_SPEED: f32 = 70.0;
const PECK_SECONDS: f32 = 2.8;

pub fn sprite_frame_count(clip: &str, painted: bool) -> u32 {
    match (clip, painted) {
        ("walk" | "stalk", true) => 256,
        ("look", true) => 1020,
        ("peck", true) => 840,
        ("settle", true) => 56,
        ("turn", true) => 80,
        ("walk" | "stalk", false) => 128,
        ("look" | "peck", false) => 96,
        ("settle", false) => 8,
        ("turn", false) => 16,
        _ => unreachable!("unknown pigeon animation"),
    }
}

fn peck_sprite_phase(time: f32) -> f32 {
    time.rem_euclid(PECK_SECONDS) / PECK_SECONDS
}
const HOLD: f32 = if cfg!(feature = "painted") {
    0.68
} else {
    0.62
};
// Both feet touch down and the head is near its resting position at this stride phase.
const STANDING_PHASE: f32 = 0.125;
const AUTO_ACTIONS: [(Mode, f32); 5] = [
    (Mode::Walk, 6.2),
    (Mode::Look, 3.5),
    (Mode::Walk, 4.6),
    (Mode::Peck, 3.1),
    (Mode::Look, 2.0),
];

// Decisions are made at footfalls or after an idle action, so display refresh never drives the RNG.
struct Roam {
    next: Mode,
    planned: bool,
    remaining: f32,
    length: f32,
    next_pace: f32,
    pace: f32,
    target_pace: f32,
    start_ground: f32,
    end_ground: f32,
    gait: Gait,
    turn: Option<f32>,
}

#[derive(Clone, Copy, PartialEq)]
enum Gait {
    Walk,
    Lower(f32),
    Stalk,
    Rise(f32),
}

#[derive(Clone, Copy)]
struct FoodPatch {
    x: f32,
    ground: f32,
    checks_left: u8,
    pecks: u8,
}

// At contact the painted bill is 458/2 - 136 logical pixels ahead of the sprite pivot.
const BILL_REACH: f32 = 93.0;
const PATCH_RADIUS: f32 = 58.0;

pub struct Motion {
    mode: Mode,
    action: Mode,
    speed: f32,
    current_speed: f32,
    phase: f32,
    walk_weight: f32,
    elapsed: f32,
    action_time: f32,
    auto_index: usize,
    auto_elapsed: f32,
    auto_roll: u32,
    x: f32,
    width: f32,
    wrap_margin: f32,
    ground: f32,
    max_ground: f32,
    roam: Roam,
    patch: Option<FoodPatch>,
    search_after: f32,
    low_retap: bool,
    initialized: bool,
    facing: f32,
    transition_from: Pose,
    transition_time: f32,
}

impl Motion {
    pub fn new() -> Self {
        let seed = if cfg!(feature = "painted") {
            use std::hash::{BuildHasher, RandomState};
            RandomState::new().hash_one(std::process::id()) as u32
        } else {
            0x8000_0000
        };
        Self::with_seed(seed)
    }

    pub fn with_seed(seed: u32) -> Self {
        Self {
            mode: Mode::Auto,
            action: Mode::Walk,
            speed: 1.0,
            current_speed: 1.0,
            phase: STANDING_PHASE,
            walk_weight: 0.0,
            elapsed: 0.0,
            action_time: 0.0,
            auto_index: 0,
            auto_elapsed: 0.0,
            auto_roll: seed,
            x: 0.0,
            width: 0.0,
            wrap_margin: 0.0,
            ground: 0.0,
            max_ground: 0.0,
            patch: None,
            search_after: 0.0,
            low_retap: false,
            roam: Roam {
                next: Mode::Walk,
                planned: false,
                remaining: 0.0,
                length: 0.0,
                next_pace: 0.0,
                pace: 1.0,
                target_pace: 1.0,
                start_ground: 0.0,
                end_ground: 0.0,
                gait: Gait::Walk,
                turn: None,
            },
            initialized: false,
            facing: 1.0,
            transition_from: Pose::standing(1.0),
            transition_time: 0.0,
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        if self.low_retap {
            self.action_time = self.peck_time();
            self.low_retap = false;
        }
        self.mode = mode;
        self.patch = None;
        self.auto_index = 0;
        self.auto_elapsed = 0.0;
        // Changing a menu choice midway through a turn must keep its angle and planted feet.
        self.roam.next = if mode == Mode::Rest || matches!(self.roam.gait, Gait::Rise(_)) {
            Mode::Look
        } else {
            Mode::Walk
        };
        self.roam.planned = self.roam.turn.is_some() || self.roam.gait != Gait::Walk;
        let requested = match mode {
            Mode::Auto => Mode::Walk,
            Mode::Rest => Mode::Look,
            manual => manual,
        };
        if requested != self.action {
            if self.can_leave_action() {
                self.begin_action(requested);
            } else if self.action == Mode::Look {
                self.action_time = look_return_time(self.action_time);
            }
        } else if mode == Mode::Rest {
            self.plan_idle();
        }
    }

    fn begin_action(&mut self, action: Mode) {
        self.transition_from = self.pose();
        self.action = action;
        self.action_time = 0.0;
        self.transition_time = 0.0;
        self.walk_weight = 0.0;
        self.low_retap = false;
        if action == Mode::Walk {
            self.phase = STANDING_PHASE;
            if self.mode == Mode::Auto && cfg!(feature = "painted") {
                self.plan_walk(true, true);
            } else if self.mode == Mode::Auto && self.approaching_edge() {
                self.facing = -self.facing;
            }
        } else if self.mode == Mode::Rest || (self.mode == Mode::Auto && cfg!(feature = "painted"))
        {
            self.plan_idle();
        }
    }

    fn plan_idle(&mut self) {
        let studying = self.at_patch();
        if self.action == Mode::Peck && studying {
            self.low_retap = self.patch.is_some_and(|p| p.pecks > 0) || self.draw() < 0.32;
        }
        self.roam.remaining = if self.low_retap {
            1.18 + 0.16 * self.draw()
        } else if self.action == Mode::Look && studying {
            0.68 + 0.9 * self.draw()
        } else if self.action == Mode::Look {
            0.38 + 2.4 * self.draw().powi(2)
        } else if self.draw() < 0.62 {
            0.87 + 0.41 * self.draw()
        } else {
            1.88 + 0.57 * self.draw()
        };
        self.roam.planned = true;
    }

    fn draw(&mut self) -> f32 {
        self.auto_roll = self
            .auto_roll
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        (self.auto_roll >> 8) as f32 / 16_777_216.0
    }

    fn at_patch(&self) -> bool {
        self.patch.is_some_and(|p| {
            (p.x - self.x - self.facing * BILL_REACH).hypot(p.ground - self.ground) <= PATCH_RADIUS
        })
    }

    fn leave_patch(&mut self) {
        self.patch = None;
        self.search_after = STRIDE * (2.4 + 3.0 * self.draw());
    }

    fn plan_walk(&mut self, can_turn: bool, animate_turn: bool) {
        let can_turn = can_turn && !self.crossing_edge();
        let step = STRIDE * 0.5;
        let mut direction = self.facing;
        if can_turn && self.width >= step {
            let heading = if self.draw() < 0.34 {
                -self.facing
            } else {
                self.facing
            };
            let available = if heading > 0.0 {
                self.width - self.x
            } else {
                self.x
            };
            direction = if available < STRIDE && self.width - available >= STRIDE {
                -heading
            } else {
                heading
            };
        }
        if can_turn
            && self.patch.is_none()
            && self.search_after <= 0.0
            && self.width >= 4.0 * step
            && self.draw() < 0.57
        {
            let ahead = if direction > 0.0 {
                self.width - self.x
            } else {
                self.x
            };
            let approach =
                (step * (1.2 + 6.0 * self.draw())).min((ahead - self.wrap_margin).max(0.0));
            let grade = (2.0 * self.draw() - 1.0) * 0.16;
            self.patch = Some(FoodPatch {
                x: self.x + direction * (approach + BILL_REACH),
                ground: (self.ground + approach * grade).clamp(0.0, self.max_ground),
                checks_left: 2 + (self.draw() * 3.0) as u8,
                pecks: 0,
            });
        }
        if let Some(patch) = self.patch {
            let to_patch = patch.x - self.x;
            let room = if to_patch >= 0.0 {
                self.width - self.x
            } else {
                self.x
            };
            // A side-on bird cannot take another step through the patch and still reach it with its bill.
            if to_patch.abs() + PATCH_RADIUS < BILL_REACH + step || room < step * 1.3 {
                self.leave_patch();
            } else if can_turn {
                direction = to_patch.signum();
            }
        }
        let turning = animate_turn && direction != self.facing;
        if !turning {
            self.facing = direction;
        }
        let available = if direction > 0.0 {
            self.width - self.x
        } else {
            self.x
        };
        // Short trips around one patch are common; only some trips cross a substantial distance.
        let approach = self.patch.map(|p| (p.x - self.x) * direction - BILL_REACH);
        let stalk = can_turn
            && available >= STRIDE + 2.0
            && approach.map_or_else(|| self.draw() < 0.34, |distance| distance < STRIDE * 2.0);
        // End where the same near and far toes meet the idle painting.
        let units = if stalk { 4.0 } else { 7.5 };
        let since_landing = (self.phase - STANDING_PHASE).rem_euclid(1.0) * STRIDE;
        let planned = approach.map_or_else(
            || 1.0 + (units * self.draw().powi(2)).floor(),
            |distance| ((distance + since_landing) / STRIDE).round().max(1.0),
        );
        let cycles = if self.wrap_margin > 0.0 {
            planned
        } else {
            planned.min(((available + since_landing) / STRIDE).floor())
        };
        let length = (cycles * STRIDE - since_landing).max(0.0);
        let grade = self.draw() * 2.0 - 1.0;
        let grade = if !stalk && grade.abs() > 0.75 {
            grade.signum() * (0.20 + (grade.abs() - 0.75) * 0.36)
        } else {
            grade * 0.115
        };
        let target_pace = if stalk {
            0.50 + 0.33 * self.draw()
        } else {
            0.78 + 0.90 * self.draw()
        };
        self.roam = Roam {
            next: Mode::Walk,
            planned: true,
            remaining: length,
            length,
            next_pace: step * (2.0 + (4.0 * self.draw()).floor()),
            pace: 0.65,
            target_pace,
            start_ground: self.ground,
            end_ground: self
                .patch
                .map_or(self.ground + length * grade, |p| p.ground)
                .clamp(0.0, self.max_ground),
            gait: if stalk { Gait::Lower(0.0) } else { Gait::Walk },
            turn: turning.then_some(0.0),
        };
    }

    fn roam_request(&mut self, dt: f32) -> Mode {
        if self.roam.next != self.action {
            return self.roam.next;
        }
        if self.action == Mode::Walk {
            if !self.roam.planned {
                self.plan_walk(self.walk_weight == 0.0, false);
            }
            if self.roam.turn.is_some() || matches!(self.roam.gait, Gait::Lower(_)) {
                return Mode::Walk;
            }
            let traveled = self.roam.length - self.roam.remaining;
            if self.width < STRIDE
                || (self.roam.remaining <= STRIDE * 0.02 && !self.crossing_edge())
                || (self.wrap_margin == 0.0 && self.approaching_edge())
            {
                let pick = self.draw();
                self.roam.next =
                    if (self.at_patch() && pick < 0.76) || (self.patch.is_none() && pick < 0.22) {
                        Mode::Peck
                    } else {
                        Mode::Look
                    };
            } else if traveled >= self.roam.next_pace {
                self.roam.target_pace = if self.roam.gait == Gait::Stalk {
                    0.51 + 0.33 * self.draw()
                } else {
                    0.63 + 1.03 * self.draw()
                };
                self.roam.next_pace += STRIDE * (1.0 + 1.8 * self.draw());
            }
            let target = if self.roam.remaining < STRIDE * 0.27 {
                self.roam.target_pace.min(0.74)
            } else {
                self.roam.target_pace
            };
            self.roam.pace += (target - self.roam.pace) * (1.0 - (-4.0 * dt).exp());
        } else {
            self.roam.remaining -= dt * self.current_speed;
            if self.roam.remaining <= 0.0 {
                let choice = self.draw();
                self.roam.next = if self.at_patch() {
                    let patch = self.patch.as_mut().unwrap();
                    patch.checks_left -= 1;
                    patch.pecks += u8::from(self.action == Mode::Peck);
                    if patch.checks_left == 0 || (self.action == Mode::Look && choice < 0.15) {
                        self.leave_patch();
                        Mode::Walk
                    } else if self.action == Mode::Look && choice < 0.76 {
                        Mode::Peck
                    } else if self.action == Mode::Peck && choice < 0.45 {
                        Mode::Look
                    } else {
                        Mode::Walk
                    }
                } else if self.patch.is_some() {
                    Mode::Walk
                } else {
                    let stay = self.mode == Mode::Rest || self.width < STRIDE;
                    match self.action {
                        Mode::Look if stay => Mode::Peck,
                        Mode::Peck if stay => Mode::Look,
                        Mode::Look if choice < 0.30 => Mode::Peck,
                        Mode::Peck if choice < 0.29 => Mode::Look,
                        _ => Mode::Walk,
                    }
                };
                if self.action == Mode::Look {
                    self.action_time = look_return_time(self.action_time);
                }
            }
        }
        self.roam.next
    }

    fn approaching_edge(&self) -> bool {
        let ahead = if self.facing > 0.0 {
            self.width - self.x
        } else {
            self.x
        };
        self.width > 0.0 && ahead <= (STRIDE * 0.55).min(self.width * 0.3)
    }

    fn crossing_edge(&self) -> bool {
        let margin = self.wrap_margin;
        margin > 0.0
            && (self.x < margin
                || self.x > self.width - margin
                || if self.facing > 0.0 {
                    self.width - self.x <= margin + STRIDE
                } else {
                    self.x <= margin + STRIDE
                })
    }

    fn can_leave_action(&self) -> bool {
        if self.action == Mode::Walk && (self.roam.gait != Gait::Walk || self.roam.turn.is_some()) {
            return false;
        }
        match self.action {
            Mode::Walk | Mode::Auto if cfg!(feature = "painted") => self.phase == STANDING_PHASE,
            Mode::Walk | Mode::Auto => (self.phase % 0.5 - STANDING_PHASE).abs() < 0.012,
            Mode::Rest => unreachable!("rest selects a look or a peck"),
            Mode::Look => look_direction(self.action_time).abs() < 0.05,
            Mode::Peck => peck_depth(self.peck_time()) < 0.015,
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() {
            self.speed = speed.clamp(0.1, 3.0);
        }
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn sprite_sample(&self) -> (Mode, f32) {
        let phase = match self.action {
            Mode::Walk | Mode::Auto => self.phase,
            Mode::Rest => unreachable!("rest selects a look or a peck"),
            Mode::Look => (self.action_time / 3.4).fract(),
            Mode::Peck => peck_sprite_phase(self.peck_time()),
        };
        (self.action, phase)
    }

    fn peck_time(&self) -> f32 {
        let t = self.action_time;
        if !self.low_retap || t <= 0.70 {
            t
        } else if t <= 0.76 {
            0.70
        } else if t <= 0.88 {
            1.46 - t
        } else {
            t - 0.30
        }
    }

    pub fn sprite_clip(&self) -> (&'static str, f32) {
        if let Some(t) = self.roam.turn {
            // The same front key holds across the mirror switch, even at a lower display rate.
            return ("turn", (t.min(1.0 - t) * 2.22).min(0.99999));
        }
        let (action, phase) = self.sprite_sample();
        match action {
            Mode::Walk | Mode::Auto => match self.roam.gait {
                Gait::Walk => ("walk", phase),
                Gait::Stalk => ("stalk", phase),
                Gait::Lower(amount) | Gait::Rise(amount) => ("settle", amount.min(0.99999)),
            },
            Mode::Look => ("look", phase),
            Mode::Peck => ("peck", phase),
            Mode::Rest => unreachable!("rest selects a look or a peck"),
        }
    }

    fn stationary_transition(&mut self, requested: Mode, dt: f32) -> bool {
        if let Some(t) = self.roam.turn {
            let duration = if self.mode == Mode::Auto { 0.58 } else { 0.42 };
            let next = t + dt * self.current_speed / duration;
            if t < 0.5 && next >= 0.5 {
                self.facing = -self.facing;
            }
            self.roam.turn = (next < 1.0).then_some(next);
            return true;
        }
        if self.action != Mode::Walk || self.roam.gait == Gait::Walk {
            return false;
        }
        let leaving = requested != Mode::Walk || self.mode != Mode::Auto;
        let delta = dt * self.current_speed / 0.18;
        self.roam.gait = match self.roam.gait {
            Gait::Lower(amount) if leaving => Gait::Rise(amount),
            Gait::Lower(amount) if amount + delta >= 1.0 => Gait::Stalk,
            Gait::Lower(amount) => Gait::Lower(amount + delta),
            Gait::Stalk if leaving && self.phase == STANDING_PHASE => Gait::Rise(1.0),
            Gait::Rise(amount) if amount <= delta => Gait::Walk,
            Gait::Rise(amount) => Gait::Rise(amount - delta),
            gait => gait,
        };
        matches!(self.roam.gait, Gait::Lower(_) | Gait::Rise(_))
    }

    pub fn x(&self) -> f32 {
        self.x
    }

    pub fn ground(&self) -> f32 {
        self.ground
    }

    pub fn set_ground(&mut self, y: f32, maximum: f32) {
        if y.is_finite() && maximum.is_finite() {
            let maximum = maximum.max(0.0);
            let y = y.clamp(0.0, maximum);
            if y != self.ground || maximum != self.max_ground {
                self.patch = None;
                self.ground = y;
                self.max_ground = maximum;
                self.roam.start_ground = y;
                self.roam.end_ground = y;
            }
        }
    }

    pub fn facing(&self) -> f32 {
        self.facing
    }

    pub fn set_x(&mut self, x: f32) {
        if x.is_finite() {
            if (self.x - x).abs() > 0.001 {
                self.patch = None;
            }
            self.x = x.max(0.0);
            self.initialized = true;
        }
    }

    #[cfg(any(windows, test))]
    pub fn wrap_at_edges(&mut self, margin: f32) {
        self.wrap_margin = margin;
    }

    pub fn update(&mut self, dt: f32, bounds_width: f32) {
        if bounds_width < self.width {
            self.patch = None;
        }
        self.width = if bounds_width.is_finite() {
            bounds_width.max(0.0)
        } else {
            0.0
        };
        if !self.initialized {
            self.x = self.width * 0.5;
            self.initialized = true;
        }
        self.x = self.x.clamp(0.0, self.width);
        if cfg!(feature = "painted")
            && self.mode == Mode::Auto
            && self.action == Mode::Walk
            && self.roam.next == Mode::Walk
            && !self.roam.planned
            && self.width > 0.0
        {
            self.plan_walk(self.walk_weight == 0.0, false);
        }
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }

        // Resume without a jump after a suspended process or a long UI operation.
        let dt = dt.min(0.1);
        let steps = (dt * 240.0).ceil().max(1.0) as u32;
        let step = dt / steps as f32;
        for _ in 0..steps {
            self.advance(step);
        }
    }

    fn advance(&mut self, dt: f32) {
        self.elapsed = (self.elapsed + dt) % 3600.0;
        self.current_speed += (self.speed - self.current_speed) * (1.0 - (-8.0 * dt).exp());
        self.transition_time = (self.transition_time + dt).min(0.28);
        let painted_auto = cfg!(feature = "painted") && self.mode == Mode::Auto;
        let (requested, manual_exit) = if painted_auto || self.mode == Mode::Rest {
            (self.roam_request(dt), false)
        } else if self.mode == Mode::Auto {
            self.legacy_request(dt)
        } else {
            (self.mode, self.mode != self.action)
        };
        if self.stationary_transition(requested, dt) {
            return;
        }
        if requested != self.action && self.can_leave_action() {
            self.begin_action(requested);
            if self.roam.turn.is_some() || matches!(self.roam.gait, Gait::Lower(_)) {
                return;
            }
        }
        // Complete the current stride or peck promptly after a menu choice at half speed.
        let clip_step = dt
            * if manual_exit {
                let floor = if cfg!(feature = "painted") && self.action == Mode::Walk {
                    1.15
                } else {
                    1.0
                };
                self.current_speed.max(floor)
            } else {
                self.current_speed
            };
        self.action_time = match self.action {
            Mode::Look => (self.action_time + clip_step) % 3.4,
            Mode::Peck => (self.action_time + clip_step) % PECK_SECONDS,
            Mode::Walk | Mode::Auto => 0.0,
            Mode::Rest => unreachable!("rest selects a look or a peck"),
        };
        let target = if self.action == Mode::Walk { 1.0 } else { 0.0 };
        self.walk_weight += (target - self.walk_weight) * (1.0 - (-10.0 * dt).exp());
        let roaming = painted_auto && self.action == Mode::Walk;
        let pace = if roaming { self.roam.pace } else { 1.0 };
        let mut distance = WALK_SPEED * self.walk_weight * clip_step * pace;
        let to_landing = (STANDING_PHASE - self.phase).rem_euclid(1.0) * STRIDE;
        let arriving =
            roaming && self.roam.remaining <= distance + STRIDE * 0.02 && !self.crossing_edge();
        let stop_here = cfg!(feature = "painted")
            && self.action == Mode::Walk
            && (requested != Mode::Walk || arriving)
            && distance + 0.0001 >= to_landing;
        if stop_here {
            distance = to_landing;
        }
        if roaming && self.roam.length > 0.0 {
            self.roam.remaining -= distance;
            self.search_after = (self.search_after - distance).max(0.0);
            let progress = smooth(1.0 - self.roam.remaining / self.roam.length);
            self.ground = (self.roam.start_ground
                + (self.roam.end_ground - self.roam.start_ground) * progress)
                .clamp(0.0, self.max_ground);
        }
        if arriving && stop_here {
            self.roam.remaining = 0.0;
        }
        self.phase = if stop_here {
            STANDING_PHASE
        } else {
            (self.phase + distance / STRIDE).fract()
        };
        if self.width > 0.0 {
            let next = self.x + distance * self.facing;
            if self.wrap_margin > 0.0 {
                if next < 0.0 || next >= self.width {
                    self.x = next.rem_euclid(self.width);
                    if self.patch.is_some() {
                        self.leave_patch();
                    }
                } else {
                    self.x = next;
                }
                return;
            }
            self.x = next.clamp(0.0, self.width);
            // Auto birds settle before turning; continuous manual walking reflects at the boundary.
            if !matches!(self.mode, Mode::Auto | Mode::Rest) {
                if self.x >= self.width {
                    self.facing = -1.0;
                } else if self.x <= 0.0 {
                    self.facing = 1.0;
                }
            }
        }
    }

    fn legacy_request(&mut self, dt: f32) -> (Mode, bool) {
        let (automatic_action, typical_duration) = AUTO_ACTIONS[self.auto_index];
        let duration = typical_duration * (0.72 + (self.auto_roll >> 16) as f32 / 65535.0 * 0.56);
        self.auto_elapsed += dt;
        let next_index = if self.action == automatic_action
            && (self.auto_elapsed >= duration
                || (self.action == Mode::Walk && self.approaching_edge()))
        {
            (self.auto_index + 1) % AUTO_ACTIONS.len()
        } else {
            self.auto_index
        };
        let requested = AUTO_ACTIONS[next_index].0;
        if requested != self.action && self.can_leave_action() {
            if next_index != self.auto_index {
                // One deterministic draw per automatic activity keeps timing independent of frame rate.
                self.auto_roll = self
                    .auto_roll
                    .wrapping_mul(1664525)
                    .wrapping_add(1013904223);
            }
            self.auto_index = next_index;
            self.auto_elapsed = 0.0;
        }
        (
            requested,
            requested != self.action && self.action != automatic_action && !self.can_leave_action(),
        )
    }

    pub fn pose(&self) -> Pose {
        let mut idle = Pose::standing(self.facing);
        idle.body_y = (self.elapsed * 2.8).sin() * 0.55;
        match self.action {
            Mode::Look => {
                idle.look = look_direction(self.action_time);
                idle.head_x += idle.look * 9.0;
                idle.head_y -= idle.look.abs() * 2.5;
            }
            Mode::Peck => {
                let time = self.peck_time().rem_euclid(PECK_SECONDS);
                let peck = peck_depth(time);
                idle.peck = peck;
                idle.head_x += 30.0 * peck;
                idle.head_y += 108.0 * peck;
                idle.body_y += 3.0 * peck;
                idle.body_pitch = 0.13 * peck;
                idle.look = (1.0 - peck) * 0.15 * (time * 4.0).sin();
            }
            Mode::Auto | Mode::Walk => {}
            Mode::Rest => unreachable!("rest selects a look or a peck"),
        }
        let walking = walk_pose(self.phase, self.facing);
        let pose = idle.blend(walking, self.walk_weight);
        self.transition_from
            .blend(pose, smooth(self.transition_time / 0.28))
    }
}

fn smooth(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn walk_pose(phase: f32, facing: f32) -> Pose {
    let mut pose = Pose::standing(facing);
    let head_phase = (phase * 2.0).fract();
    // Subtract the distance travelled during hold: the head stays still in world coordinates.
    pose.head_x = 58.0 - STRIDE * 0.5 * head_phase
        + STRIDE * 0.5 * smooth((head_phase - HOLD) / (1.0 - HOLD));
    pose.head_y = -61.0;
    if cfg!(feature = "painted") {
        // A small dip accompanies the fast reach; the eye stays level throughout the hold.
        let reach = ((head_phase - HOLD) / (1.0 - HOLD)).clamp(0.0, 1.0);
        pose.head_y += 1.4 * (PI * reach).sin().powi(2);
    }
    pose.body_y = (phase * TAU * 2.0).cos() * 1.6;
    pose.body_pitch = (phase * TAU).sin() * 0.018;
    (pose.near_foot_x, pose.near_foot_y) = foot_position(phase);
    (pose.far_foot_x, pose.far_foot_y) = foot_position((phase + 0.5).fract());
    pose
}

fn foot_position(phase: f32) -> (f32, f32) {
    const STANCE: f32 = 0.64;
    const FRONT: f32 = STRIDE * 0.25;
    if phase < STANCE {
        (FRONT - STRIDE * phase, 72.0)
    } else {
        let swing = (phase - STANCE) / (1.0 - STANCE);
        let mut x = FRONT - STRIDE * STANCE + STRIDE * STANCE * smooth(swing);
        let mut lift = (swing * PI).sin();
        if cfg!(feature = "painted") {
            // Match the planted toe's backward speed in body space as it leaves and meets the ground.
            let edge = swing.min(1.0 - swing);
            let contact = (1.0 - edge / 0.16).max(0.0).powi(2);
            x += STRIDE * (1.0 - STANCE) * edge.copysign(swing - 0.5) * contact;
            // Round the ends of the lift arc while keeping the original height at mid-swing.
            lift = lift * lift / (0.12 + 0.88 * lift);
        }
        (x, 72.0 - 15.0 * lift)
    }
}

fn peck_pulse(time: f32, start: f32) -> f32 {
    let time = time - start;
    if time < 0.0 {
        0.0
    } else if time < 0.18 {
        smooth(time / 0.18)
    } else if time < 0.25 {
        1.0
    } else {
        1.0 - smooth((time - 0.25) / 0.38)
    }
}

fn peck_depth(time: f32) -> f32 {
    if !cfg!(feature = "painted") {
        return peck_pulse(time, 0.35).max(peck_pulse(time, 1.35) * 0.92);
    }
    // Briefly sight the grain, tap it, then lift. The second tap comes back faster.
    fn tap(t: f32) -> f32 {
        if t < 0.15 {
            0.14 * smooth(t / 0.075)
        } else if t < 0.31 {
            0.14 + 0.86 * smooth((t - 0.15) / 0.12)
        } else {
            1.0 - smooth((t - 0.31) / 0.27)
        }
    }
    tap(time - 0.27).max(tap((time - 1.36) * 1.16))
}

fn look_direction(time: f32) -> f32 {
    let t = time.rem_euclid(3.4);
    if t < 0.35 {
        0.85 * smooth(t / 0.35)
    } else if t < 0.9 {
        0.85
    } else if t < 1.2 {
        0.85 * (1.0 - smooth((t - 0.9) / 0.3))
    } else if t < 1.45 {
        0.0
    } else if t < 1.8 {
        -0.95 * smooth((t - 1.45) / 0.35)
    } else if t < 2.35 {
        -0.95
    } else {
        -0.95 * (1.0 - smooth((t - 2.35) / 0.35))
    }
}

fn look_return_time(time: f32) -> f32 {
    // Both turns use symmetric easing. Enter the return at the angle already on screen.
    if time < 0.9 {
        0.9 + 0.3 * (1.0 - (time / 0.35).min(1.0))
    } else if (1.45..2.35).contains(&time) {
        2.35 + 0.35 * (1.0 - ((time - 1.45) / 0.35).min(1.0))
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_wraps_in_both_directions_without_resetting_the_stride() {
        let width = 900.0;
        for facing in [-1.0, 1.0] {
            let mut motion = Motion::with_seed(0);
            motion.wrap_at_edges(107.0);
            motion.set_mode(Mode::Walk);
            motion.facing = facing;
            motion.walk_weight = 1.0;
            motion.set_ground(193.5, 600.0);
            let x = if facing < 0.0 { 0.125 } else { width - 0.125 };
            motion.set_x(x);
            let phase = motion.phase;
            motion.update(1.0 / 240.0, width);
            let distance = WALK_SPEED / 240.0;
            assert!((motion.x() - (x + facing * distance).rem_euclid(width)).abs() < 0.001);
            assert_eq!(motion.facing(), facing);
            assert_eq!(motion.ground(), 193.5);
            assert!((motion.phase - (phase + distance / STRIDE).fract()).abs() < 0.001);
        }
    }

    #[cfg(feature = "painted")]
    #[test]
    fn automatic_walk_finishes_an_edge_crossing_before_stopping() {
        let (width, margin) = (1200.0, 107.0);
        for seed in 0..8 {
            for facing in [-1.0, 1.0] {
                let mut motion = Motion::with_seed(seed);
                motion.wrap_at_edges(margin);
                motion.facing = facing;
                motion.set_x(if facing < 0.0 { margin } else { width - margin });
                motion.set_ground(220.0, 500.0);
                let mut wrapped = false;
                for _ in 0..600 {
                    let previous = motion.x();
                    motion.update(1.0 / 120.0, width);
                    assert_eq!(motion.facing(), facing);
                    assert_eq!(motion.sprite_clip().0, "walk");
                    wrapped |= (previous - motion.x()).abs() > width / 2.0;
                    if wrapped && (margin..=width - margin).contains(&motion.x()) {
                        break;
                    }
                }
                assert!(wrapped && (margin..=width - margin).contains(&motion.x()));
            }
        }
    }

    fn walking() -> Motion {
        let mut motion = Motion::new();
        motion.mode = Mode::Walk;
        motion.action = Mode::Walk;
        motion.phase = 0.0;
        motion.walk_weight = 1.0;
        motion.transition_time = 0.28;
        motion.set_x(1000.0);
        motion.width = 10_000.0;
        motion
    }

    #[test]
    fn head_holds_world_position_while_the_body_advances() {
        let mut motion = walking();
        motion.phase = 0.04;
        let before = motion.x() + motion.pose().head_x;
        motion.update(0.08, 10_000.0);
        let after = motion.x() + motion.pose().head_x;
        assert!(
            (before - after).abs() < 0.001,
            "head drift: {}",
            after - before
        );
        assert!(motion.x() > 1005.0);
    }

    #[test]
    fn planted_foot_does_not_slide() {
        let mut motion = walking();
        motion.phase = 0.1;
        let before = motion.x() + motion.pose().near_foot_x;
        motion.update(0.08, 10_000.0);
        let after = motion.x() + motion.pose().near_foot_x;
        assert!((before - after).abs() < 0.001);
        assert_eq!(motion.pose().near_foot_y, 72.0);
    }

    #[cfg(feature = "painted")]
    #[test]
    fn swinging_toes_leave_and_reach_the_floor_at_rest() {
        let world = |phase: f32| {
            let (x, y) = foot_position(phase.rem_euclid(1.0));
            (phase * STRIDE + x, y)
        };
        let epsilon = 0.0001;
        for contact in [0.64, 1.0] {
            let (x, y) = world(contact);
            for next in [contact - epsilon, contact + epsilon] {
                let (nx, ny) = world(next);
                assert!((nx - x).hypot(ny - y) < 3.0 * epsilon);
            }
        }
        assert_eq!(world(0.82).1, 57.0);
        // Easing contact must not create a faster sideways flick halfway through the step.
        let old_peak = STRIDE * (1.0 + 1.5 * 0.64 / 0.36);
        for i in 0..1024 {
            let dx = world((i + 1) as f32 / 1024.0).0 - world(i as f32 / 1024.0).0;
            assert!((-0.001..=old_peak / 1024.0 + 0.001).contains(&dx));
        }
    }

    #[cfg(feature = "painted")]
    #[test]
    fn stopping_lands_on_the_same_two_toes_as_idle_and_the_next_stride() {
        for speed in [1.0, 1.5, 2.0] {
            for hz in [60, 120, 144] {
                for phase in [STANDING_PHASE, 0.13, 0.45, STANDING_PHASE + 0.5, 0.93] {
                    let mut m = walking();
                    m.phase = phase;
                    m.set_speed(speed);
                    let start = m.x;
                    m.set_mode(Mode::Rest);
                    for _ in 0..hz {
                        m.update(1.0 / hz as f32, 10_000.0);
                        if m.action != Mode::Walk {
                            break;
                        }
                    }
                    assert_eq!(m.action, Mode::Look);
                    assert_eq!(m.phase, STANDING_PHASE);
                    let final_step = (STANDING_PHASE - phase).rem_euclid(1.0) * STRIDE;
                    assert!((m.x - start - final_step).abs() < 0.005);
                    let idle = Pose::standing(m.facing);
                    let landing = walk_pose(m.phase, m.facing);
                    assert_eq!(
                        (landing.near_foot_x, landing.far_foot_x),
                        (idle.near_foot_x, idle.far_foot_x)
                    );
                    assert_eq!(
                        (landing.near_foot_y, landing.far_foot_y),
                        (idle.near_foot_y, idle.far_foot_y)
                    );
                }
            }
        }
    }

    #[test]
    fn high_refresh_and_lower_refresh_cover_the_same_distance() {
        let mut at_60 = walking();
        let mut at_144 = walking();
        for _ in 0..600 {
            at_60.update(1.0 / 60.0, 10_000.0);
        }
        for _ in 0..1440 {
            at_144.update(1.0 / 144.0, 10_000.0);
        }
        assert!((at_60.x() - at_144.x()).abs() < 0.25);
        assert!((at_60.pose().head_x - at_144.pose().head_x).abs() < 0.25);
    }

    #[test]
    fn loop_has_no_pose_jump() {
        let first = walk_pose(0.0, 1.0);
        let last = walk_pose(1.0 - 0.00001, 1.0);
        assert!((first.head_x - last.head_x).abs() < 0.01);
        assert!((first.near_foot_x - last.near_foot_x).abs() < 0.01);
        assert!((first.near_foot_y - last.near_foot_y).abs() < 0.01);
    }

    #[test]
    fn speed_changes_distance_without_changing_stride_length() {
        let mut slow = walking();
        let mut fast = walking();
        slow.set_speed(0.5);
        slow.current_speed = 0.5;
        fast.set_speed(2.0);
        fast.current_speed = 2.0;
        for _ in 0..240 {
            slow.update(1.0 / 240.0, 10_000.0);
            fast.update(1.0 / 240.0, 10_000.0);
        }
        assert!(((fast.x() - 1000.0) / (slow.x() - 1000.0) - 4.0).abs() < 0.01);
    }

    #[test]
    fn actions_transition_smoothly_and_inputs_stay_finite() {
        let mut motion = walking();
        let before = motion.pose();
        motion.set_mode(Mode::Peck);
        let after = motion.pose();
        assert_eq!(before.head_x, after.head_x);
        assert_eq!(before.head_y, after.head_y);
        motion.set_speed(f32::NAN);
        motion.update(f32::NAN, f32::INFINITY);
        assert!(motion.x().is_finite());
        let mut deepest = -61.0_f32;
        for _ in 0..720 {
            motion.update(1.0 / 240.0, 1000.0);
            deepest = deepest.max(motion.pose().head_y);
        }
        assert!(deepest > 40.0);
        assert!(motion.walk_weight < 0.001);
    }

    #[test]
    #[cfg(not(feature = "painted"))]
    fn auto_cycle_changes_action_at_every_boundary_including_wrap() {
        let mut motion = Motion::new();
        let mut boundaries = 0;
        let duration = AUTO_ACTIONS
            .iter()
            .map(|(_, duration)| duration)
            .sum::<f32>();
        // The schedule sets minimum dwell times; finish each step or head movement before leaving.
        for _ in 0..((duration + 4.0) * 100.0) as usize {
            let previous_index = motion.auto_index;
            let previous_action = motion.action;
            motion.update(0.01, 10_000.0);
            if motion.auto_index != previous_index {
                assert_ne!(motion.action, previous_action);
                boundaries += 1;
                if boundaries == AUTO_ACTIONS.len() {
                    break;
                }
            }
        }
        assert_eq!(boundaries, AUTO_ACTIONS.len());
    }

    #[test]
    fn sprite_sample_tracks_actual_action_and_distance() {
        let mut motion = walking();
        let start = motion.x();
        motion.update(0.1, 10_000.0);
        let (action, phase) = motion.sprite_sample();
        assert_eq!(action, Mode::Walk);
        assert!((phase * STRIDE - (motion.x() - start)).abs() < 0.001);
        motion = Motion::with_seed(19);
        for _ in 0..67 {
            motion.update(0.1, 10_000.0);
        }
        assert_eq!(motion.mode(), Mode::Auto);
        if !cfg!(feature = "painted") {
            assert_eq!(motion.sprite_sample().0, Mode::Look);
        }
        assert!((0.0..1.0).contains(&motion.sprite_sample().1));
    }

    #[test]
    #[cfg(not(feature = "painted"))]
    fn auto_pauses_before_turning_and_varies_reproducible_dwell_times() {
        fn itinerary() -> Vec<(Mode, u32)> {
            let mut motion = Motion::new();
            motion.set_x(200.0);
            let mut events = Vec::new();
            let mut last_change = 0;
            let mut turns = 0;
            for tick in 1_u32..=144 * 120 {
                let before = motion.action;
                let facing = motion.facing;
                motion.update(1.0 / 144.0, 320.0);
                assert!((0.0..=320.0).contains(&motion.x()));
                if motion.facing != facing {
                    assert_ne!(before, Mode::Walk);
                    assert_eq!(motion.action, Mode::Walk);
                    assert!(if motion.facing > 0.0 {
                        motion.x() < 34.0
                    } else {
                        motion.x() > 286.0
                    });
                    turns += 1;
                }
                if motion.action != before {
                    events.push((before, tick - last_change));
                    last_change = tick;
                }
            }
            assert!(turns >= 2);
            assert!(events.iter().any(|(m, _)| *m == Mode::Peck));
            // Compare the same scheduled look, rather than the two look slots with different baselines.
            let durations: Vec<_> = events.iter().skip(1).step_by(AUTO_ACTIONS.len()).collect();
            assert!(durations.iter().all(|(m, _)| *m == Mode::Look));
            assert!(
                durations
                    .windows(2)
                    .any(|pair| pair[0].1.abs_diff(pair[1].1) > 8)
            );
            events
        }
        assert_eq!(itinerary(), itinerary());
    }

    #[cfg(feature = "painted")]
    #[test]
    fn seeded_roaming_pauses_to_turn_and_varies_gait_pace_and_destination() {
        fn sample(seed: u32, hz: i32) -> Vec<(&'static str, i32)> {
            let mut m = Motion::with_seed(seed);
            m.set_x(320.0);
            m.set_ground(120.0, 200.0);
            m.update(0.0, 640.0);
            let mut itinerary = vec![(m.sprite_clip().0, m.facing as i32)];
            let (mut turns_inside, mut resumes_straight) = (0, 0);
            let (mut deepest, mut highest) = (m.ground(), m.ground());
            let mut pace_range = (f32::MAX, 0.0_f32);
            for _ in 0..hz * 150 {
                let (prior, direction, x, y) = (m.sprite_clip().0, m.facing, m.x, m.ground);
                m.update(1.0 / hz as f32, 640.0);
                let clip = m.sprite_clip().0;
                assert!((0.0..=640.0).contains(&m.x) && (0.0..=200.0).contains(&m.ground));
                if direction != m.facing {
                    assert_eq!((prior, clip), ("turn", "turn"));
                    let frames = sprite_frame_count("turn", true);
                    assert_eq!((m.sprite_clip().1 * frames as f32) as u32, frames - 1);
                    if (50.0..590.0).contains(&x) {
                        turns_inside += 1;
                    }
                } else if matches!(prior, "look" | "peck") && matches!(clip, "walk" | "settle") {
                    resumes_straight += 1;
                }
                if matches!(clip, "look" | "peck" | "settle" | "turn") && prior == clip {
                    assert_eq!((m.x, m.ground), (x, y), "sliding during {clip}");
                }
                if clip == "settle" {
                    assert!((m.phase - STANDING_PHASE).abs() < 0.001);
                }
                if clip == "stalk" && prior != "stalk" {
                    assert_eq!(prior, "settle");
                }
                if prior == "stalk" && clip != "stalk" {
                    assert_eq!(clip, "settle");
                }
                if matches!(clip, "walk" | "stalk") && m.walk_weight > 0.95 {
                    pace_range = (pace_range.0.min(m.roam.pace), pace_range.1.max(m.roam.pace));
                }
                if prior != clip {
                    itinerary.push((clip, m.facing as i32));
                }
                deepest = deepest.max(m.ground);
                highest = highest.min(m.ground);
            }
            assert!(deepest - highest > 12.0 && pace_range.1 - pace_range.0 > 0.50);
            assert!(turns_inside >= 2 && resumes_straight >= 2);
            for clip in ["walk", "stalk", "settle", "turn", "look", "peck"] {
                assert!(itinerary.iter().any(|(name, _)| *name == clip));
            }
            itinerary
        }
        let first = sample(19, 60);
        assert_eq!(first, sample(19, 60));
        assert_ne!(first, sample(467, 60));
        assert_eq!(first[..20], sample(19, 144)[..20]);
    }

    #[cfg(feature = "painted")]
    #[test]
    fn menu_change_during_turn_finishes_on_the_new_side_before_idle() {
        let mut m = Motion::with_seed(19);
        m.set_x(320.0);
        m.set_ground(100.0, 200.0);
        for _ in 0..60 * 60 {
            m.update(1.0 / 60.0, 640.0);
            if m.roam.turn.is_some_and(|t| t < 0.08) {
                let (x, y, direction) = (m.x, m.ground, m.facing);
                m.set_mode(Mode::Peck);
                let mut turns = 0;
                for _ in 0..90 {
                    let old = m.facing;
                    m.update(1.0 / 60.0, 640.0);
                    if m.facing != old {
                        turns += 1;
                    }
                    assert_eq!((m.x, m.ground), (x, y));
                    if m.sprite_clip().0 == "peck" {
                        assert_eq!(turns, 1);
                        assert_eq!(m.facing, -direction);
                        return;
                    }
                }
                break;
            }
        }
        panic!("expected a full stationary turn and the requested peck");
    }

    #[cfg(feature = "painted")]
    #[test]
    fn foraging_revisits_a_reachable_patch_with_a_smaller_rebound_then_moves_on() {
        let mut m = Motion::with_seed(1);
        m.set_x(320.0);
        m.set_ground(108.0, 180.0);
        m.update(0.0, 640.0);
        let food = m.patch.expect("selected patch");
        let mut normal_tap = false;
        let mut fast_tap = false;
        let mut revisits = 0;
        let mut last = m.action;
        let mut checked_low_rebound = false;
        let mut moved = false;
        for _ in 0..60 * 10 {
            m.update(1.0 / 60.0, 640.0);
            if m.patch.is_none() {
                assert!(m.search_after > STRIDE);
                moved = true;
                break;
            }
            if m.action == Mode::Peck {
                let patch = m.patch.unwrap();
                assert_eq!((patch.x, patch.ground), (food.x, food.ground));
                assert!(m.at_patch());
                normal_tap |= !m.low_retap;
                fast_tap |= m.low_retap;
                if m.low_retap && (0.71..0.75).contains(&m.action_time) {
                    assert!((0.52..0.65).contains(&peck_depth(m.peck_time())));
                    checked_low_rebound = true;
                }
                if m.low_retap && (0.875..0.89).contains(&m.action_time) {
                    assert!(peck_depth(m.peck_time()) > 0.99);
                }
                if last != Mode::Peck {
                    revisits += 1;
                }
            } else if last == Mode::Peck {
                assert!(normal_tap);
            }
            last = m.action;
        }
        assert!(normal_tap && fast_tap && checked_low_rebound && revisits >= 2 && moved);
    }

    #[test]
    fn interrupting_a_peck_waits_for_an_upright_sprite_and_starts_on_planted_feet() {
        let mut motion = Motion::new();
        motion.set_mode(Mode::Peck);
        for _ in 0..55 {
            motion.update(0.01, 10_000.0);
        }
        assert!(peck_depth(motion.action_time) > 0.99);
        let x = motion.x();
        motion.set_mode(Mode::Walk);
        for _ in 0..240 {
            let previous_depth = peck_depth(motion.action_time);
            motion.update(1.0 / 240.0, 10_000.0);
            if motion.sprite_sample().0 == Mode::Walk {
                assert!(previous_depth < 0.015);
                let step = walk_pose(motion.phase, 1.0);
                assert_eq!((step.near_foot_y, step.far_foot_y), (72.0, 72.0));
                return;
            }
            assert_eq!(motion.x(), x);
        }
        panic!("walk did not start after the peck recovered");
    }

    #[test]
    fn stopping_walk_finishes_the_step_and_idle_speed_scales_both_clips() {
        let mut motion = walking();
        motion.phase = 0.31;
        motion.set_mode(Mode::Look);
        for _ in 0..240 {
            let was_safe = motion.can_leave_action();
            motion.update(1.0 / 240.0, 10_000.0);
            if motion.sprite_sample().0 == Mode::Look {
                assert!(was_safe);
                break;
            }
        }
        assert_eq!(motion.sprite_sample().0, Mode::Look);
        let resting_x = motion.x();
        for _ in 0..30 {
            motion.update(0.01, 10_000.0);
        }
        assert_eq!(motion.x(), resting_x);

        for (action, duration) in [(Mode::Look, 3.4), (Mode::Peck, 2.8)] {
            for speed in [0.5, 2.0] {
                let mut motion = Motion::new();
                motion.set_mode(action);
                motion.set_speed(speed);
                motion.current_speed = speed;
                for _ in 0..60 {
                    motion.update(1.0 / 60.0, 1000.0);
                }
                assert!((motion.action_time - speed).abs() < 0.001);
                let expected = if action == Mode::Peck {
                    peck_sprite_phase(motion.action_time)
                } else {
                    motion.action_time / duration
                };
                assert!((motion.sprite_sample().1 - expected).abs() < 0.001);
            }
        }
    }

    #[cfg(feature = "painted")]
    #[test]
    fn half_speed_idle_motion_can_change_pose_at_every_144hz_tick() {
        for (clip, seconds) in [("look", 3.4), ("peck", PECK_SECONDS)] {
            let frames = sprite_frame_count(clip, true);
            let mut previous = None;
            for tick in 0..(seconds * 288.0) as usize {
                let t = tick as f32 / 288.0;
                let phase = if clip == "peck" {
                    peck_sprite_phase(t)
                } else {
                    t / seconds
                };
                let index = (phase * frames as f32) as u32;
                assert!(index < frames && previous != Some(index));
                previous = Some(index);
            }
        }
        assert_eq!(peck_sprite_phase(PECK_SECONDS), 0.0);
        for (start, tempo) in [(0.27, 1.0), (1.36, 1.16)] {
            for (offset, depth) in [(0.0, 0.0), (0.1, 0.14), (0.29, 1.0), (0.58, 0.0)] {
                assert!((peck_depth(start + offset / tempo) - depth).abs() < 0.001);
            }
        }
    }

    #[test]
    fn rest_settles_then_varies_idle_actions_in_place_and_can_resume() {
        for speed in [1.0, 1.5, 2.0] {
            let mut m = Motion::with_seed(35);
            m.set_x(400.0);
            m.set_ground(84.0, 200.0);
            m.set_mode(Mode::Walk);
            m.set_speed(speed);
            for _ in 0..35 {
                m.update(1.0 / 120.0, 900.0);
            }
            let entry_x = m.x();
            m.set_mode(Mode::Rest);
            assert_eq!(m.mode(), Mode::Rest);
            for _ in 0..120 {
                m.update(1.0 / 120.0, 900.0);
                if m.action == Mode::Look {
                    break;
                }
            }
            assert_eq!(m.action, Mode::Look);
            let settled = (m.x(), m.ground(), m.facing());
            let longest_finish = if cfg!(feature = "painted") {
                STRIDE
            } else {
                STRIDE * 0.5
            };
            assert!((settled.0 - entry_x).abs() < longest_finish);
            let mut last_switch = 0;
            let mut peck_durations = Vec::new();
            for tick in 1..=3600 {
                let preceding = m.action;
                m.update(1.0 / 120.0, 900.0);
                assert!(matches!(m.sprite_clip().0, "look" | "peck"));
                assert_eq!((m.x(), m.ground(), m.facing()), settled);
                if m.action != preceding {
                    if preceding == Mode::Peck {
                        peck_durations.push(tick - last_switch);
                    }
                    last_switch = tick;
                }
            }
            assert!(peck_durations.len() >= 4);
            assert!(peck_durations.iter().min() != peck_durations.iter().max());
            m.set_mode(Mode::Auto);
            for _ in 0..360 {
                m.update(1.0 / 120.0, 900.0);
                if (m.x() - settled.0).abs() > 2.0 {
                    break;
                }
            }
            assert!((m.x() - settled.0).abs() > 2.0);
        }
    }

    #[test]
    fn selecting_rest_during_a_peck_waits_for_the_head_to_rise() {
        let mut m = Motion::with_seed(7);
        m.set_x(400.0);
        m.set_mode(Mode::Peck);
        for _ in 0..130 {
            m.update(1.0 / 240.0, 900.0);
        }
        let depth = peck_depth(m.peck_time());
        assert!(depth > 0.9);
        m.set_mode(Mode::Rest);
        assert_eq!(m.action, Mode::Peck);
        assert_eq!(peck_depth(m.peck_time()), depth);
        for _ in 0..240 {
            let could_leave = m.can_leave_action();
            m.update(1.0 / 240.0, 900.0);
            assert_eq!(m.x(), 400.0);
            if m.action == Mode::Look {
                assert!(could_leave);
                return;
            }
        }
        panic!("the resting bird did not finish its peck");
    }

    #[test]
    fn manual_interrupt_finishes_at_a_safe_pose_within_a_short_beat() {
        let mut worst = 0.0_f32;
        for rate in [60, 120, 144] {
            let dt = 1.0 / rate as f32;
            for speed in [0.5, 1.0, 1.5, 2.0] {
                for from in [Mode::Walk, Mode::Look, Mode::Peck] {
                    for to in [Mode::Walk, Mode::Look, Mode::Peck] {
                        if from == to {
                            continue;
                        }
                        for index in 0..128 {
                            let mut m = walking();
                            m.action = from;
                            m.mode = from;
                            m.phase = index as f32 / 128.0;
                            m.action_time = m.phase * if from == Mode::Look { 3.4 } else { 2.8 };
                            m.speed = speed;
                            m.current_speed = speed;
                            let angle = look_direction(m.action_time);
                            m.set_mode(to);
                            if m.action == Mode::Look && from == Mode::Look {
                                assert!((look_direction(m.action_time) - angle).abs() < 0.0001);
                            }
                            let mut delay = 0.0;
                            while m.sprite_sample().0 != to && delay <= 0.62 {
                                m.update(dt, 10000.0);
                                delay += dt;
                            }
                            worst = worst.max(delay);
                            assert!(
                                delay <= 0.62,
                                "{from:?}->{to:?} phase {index} speed {speed} waited {delay}"
                            );
                        }
                    }
                }
            }
        }
        println!("Longest manual transition across 9216 cases: {worst:.3}s");
    }
}
