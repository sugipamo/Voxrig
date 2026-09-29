//! Piston block actions and independent client moving-block lifetimes.
use super::rules::facing;
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Extend,
    Retract,
    Drop,
}
impl Action {
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Extend),
            1 => Some(Self::Retract),
            2 => Some(Self::Drop),
            _ => None,
        }
    }
}
impl Reconstruction {
    pub fn action(
        &mut self,
        world: &World,
        p: Pos,
        action: Action,
        parameter: Direction,
        block_name: &str,
        seq: u64,
    ) {
        if self.issue.is_some() {
            return;
        }
        let mut next = self.clone();
        match next.apply_action(world, p, action, parameter, block_name, seq) {
            Ok(()) => *self = next,
            Err(issue) => {
                self.issue = Some(issue);
                self.revision += 1;
            }
        }
    }
    fn apply_action(
        &mut self,
        world: &World,
        p: Pos,
        action: Action,
        parameter: Direction,
        block_name: &str,
        seq: u64,
    ) -> ApplyResult<()> {
        let body = self.read(world, p)?;
        // Vanilla dispatches the action only while the current block identity still matches.
        if body.name != block_name {
            return Ok(());
        }
        let Some(Kind::Piston { sticky }) = classify(&body) else {
            return Err(ReconstructionIssue::InvalidAction);
        };
        let dir = facing(&body)?;
        let head = dir.offset(p, 1);
        if action == Action::Extend {
            self.move_line(world, head, dir, dir, true, seq)?;
            let carried = state(
                "piston_head",
                &[
                    ("facing", dir.name()),
                    ("type", if sticky { "sticky" } else { "normal" }),
                    ("short", "false"),
                ],
            );
            self.install(
                world,
                MovingBlock {
                    position: head,
                    carried,
                    direction: dir,
                    extending: true,
                    role: CarrierRole::Head,
                    progress: MotionProgress::Start,
                    last_progress: MotionProgress::Start,
                    completion_waits: 0,
                    action_sequence: seq,
                },
                sticky,
            )?;
            let mut extended = body;
            extended.properties.insert("extended".into(), "true".into());
            self.put(world, p, extended, seq, true, 0)?;
        } else {
            self.finish(world, head, true)?;
            let carried = state(
                if sticky { "sticky_piston" } else { "piston" },
                &[("facing", parameter.name()), ("extended", "false")],
            );
            self.install(
                world,
                MovingBlock {
                    position: p,
                    carried,
                    direction: dir,
                    extending: false,
                    role: CarrierRole::Body,
                    progress: MotionProgress::Start,
                    last_progress: MotionProgress::Start,
                    completion_waits: 0,
                    action_sequence: seq,
                },
                sticky,
            )?;
            self.neighbors(world, p, seq, 0)?;
            let payload = dir.offset(p, 2);
            let finishing_payload = sticky
                && self
                    .moving
                    .get(&payload)
                    .is_some_and(|m| m.direction == dir && m.extending);
            if finishing_payload {
                self.finish(world, payload, true)?;
            } else {
                self.put(world, head, state("air", &[]), seq, true, 0)?;
                if sticky && action == Action::Retract {
                    let (s, kind) = self.kind(world, payload)?;
                    if Self::movable(&s, kind) {
                        self.move_line(world, payload, dir.opposite(), dir, false, seq)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn movable(s: &NativeBlockState, kind: Kind) -> bool {
        match kind {
            Kind::Solid | Kind::Stairs => true,
            Kind::Piston { .. } => s.properties.get("extended").is_some_and(|s| s == "false"),
            _ => false,
        }
    }
    fn move_line(
        &mut self,
        world: &World,
        start: Pos,
        movement: Direction,
        facing: Direction,
        extending: bool,
        seq: u64,
    ) -> ApplyResult<()> {
        let mut moved = Vec::new();
        let mut destroyed = None;
        for distance in 0..=12 {
            let p = movement.offset(start, distance);
            let (s, kind) = self.kind(world, p)?;
            if kind == Kind::Air {
                break;
            }
            if kind == Kind::Lever && extending {
                destroyed = Some(p);
                break;
            }
            if !Self::movable(&s, kind) || distance == 12 {
                return Err(ReconstructionIssue::InvalidAction);
            }
            moved.push((p, s));
            // Retraction pulls one stable payload. Branching adhesion remains explicitly unsupported.
            if !extending {
                break;
            }
        }
        if let Some(p) = destroyed {
            self.put(world, p, state("air", &[]), seq, false, 0)?;
        }
        let destinations: Vec<_> = moved.iter().map(|(p, _)| movement.offset(*p, 1)).collect();
        for (p, carried) in moved.iter().rev() {
            let destination = movement.offset(*p, 1);
            self.install(
                world,
                MovingBlock {
                    position: destination,
                    carried: carried.clone(),
                    direction: facing,
                    extending,
                    role: CarrierRole::Payload,
                    progress: MotionProgress::Start,
                    last_progress: MotionProgress::Start,
                    completion_waits: 0,
                    action_sequence: seq,
                },
                false,
            )?;
        }
        for (p, _) in &moved {
            if !destinations.contains(p) && !(extending && *p == start) {
                self.put(world, *p, state("air", &[]), seq, false, 0)?;
                self.neighbors(world, *p, seq, 0)?;
            }
        }
        if let Some(p) = destroyed {
            self.neighbors(world, p, seq, 0)?;
        }
        Ok(())
    }
    fn install(&mut self, world: &World, motion: MovingBlock, sticky: bool) -> ApplyResult<()> {
        if self.moving.len() >= 4096 {
            return Err(ReconstructionIssue::Limit);
        }
        let moving = state(
            "moving_piston",
            &[
                ("facing", motion.direction.name()),
                ("type", if sticky { "sticky" } else { "normal" }),
            ],
        );
        let shape = motion.role != CarrierRole::Body; // flags 324 vs 276
        self.put(
            world,
            motion.position,
            moving,
            motion.action_sequence,
            shape,
            0,
        )?;
        self.order.retain(|p| *p != motion.position);
        self.order.push(motion.position);
        self.moving.insert(motion.position, motion);
        Ok(())
    }
    fn finish(&mut self, world: &World, p: Pos, forced: bool) -> ApplyResult<()> {
        let Some(motion) = self.moving.get(&p).cloned() else {
            return Ok(());
        };
        if self.read(world, p)?.name != "minecraft:moving_piston" {
            self.moving.remove(&p);
            self.order.retain(|pos| *pos != p);
            return Ok(());
        }
        let mut after = if forced && motion.role != CarrierRole::Payload {
            state("air", &[])
        } else {
            motion.carried
        };
        if classify(&after) == Some(Kind::Stairs) {
            after = self.stair_shape(world, p, after)?;
        }
        self.put(world, p, after, motion.action_sequence, true, 0)?;
        self.moving.remove(&p);
        self.order.retain(|pos| *pos != p);
        Ok(())
    }
    pub(super) fn step(&mut self, world: &World) -> ApplyResult<()> {
        for p in self.order.clone() {
            let Some(m) = self.moving.get_mut(&p) else {
                continue;
            };
            m.last_progress = m.progress;
            match m.progress {
                MotionProgress::Start => m.progress = MotionProgress::Half,
                MotionProgress::Half => m.progress = MotionProgress::Full,
                MotionProgress::Full if m.completion_waits < 5 => m.completion_waits += 1,
                MotionProgress::Full => {
                    self.finish(world, p, false)?;
                }
            }
            self.revision += 1;
        }
        Ok(())
    }
}
