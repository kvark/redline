//! Live time gaps between racers: each racer leaves a timestamp at every
//! metre of race distance, so the gap to a rival ahead is how long ago they
//! passed the point you are at now.

/// Distance between timing marks (m).
pub const MARK_SPACING: f32 = 1.0;

#[derive(Clone, Debug, Default)]
pub struct GapTimer {
    origin: f32,
    clock: f32,
    /// `marks[racer][k]`: clock when the racer first reached mark `k`.
    marks: Vec<Vec<f32>>,
    distances: Vec<f32>,
}

/// The player's split against the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Split {
    /// Seconds behind the leader.
    Behind(f32),
    /// Leading, this many seconds clear of P2.
    Leading(f32),
}

impl Split {
    /// "+0.8s" behind the leader, "-1.2s" clear of P2.
    pub fn text(self) -> String {
        match self {
            Self::Behind(gap) => format!("+{gap:.1}s"),
            Self::Leading(gap) => format!("-{gap:.1}s"),
        }
    }

    pub fn reference(self) -> &'static str {
        match self {
            Self::Behind(_) => "to leader",
            Self::Leading(_) => "over P2",
        }
    }
}

impl GapTimer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Advance the clock by `dt` and stamp each racer's race distance (m).
    pub fn observe(&mut self, distances: &[f32], dt: f32) {
        if self.marks.len() != distances.len() {
            self.origin = distances.iter().copied().fold(f32::MAX, f32::min).floor();
            self.marks = vec![Vec::new(); distances.len()];
            self.clock = 0.0;
        } else {
            self.clock += dt;
        }
        for (marks, &d) in self.marks.iter_mut().zip(distances) {
            let mark = ((d - self.origin) / MARK_SPACING).max(0.0) as usize;
            while marks.len() <= mark {
                marks.push(self.clock);
            }
        }
        self.distances = distances.to_vec();
    }

    /// Seconds since `ahead` passed the mark `behind` has reached, if it has.
    pub fn gap(&self, ahead: usize, behind: usize) -> Option<f32> {
        let mark = self.marks.get(behind)?.len().checked_sub(1)?;
        let stamp = *self.marks.get(ahead)?.get(mark)?;
        Some(self.clock - stamp)
    }

    /// Racer ids from leader to last by race distance.
    pub fn order(&self) -> Vec<usize> {
        let mut ids: Vec<usize> = (0..self.distances.len()).collect();
        ids.sort_by(|&a, &b| self.distances[b].total_cmp(&self.distances[a]));
        ids
    }

    /// `racer`'s split: behind the leader, or clear of P2 when leading.
    pub fn split(&self, racer: usize) -> Option<Split> {
        let order = self.order();
        let leader = *order.first()?;
        if leader == racer {
            self.gap(racer, *order.get(1)?).map(Split::Leading)
        } else {
            self.gap(leader, racer).map(Split::Behind)
        }
    }

    /// Each racer from leader to last, with its gap to the leader.
    pub fn standings(&self) -> Vec<(usize, f32)> {
        let order = self.order();
        let Some(&leader) = order.first() else {
            return Vec::new();
        };
        order
            .iter()
            .map(|&id| (id, self.gap(leader, id).unwrap_or(0.0)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Racer 0 at 10 m/s, racer 1 at 8 m/s from the same start.
    fn drive(seconds: f32) -> GapTimer {
        let mut timer = GapTimer::default();
        let dt = 0.01;
        let mut t = 0.0;
        timer.observe(&[0.0, 0.0], dt);
        while t < seconds {
            t += dt;
            timer.observe(&[10.0 * t, 8.0 * t], dt);
        }
        timer
    }

    #[test]
    fn gap_is_time_since_leader_passed() {
        let timer = drive(5.0);
        // Racer 1 is at 40 m; racer 0 passed 40 m at t=4.0, one second ago.
        let gap = timer.gap(0, 1).unwrap();
        assert!((gap - 1.0).abs() < 0.03, "gap {gap}");
        // Racer 1 has not reached racer 0's 50 m mark.
        assert_eq!(timer.gap(1, 0), None);
        assert_eq!(timer.order(), vec![0, 1]);
    }

    #[test]
    fn split_flips_to_lead_over_p2() {
        let timer = drive(5.0);
        let Some(Split::Leading(lead)) = timer.split(0) else {
            panic!("racer 0 should lead");
        };
        assert!((lead - 1.0).abs() < 0.03);
        let Some(Split::Behind(behind)) = timer.split(1) else {
            panic!("racer 1 should trail");
        };
        assert!((behind - 1.0).abs() < 0.03);
        assert_eq!(Split::Behind(0.84).text(), "+0.8s");
        assert_eq!(Split::Leading(1.25).text(), "-1.2s");
        let standings = timer.standings();
        assert_eq!(standings[0], (0, 0.0));
        assert_eq!(standings[1].0, 1);
    }

    #[test]
    fn grid_behind_the_line_uses_a_negative_origin() {
        let mut timer = GapTimer::default();
        timer.observe(&[-6.0, -2.0], 0.01);
        timer.observe(&[-1.0, 1.0], 0.5);
        timer.observe(&[1.5, 3.0], 0.5);
        // Racer 0 reached 1 m (racer 1 passed it at clock 0.5) at clock 1.0.
        assert!((timer.gap(1, 0).unwrap() - 0.5).abs() < 1e-5);
        assert_eq!(timer.split(0), Some(Split::Behind(0.5)));
        assert!(GapTimer::default().split(0).is_none());
    }
}
