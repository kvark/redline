//! AI racecraft: where on the ribbon a driver wants to be and how hard to push.
//!
//! Pure decision logic — no physics. [`crate::ai::Driver`] turns the returned
//! [`Plan`] into the same steering / target-speed inputs the player uses, so
//! every pass, block and pad run is still carried out by the Rapier vehicle.
//!
//! Lateral is signed metres from the ribbon centre along `normal × tangent`
//! (the same convention as [`crate::planet::TrackQuery::lateral`]).

/// Lateral separation (m) that keeps two craft side by side without touching.
pub const SIDE_GAP: f32 = 2.7;
/// Treat a rival this far ahead (m) as traffic worth reacting to.
pub const FOLLOW_RANGE: f32 = 14.0;
/// Rivals within this along-track band (m) are alongside.
pub const ALONGSIDE: f32 = 4.5;
/// Closing speed (m/s) above which a car behind is lapping traffic: yield, don't block.
pub const LAPPING_CLOSING: f32 = 6.0;
/// Look this far back (m) for lapping traffic.
pub const YIELD_RANGE: f32 = 36.0;
/// A car ahead pulling away slower than this (m/s) is still worth attacking;
/// the attack pace does the closing.
pub const ATTACK_CLOSING: f32 = 0.3;
/// Pace multiplier while committed to a pass (intent through target speed).
pub const ATTACK_PACE: f32 = 1.08;
/// Pace while sitting in a rival's wake waiting for room.
pub const TUCK_PACE: f32 = 1.04;
/// Give up on a pass that has not cleared after this long (s).
pub const PASS_TIMEOUT: f32 = 5.0;
/// Hold a fresh commitment at least this long before aborting (s), so the AI
/// does not twitch between sides.
pub const PASS_MIN_COMMIT: f32 = 0.6;
/// Pass is complete once the rival is this far behind (m).
pub const PASS_CLEAR: f32 = 3.5;
/// Max lateral target slew (m/s) on the line, while defending and when passing.
const LINE_SLEW: f32 = 2.4;
const PASS_SLEW: f32 = 3.0;
const YIELD_SLEW: f32 = 4.0;
/// Fraction of the usable half-ribbon a yielding car moves out to.
const YIELD_EDGE: f32 = 0.7;
/// Start moving over this many seconds before a lapping car arrives.
const YIELD_WARNING: f32 = 2.0;
/// Defend a follower between these distances behind (m): start, hold, range.
const DEFEND_START: f32 = 5.0;
const DEFEND_HOLD: f32 = 3.0;
const DEFEND_RANGE: f32 = 12.0;
/// Seconds a car that lapped us stays exempt from blocking.
const YIELD_MEMORY: f32 = 5.0;
/// Most a defender moves off its line to cover a follower (m).
const DEFEND_REACH: f32 = 2.2;
/// Racing line offset (m) per radian of upcoming turn.
pub const LINE_GAIN: f32 = 9.0;
/// Fraction of the usable half-ribbon the racing line may use.
const LINE_SPAN: f32 = 0.6;
/// Pads closer than this (m) pull the line onto their footprint.
pub const PAD_LOOKAHEAD: f32 = 42.0;
/// An attacker at least this far behind its target (m) runs over a pad first.
const PAD_RUN_GAP: f32 = 7.0;
/// A yielding car can still clip a pad this far inside its edge (m).
const PAD_EDGE_MARGIN: f32 = 0.4;
/// Stay this far inside a pad's edge (m) when aiming for it.
const PAD_MARGIN: f32 = 2.6;

/// Pack pace: cars behind the AI pack's centre find a little extra (they are
/// chasing tows), the pack leader runs in dirty air. Keeps the AI together so
/// they keep racing each other instead of spreading out from the grid.
pub const PACK_GAIN: f32 = 0.004;
pub const PACK_PUSH: f32 = 0.05;
pub const PACK_EASE: f32 = 0.03;

/// Pace multiplier for a car `behind` metres behind the pack centre (negative when ahead).
pub fn pack_pace(behind: f32) -> f32 {
    1.0 + (behind * PACK_GAIN).clamp(-PACK_EASE, PACK_PUSH)
}

/// Another racer, relative to the driver doing the planning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rival {
    /// Stable racer id (0 = player).
    pub id: usize,
    /// Along-track distance (m); positive when the rival is ahead.
    pub ahead: f32,
    pub lateral: f32,
    /// My speed minus theirs (m/s); positive when I am catching them.
    pub closing: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadAhead {
    /// Distance (m) to the pad centre.
    pub distance: f32,
    /// Pad centre lateral (m).
    pub lateral: f32,
    pub half_width: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Situation<'a> {
    pub lateral: f32,
    /// Largest |lateral| (m) a craft centre should use.
    pub half_room: f32,
    /// Signed heading change (rad) over the next stretch; + bends toward +lateral.
    pub turn: f32,
    pub pad: Option<PadAhead>,
    pub rivals: &'a [Rival],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// Racing line (and pads).
    Line,
    /// Following a rival, waiting for a gap.
    Tuck { target: usize },
    /// Committed to pass `target` on `side` (+1 / -1).
    Pass { target: usize, side: f32, time: f32 },
    /// Covering a follower with one move to `cover` (held, not re-aimed).
    Defend { target: usize, cover: f32 },
    /// Moving aside for a much faster car.
    Yield { target: usize, side: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub lateral: f32,
    pub pace: f32,
}

/// Per-race racecraft counters for logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tally {
    pub passes_committed: u32,
    pub passes_completed: u32,
    pub passes_aborted: u32,
    pub defends: u32,
    pub switchbacks: u32,
}

#[derive(Clone, Debug)]
pub struct Planner {
    pub mode: Mode,
    lateral: f32,
    /// Seconds before a new pass may start after an abort / completion.
    cooldown: f32,
    /// A car that recently lapped / out-paced us: never block it (id, secs left).
    lapped_by: Option<(usize, f32)>,
    pub tally: Tally,
}

impl Planner {
    pub fn new(lateral: f32) -> Self {
        Self {
            mode: Mode::Line,
            lateral,
            cooldown: 0.0,
            lapped_by: None,
            tally: Tally::default(),
        }
    }

    /// Restart racecraft (after a teleport): back on the line at `lateral`.
    pub fn reset_to(&mut self, lateral: f32) {
        self.mode = Mode::Line;
        self.lateral = lateral;
        self.cooldown = 0.5;
    }

    pub fn plan(&mut self, sit: &Situation, dt: f32) -> Plan {
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.lapped_by = self
            .lapped_by
            .map(|(id, t)| (id, t - dt))
            .filter(|&(_, t)| t > 0.0);
        self.mode = self.next_mode(sit, dt);
        if let Mode::Yield { target, .. } = self.mode {
            self.lapped_by = Some((target, YIELD_MEMORY));
        }
        let line = racing_line(sit.turn, sit.half_room);
        let (mut target, pace, slew) = match self.mode {
            Mode::Line => (line, 1.0, LINE_SLEW),
            Mode::Tuck { target } => {
                let rival = find(sit.rivals, target).expect("tucked rival");
                // Sit in the wake slightly offset, ready to pull out, and
                // never close faster than the gap allows.
                let pace = if rival.ahead < 6.0 && rival.closing > 0.0 {
                    1.0 - (rival.closing * 0.04).min(0.25)
                } else {
                    TUCK_PACE
                };
                (rival.lateral, pace, LINE_SLEW)
            }
            Mode::Pass { target, side, .. } => {
                let rival = find(sit.rivals, target).expect("passed rival");
                let slot = (rival.lateral + side * SIDE_GAP).clamp(-sit.half_room, sit.half_room);
                // Still a few lengths back with a pad coming: take the pad in
                // the rival's wake and pull out with the boost.
                let lateral = match sit.pad {
                    Some(pad) if rival.ahead > PAD_RUN_GAP && pad.distance < rival.ahead => {
                        pad_target(slot, pad)
                    }
                    _ => slot,
                };
                (lateral, ATTACK_PACE, PASS_SLEW)
            }
            Mode::Defend { cover, .. } => (cover, 1.0, LINE_SLEW),
            Mode::Yield { target, side } => {
                let rival = find(sit.rivals, target).expect("yielded rival");
                // Stay on the line (and pads) if that already leaves room;
                // else ride the pad's far edge; else move out to the edge.
                let preferred = sit.pad.map_or(line, |pad| pad_target(line, pad));
                let pad_edge = sit
                    .pad
                    .filter(|pad| pad.distance <= PAD_LOOKAHEAD)
                    .map(|pad| pad.lateral + side * (pad.half_width - PAD_EDGE_MARGIN));
                let lateral = if (preferred - rival.lateral) * side >= SIDE_GAP * 1.2 {
                    preferred
                } else if let Some(edge) =
                    pad_edge.filter(|edge| (edge - rival.lateral) * side >= SIDE_GAP)
                {
                    edge
                } else {
                    yield_lateral(side, rival.lateral, sit.half_room)
                };
                (lateral, 1.0, YIELD_SLEW)
            }
        };
        if matches!(
            self.mode,
            Mode::Line | Mode::Tuck { .. } | Mode::Defend { .. }
        ) && let Some(pad) = sit.pad
        {
            target = pad_target(target, pad);
        }
        target = keep_clear(target, sit.lateral, sit.rivals, sit.half_room);
        let step = slew * dt;
        self.lateral += (target - self.lateral).clamp(-step, step);
        // Slew keeps lines smooth, but never at the cost of steering into a
        // car that is already alongside.
        self.lateral = keep_clear(self.lateral, sit.lateral, sit.rivals, sit.half_room);
        Plan {
            lateral: self.lateral,
            pace,
        }
    }

    fn next_mode(&mut self, sit: &Situation, dt: f32) -> Mode {
        if matches!(self.mode, Mode::Pass { .. })
            && let Some(mode) = lapping_threat(sit)
        {
            // Lapping traffic outranks a pass: abort and open the door.
            self.end_pass(false, "lapping traffic");
            return mode;
        }
        match self.mode {
            // Keep the door open until the faster car is clearly past.
            Mode::Yield { target, side }
                if find(sit.rivals, target)
                    .is_some_and(|r| r.ahead < ALONGSIDE && r.ahead > -YIELD_RANGE) =>
            {
                Mode::Yield { target, side }
            }
            Mode::Pass { target, side, time } => {
                let time = time + dt;
                let Some(rival) = find(sit.rivals, target) else {
                    return self.end_pass(false, "lost");
                };
                if rival.ahead < -PASS_CLEAR {
                    return self.end_pass(true, "clear");
                }
                let blocked = !side_open(rival, side, sit) && rival.ahead > ALONGSIDE * 0.5;
                if time > PASS_TIMEOUT {
                    return self.end_pass(false, "timeout");
                }
                if time > PASS_MIN_COMMIT && rival.ahead > FOLLOW_RANGE {
                    return self.end_pass(false, "dropped back");
                }
                if blocked && time > PASS_MIN_COMMIT {
                    // Door shut: switch back to the other side if it is open,
                    // otherwise drop back into the wake.
                    if side_open(rival, -side, sit) {
                        self.tally.switchbacks += 1;
                        return Mode::Pass {
                            target,
                            side: -side,
                            time: 0.0,
                        };
                    }
                    return self.end_pass(false, "both sides shut");
                }
                Mode::Pass { target, side, time }
            }
            _ => self.choose(sit),
        }
    }

    fn end_pass(&mut self, completed: bool, why: &str) -> Mode {
        log::debug!("pass ended: {why}");
        if completed {
            self.tally.passes_completed += 1;
            self.cooldown = 0.8;
        } else {
            self.tally.passes_aborted += 1;
            self.cooldown = 1.5;
        }
        Mode::Line
    }

    fn choose(&mut self, sit: &Situation) -> Mode {
        if let Some(mode) = lapping_threat(sit) {
            return mode;
        }
        // Slower car ahead in my lane: attack or tuck.
        if let Some(rival) = sit
            .rivals
            .iter()
            .filter(|r| {
                r.ahead > 0.0
                    && r.ahead < FOLLOW_RANGE
                    && r.closing > -ATTACK_CLOSING
                    && (r.lateral - sit.lateral).abs() < SIDE_GAP
            })
            .min_by(|a, b| a.ahead.total_cmp(&b.ahead))
        {
            if self.cooldown <= 0.0
                && let Some(side) = choose_pass_side(rival, sit)
            {
                self.tally.passes_committed += 1;
                return Mode::Pass {
                    target: rival.id,
                    side,
                    time: 0.0,
                };
            }
            return Mode::Tuck { target: rival.id };
        }
        // Follower lining up a move: cover it.
        if let Some(rival) = sit
            .rivals
            .iter()
            .filter(|r| {
                let defending = matches!(self.mode, Mode::Defend { target, .. } if target == r.id);
                // Move early or not at all: a car already in the gearbox gets
                // the line it chose, so the block never becomes a brake test.
                let near = if defending {
                    -DEFEND_HOLD
                } else {
                    -DEFEND_START
                };
                r.ahead < near
                    && r.ahead > -DEFEND_RANGE
                    && -r.closing > -ATTACK_CLOSING
                    && -r.closing < LAPPING_CLOSING
                    && self.lapped_by.is_none_or(|(id, _)| id != r.id)
            })
            .max_by(|a, b| a.ahead.total_cmp(&b.ahead))
        {
            if let Mode::Defend { target, cover } = self.mode
                && target == rival.id
            {
                return Mode::Defend { target, cover };
            }
            self.tally.defends += 1;
            let line = racing_line(sit.turn, sit.half_room);
            return Mode::Defend {
                target: rival.id,
                cover: defend_lateral(line, rival.lateral, sit.half_room),
            };
        }
        Mode::Line
    }
}

/// A much faster car arriving from behind within the warning time.
fn lapping_threat(sit: &Situation) -> Option<Mode> {
    sit.rivals
        .iter()
        .filter(|r| {
            r.ahead < 0.0
                && r.ahead > -YIELD_RANGE
                && -r.closing >= LAPPING_CLOSING
                && -r.ahead < -r.closing * YIELD_WARNING
        })
        .max_by(|a, b| a.ahead.total_cmp(&b.ahead))
        .map(|rival| Mode::Yield {
            target: rival.id,
            side: yield_side(sit.lateral, rival.lateral),
        })
}

fn find(rivals: &[Rival], id: usize) -> Option<&Rival> {
    rivals.iter().find(|r| r.id == id)
}

/// Inside of the upcoming bend, scaled by how hard it turns.
pub fn racing_line(turn: f32, half_room: f32) -> f32 {
    let span = (half_room * LINE_SPAN).max(0.0);
    (turn * LINE_GAIN).clamp(-span, span)
}

/// Lateral the passer aims for beside `rival` on `side`, if it fits.
fn pass_slot(rival: &Rival, side: f32, half_room: f32) -> Option<f32> {
    let slot = rival.lateral + side * SIDE_GAP;
    (slot.abs() <= half_room).then_some(slot)
}

/// Is the slot beside `rival` on `side` free of the ribbon edge and other cars?
pub fn side_open(rival: &Rival, side: f32, sit: &Situation) -> bool {
    let Some(slot) = pass_slot(rival, side, sit.half_room) else {
        return false;
    };
    !sit.rivals.iter().any(|other| {
        other.id != rival.id
            && other.ahead > -ALONGSIDE
            && other.ahead < rival.ahead + ALONGSIDE
            && (other.lateral - slot).abs() < SIDE_GAP * 0.8
    })
}

/// Pick the side with more room; the inside of the next bend wins ties.
pub fn choose_pass_side(rival: &Rival, sit: &Situation) -> Option<f32> {
    let inside = if sit.turn.abs() > 0.02 {
        sit.turn.signum()
    } else {
        // Straight: go round the side I am already leaning to.
        if sit.lateral >= rival.lateral {
            1.0
        } else {
            -1.0
        }
    };
    let score = |side: f32| {
        side_open(rival, side, sit).then(|| {
            let room = sit.half_room - (rival.lateral + side * SIDE_GAP) * side;
            room + if side == inside { 0.8 } else { 0.0 }
        })
    };
    match (score(1.0), score(-1.0)) {
        (Some(a), Some(b)) => Some(if a >= b { 1.0 } else { -1.0 }),
        (Some(_), None) => Some(1.0),
        (None, Some(_)) => Some(-1.0),
        (None, None) => None,
    }
}

/// Shift toward the follower's lane, at most [`DEFEND_REACH`] off the line.
pub fn defend_lateral(line: f32, follower: f32, half_room: f32) -> f32 {
    (line + (follower - line).clamp(-DEFEND_REACH, DEFEND_REACH)).clamp(-half_room, half_room)
}

/// Move to the side away from a faster car closing from behind.
/// Side (+1 / -1) to move to for a faster car: away from it, or away from
/// the centre it is using when we are already in its lane.
pub fn yield_side(lateral: f32, follower: f32) -> f32 {
    let delta = lateral - follower;
    if delta.abs() > 0.5 {
        delta.signum()
    } else if follower > 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// Run near the edge on `side`, at least a generous gap off the follower.
pub fn yield_lateral(side: f32, follower: f32, half_room: f32) -> f32 {
    let edge = side * half_room * YIELD_EDGE;
    let gap = follower + side * SIDE_GAP * 1.2;
    let target = if side > 0.0 {
        edge.max(gap)
    } else {
        edge.min(gap)
    };
    target.clamp(-half_room, half_room)
}

/// Pull a lateral target onto the pad footprint while the pad is in reach.
pub fn pad_target(target: f32, pad: PadAhead) -> f32 {
    if pad.distance < 0.0 || pad.distance > PAD_LOOKAHEAD {
        return target;
    }
    let reach = (pad.half_width - PAD_MARGIN).max(0.0);
    target.clamp(pad.lateral - reach, pad.lateral + reach)
}

/// Never steer into a car that is alongside: hold at least [`SIDE_GAP`] off it.
pub fn keep_clear(target: f32, lateral: f32, rivals: &[Rival], half_room: f32) -> f32 {
    let mut target = target;
    for rival in rivals.iter().filter(|r| r.ahead.abs() < ALONGSIDE) {
        let side = if lateral >= rival.lateral { 1.0 } else { -1.0 };
        let limit = rival.lateral + side * SIDE_GAP;
        if side > 0.0 {
            target = target.max(limit);
        } else {
            target = target.min(limit);
        }
    }
    target.clamp(-half_room, half_room)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: f32 = 5.5;

    fn rival(id: usize, ahead: f32, lateral: f32, closing: f32) -> Rival {
        Rival {
            id,
            ahead,
            lateral,
            closing,
        }
    }

    fn sit<'a>(lateral: f32, turn: f32, rivals: &'a [Rival]) -> Situation<'a> {
        Situation {
            lateral,
            half_room: ROOM,
            turn,
            pad: None,
            rivals,
        }
    }

    #[test]
    fn racing_line_takes_the_inside_and_stays_on_the_ribbon() {
        assert_eq!(racing_line(0.0, ROOM), 0.0);
        assert!(racing_line(0.2, ROOM) > 0.0);
        assert!(racing_line(-0.2, ROOM) < 0.0);
        assert!(racing_line(10.0, ROOM) <= ROOM * LINE_SPAN);
        assert!(racing_line(-10.0, ROOM) >= -ROOM * LINE_SPAN);
    }

    #[test]
    fn pass_side_prefers_room_then_inside() {
        // Rival hugging the + edge: only the - side fits.
        let r = rival(1, 6.0, 4.0, 1.0);
        let rivals = [r];
        assert_eq!(choose_pass_side(&r, &sit(3.5, 0.3, &rivals)), Some(-1.0));
        // Centred rival, bend to +: take the inside.
        let r = rival(1, 6.0, 0.0, 1.0);
        let rivals = [r];
        assert_eq!(choose_pass_side(&r, &sit(0.0, 0.3, &rivals)), Some(1.0));
        assert_eq!(choose_pass_side(&r, &sit(0.0, -0.3, &rivals)), Some(-1.0));
        // A third car filling the + slot forces the - side.
        let rivals = [r, rival(2, 5.0, 2.7, 0.0)];
        assert_eq!(choose_pass_side(&r, &sit(0.0, 0.3, &rivals)), Some(-1.0));
        // Both slots filled: no pass.
        let rivals = [r, rival(2, 5.0, 2.7, 0.0), rival(3, 6.0, -2.7, 0.0)];
        assert_eq!(choose_pass_side(&r, &sit(0.0, 0.3, &rivals)), None);
    }

    #[test]
    fn commits_to_a_pass_then_rejoins_the_line() {
        let mut planner = Planner::new(0.0);
        let rivals = [rival(1, 8.0, 0.0, 1.5)];
        let plan = planner.plan(&sit(0.0, 0.2, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Pass { target: 1, side, .. } if side > 0.0));
        assert_eq!(plan.pace, ATTACK_PACE);
        assert_eq!(planner.tally.passes_committed, 1);
        // Lateral eases toward the slot, never jumps.
        assert!(plan.lateral > 0.0 && plan.lateral < 0.1);
        // Alongside, then clear: pass completes and the AI goes back to the line.
        let mut lateral = plan.lateral;
        for ahead in [4.0, 1.0, -2.0] {
            let rivals = [rival(1, ahead, 0.0, 1.5)];
            lateral = planner
                .plan(&sit(lateral.max(2.7), 0.2, &rivals), 0.2)
                .lateral;
            assert!(matches!(planner.mode, Mode::Pass { .. }));
        }
        let rivals = [rival(1, -4.0, 0.0, 1.5)];
        planner.plan(&sit(2.7, 0.2, &rivals), 0.01);
        assert_eq!(planner.mode, Mode::Line);
        assert_eq!(planner.tally.passes_completed, 1);
    }

    #[test]
    fn switches_back_then_aborts_when_both_doors_shut() {
        let mut planner = Planner::new(0.0);
        let rivals = [rival(1, 8.0, 0.0, 1.5)];
        planner.plan(&sit(0.0, 0.2, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Pass { side, .. } if side > 0.0));
        // Defender moves to the + edge before I get alongside. Inside the
        // minimum commitment the AI holds its choice…
        let shut = [rival(1, 7.0, 4.5, 1.5)];
        planner.plan(&sit(0.5, 0.2, &shut), 0.1);
        assert!(matches!(planner.mode, Mode::Pass { .. }));
        // …then switches back to the open side without a new commitment.
        planner.plan(&sit(0.5, 0.2, &shut), PASS_MIN_COMMIT);
        assert!(matches!(planner.mode, Mode::Pass { side, .. } if side < 0.0));
        assert_eq!(planner.tally.switchbacks, 1);
        assert_eq!(planner.tally.passes_committed, 1);
        // Both doors shut (a third car on the - side): abort, back into the
        // wake, and wait out the cooldown.
        let shut = [rival(1, 7.0, 4.5, 1.5), rival(2, 7.0, 1.8, 0.0)];
        planner.plan(&sit(0.5, 0.2, &shut), PASS_MIN_COMMIT + 0.1);
        assert_eq!(planner.mode, Mode::Line);
        assert_eq!(planner.tally.passes_aborted, 1);
        let rivals = [rival(1, 6.0, 4.5, 1.5)];
        planner.plan(&sit(4.0, 0.2, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Tuck { target: 1 }));
    }

    #[test]
    fn lapping_traffic_aborts_a_pass() {
        let mut planner = Planner::new(0.0);
        let rivals = [rival(1, 8.0, 0.0, 1.5)];
        planner.plan(&sit(0.0, 0.2, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Pass { .. }));
        let rivals = [rival(1, 8.0, 0.0, 1.5), rival(0, -15.0, 0.5, -9.0)];
        planner.plan(&sit(0.3, 0.2, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Yield { target: 0, side } if side < 0.0));
        assert_eq!(planner.tally.passes_aborted, 1);
    }

    #[test]
    fn pack_pace_tows_the_back_and_holds_the_front() {
        assert_eq!(pack_pace(0.0), 1.0);
        assert!(pack_pace(10.0) > 1.0 && pack_pace(10.0) < 1.0 + PACK_PUSH);
        assert_eq!(pack_pace(1000.0), 1.0 + PACK_PUSH);
        assert_eq!(pack_pace(-1000.0), 1.0 - PACK_EASE);
    }

    #[test]
    fn pass_times_out() {
        let mut planner = Planner::new(0.0);
        let rivals = [rival(1, 8.0, 0.0, 0.3)];
        planner.plan(&sit(0.0, 0.0, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Pass { .. }));
        for _ in 0..((PASS_TIMEOUT / 0.1) as usize + 2) {
            planner.plan(&sit(2.7, 0.0, &rivals), 0.1);
        }
        assert_eq!(planner.tally.passes_aborted, 1);
    }

    #[test]
    fn defends_against_a_follower_but_yields_to_lapping_traffic() {
        let mut planner = Planner::new(0.0);
        // Follower 6 m back, slightly faster, on my - side: cover it.
        let rivals = [rival(1, -6.0, -3.0, -1.0)];
        let mut plan = planner.plan(&sit(0.0, 0.0, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Defend { target: 1, .. }));
        for _ in 0..200 {
            plan = planner.plan(&sit(plan.lateral, 0.0, &rivals), 0.01);
        }
        assert!(plan.lateral < -1.5 && plan.lateral >= -DEFEND_REACH - 1e-4);
        assert_eq!(planner.tally.defends, 1);
        // A car closing at lapping pace gets room instead.
        let rivals = [rival(1, -8.0, 0.0, -8.0)];
        let mut planner = Planner::new(0.5);
        let mut plan = planner.plan(&sit(0.5, 0.0, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Yield { target: 1, .. }));
        for _ in 0..200 {
            plan = planner.plan(&sit(plan.lateral, 0.0, &rivals), 0.01);
        }
        assert!(plan.lateral >= SIDE_GAP - 1e-4);
        // Even if it slows (closing drops), keep the door open until it is past…
        let rivals = [rival(1, -2.0, 0.0, 0.5)];
        planner.plan(&sit(plan.lateral, 0.0, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Yield { target: 1, .. }));
        // …and once it is ahead and drops back, it is not blocked straight away.
        let rivals = [rival(1, 6.0, 0.0, -0.5)];
        planner.plan(&sit(plan.lateral, 0.0, &rivals), 0.01);
        let rivals = [rival(1, -7.0, 0.0, -1.0)];
        planner.plan(&sit(plan.lateral, 0.0, &rivals), 0.01);
        assert_eq!(planner.mode, Mode::Line);
    }

    #[test]
    fn defence_starts_early_and_releases_a_car_alongside() {
        let mut planner = Planner::new(0.0);
        // Too close to start a block.
        let rivals = [rival(1, -4.0, -2.0, -1.0)];
        planner.plan(&sit(0.0, 0.0, &rivals), 0.01);
        assert_eq!(planner.mode, Mode::Line);
        // Early enough: cover, and hold while it closes…
        let rivals = [rival(1, -8.0, -2.0, -1.0)];
        planner.plan(&sit(0.0, 0.0, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Defend { .. }));
        let rivals = [rival(1, -3.5, -2.0, -1.0)];
        planner.plan(&sit(0.0, 0.0, &rivals), 0.01);
        assert!(matches!(planner.mode, Mode::Defend { .. }));
        // …then let it have its line.
        let rivals = [rival(1, -2.5, -2.0, -1.0)];
        planner.plan(&sit(0.0, 0.0, &rivals), 0.01);
        assert_eq!(planner.mode, Mode::Line);
        assert_eq!(planner.tally.defends, 1);
    }

    #[test]
    fn defend_and_yield_stay_on_the_ribbon() {
        assert_eq!(defend_lateral(0.0, -6.0, ROOM), -DEFEND_REACH);
        assert_eq!(defend_lateral(4.5, 6.0, ROOM), ROOM);
        assert_eq!(yield_lateral(-1.0, -4.0, ROOM), -ROOM);
        assert_eq!(yield_lateral(1.0, 0.0, ROOM), ROOM * YIELD_EDGE);
        assert_eq!(yield_lateral(1.0, 3.0, ROOM), ROOM);
        assert_eq!(yield_side(2.0, 0.0), 1.0);
        assert_eq!(yield_side(0.2, 0.0), 1.0);
        assert_eq!(yield_side(0.2, 0.4), -1.0);
        assert_eq!(yield_side(-3.0, -1.0), -1.0);
    }

    #[test]
    fn pads_pull_the_line_onto_their_footprint() {
        let pad = PadAhead {
            distance: 12.0,
            lateral: 0.0,
            half_width: 3.0,
        };
        assert_eq!(pad_target(4.0, pad), 3.0 - PAD_MARGIN);
        assert_eq!(pad_target(-4.0, pad), -(3.0 - PAD_MARGIN));
        assert_eq!(pad_target(0.2, pad), 0.2);
        let far = PadAhead {
            distance: PAD_LOOKAHEAD + 1.0,
            ..pad
        };
        assert_eq!(pad_target(4.0, far), 4.0);
        // On the line with a pad ahead, the planner drifts onto the pad.
        let mut planner = Planner::new(4.0);
        let mut plan = Plan {
            lateral: 4.0,
            pace: 1.0,
        };
        for _ in 0..300 {
            let s = Situation {
                pad: Some(pad),
                ..sit(plan.lateral, 1.0, &[])
            };
            plan = planner.plan(&s, 0.01);
        }
        assert!((plan.lateral - (3.0 - PAD_MARGIN)).abs() < 1e-3);
    }

    #[test]
    fn attacker_takes_the_pad_before_pulling_out() {
        let pad = PadAhead {
            distance: 5.0,
            lateral: 0.0,
            half_width: 3.0,
        };
        let mut planner = Planner::new(0.0);
        let rivals = [rival(1, 12.0, 0.0, 1.0)];
        let s = Situation {
            pad: Some(pad),
            ..sit(0.0, 0.2, &rivals)
        };
        let mut plan = planner.plan(&s, 0.01);
        assert!(matches!(planner.mode, Mode::Pass { .. }));
        for _ in 0..100 {
            plan = planner.plan(
                &Situation {
                    lateral: plan.lateral,
                    ..s
                },
                0.01,
            );
        }
        assert!(plan.lateral.abs() <= 3.0 - PAD_MARGIN + 1e-4);
        // Close behind: go for the slot even with a pad around.
        let rivals = [rival(1, 5.0, 0.0, 1.0)];
        for _ in 0..100 {
            let s = Situation {
                pad: Some(pad),
                ..sit(plan.lateral, 0.2, &rivals)
            };
            plan = planner.plan(&s, 0.01);
        }
        assert!(plan.lateral > 2.0);
    }

    #[test]
    fn never_steers_into_a_car_alongside() {
        let rivals = [rival(1, 1.0, 1.0, 0.0)];
        // I'm on the - side of it; a + target is held back to its - flank.
        assert_eq!(keep_clear(3.0, -1.0, &rivals, ROOM), 1.0 - SIDE_GAP);
        // Far behind cars do not constrain.
        let rivals = [rival(1, -9.0, 1.0, 0.0)];
        assert_eq!(keep_clear(3.0, -1.0, &rivals, ROOM), 3.0);
    }
}
