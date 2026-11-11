//! Per-race pack statistics for the end-of-race log: how often the order
//! changes, who passes whom, pad usage, contacts and recoveries.
//!
//! Observation only — nothing here feeds back into driving, so scripted races
//! stay deterministic whether or not the stats are read.

use std::collections::HashSet;

/// A racer must lead by this much (m along the ribbon) before a swap counts,
/// so two cars running nose-to-tail do not rack up phantom passes.
pub const SWAP_HYSTERESIS: f32 = 1.5;
/// Off-ribbon excursion: enter beyond this (m outside the edge), leave inside the edge.
const OFF_RIBBON_ENTER: f32 = 0.5;

/// One racer's state for a single physics step.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// Nearest track sample index.
    pub sample: usize,
    pub is_ai: bool,
    /// Signed distance outside the ribbon edge (m); negative is on the ribbon.
    pub off_track: f32,
}

#[derive(Clone, Debug, Default)]
pub struct RaceStats {
    /// Unwrapped distance along the ribbon (m) per racer (0 = player).
    progress: Vec<f32>,
    last_sample: Vec<usize>,
    off_ribbon: Vec<bool>,
    /// For each pair (a < b): +1 when a leads, -1 when b leads.
    leader: Vec<i8>,
    pair_contact: HashSet<(usize, usize)>,
    pub position_changes: u32,
    pub ai_overtakes: u32,
    pub contacts: u32,
    pub ai_off_ribbon: u32,
    pub player_off_ribbon: u32,
    pub ai_recoveries: u32,
    pub player_recoveries: u32,
}

impl RaceStats {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Advance with this step's field. `spacing` is metres per track sample.
    pub fn observe(&mut self, entries: &[Entry], track_len: usize, spacing: f32) {
        let n = entries.len();
        if self.progress.len() != n {
            self.progress = entries.iter().map(|e| e.sample as f32 * spacing).collect();
            // The grid sits behind the start gate: samples past the halfway mark
            // are "behind" sample 0, not most of a lap ahead.
            for (p, e) in self.progress.iter_mut().zip(entries) {
                if e.sample > track_len / 2 {
                    *p -= track_len as f32 * spacing;
                }
            }
            self.last_sample = entries.iter().map(|e| e.sample).collect();
            self.off_ribbon = vec![false; n];
            self.leader = Vec::with_capacity(n * n / 2);
            for a in 0..n {
                for b in a + 1..n {
                    self.leader.push(if self.progress[a] >= self.progress[b] {
                        1
                    } else {
                        -1
                    });
                }
            }
            return;
        }
        for (i, e) in entries.iter().enumerate() {
            let delta = wrap_delta(e.sample, self.last_sample[i], track_len);
            self.progress[i] += delta as f32 * spacing;
            self.last_sample[i] = e.sample;
            let off = if self.off_ribbon[i] {
                e.off_track > 0.0
            } else {
                e.off_track > OFF_RIBBON_ENTER
            };
            if off && !self.off_ribbon[i] {
                if e.is_ai {
                    self.ai_off_ribbon += 1;
                } else {
                    self.player_off_ribbon += 1;
                }
            }
            self.off_ribbon[i] = off;
        }
        let mut k = 0;
        for a in 0..n {
            for b in a + 1..n {
                let gap = self.progress[a] - self.progress[b];
                let now = if gap > SWAP_HYSTERESIS {
                    1
                } else if gap < -SWAP_HYSTERESIS {
                    -1
                } else {
                    self.leader[k]
                };
                if now != self.leader[k] {
                    self.position_changes += 1;
                    let gainer = if now > 0 { a } else { b };
                    if entries[gainer].is_ai {
                        self.ai_overtakes += 1;
                    }
                    self.leader[k] = now;
                }
                k += 1;
            }
        }
    }

    /// Record the set of racer pairs touching this step; counts new contacts only.
    pub fn observe_contacts(&mut self, pairs: &HashSet<(usize, usize)>) {
        for pair in pairs.difference(&self.pair_contact) {
            log::debug!("contact between racers {} and {}", pair.0, pair.1);
            self.contacts += 1;
        }
        self.pair_contact.clone_from(pairs);
    }

    pub fn record_recovery(&mut self, is_ai: bool) {
        if is_ai {
            self.ai_recoveries += 1;
        } else {
            self.player_recoveries += 1;
        }
    }

    /// Gap (m) between the furthest and least advanced racer.
    pub fn spread(&self) -> f32 {
        let max = self.progress.iter().copied().fold(f32::MIN, f32::max);
        let min = self.progress.iter().copied().fold(f32::MAX, f32::min);
        if self.progress.is_empty() {
            0.0
        } else {
            max - min
        }
    }

    /// Unwrapped race distance (m) per racer (0 = player).
    pub fn distances(&self) -> &[f32] {
        &self.progress
    }

    /// Racer ids from leader to last.
    pub fn order(&self) -> Vec<usize> {
        let mut ids: Vec<usize> = (0..self.progress.len()).collect();
        ids.sort_by(|&a, &b| {
            self.progress[b]
                .partial_cmp(&self.progress[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ids
    }
}

/// Unwrapped distance along the ribbon for one craft.
#[derive(Clone, Copy, Debug, Default)]
pub struct Odometer {
    last: Option<usize>,
    /// Metres from the start line (negative on the grid behind it).
    pub distance: f32,
}

impl Odometer {
    pub fn update(&mut self, sample: usize, track_len: usize, spacing: f32) -> f32 {
        self.distance = match self.last {
            Some(last) => self.distance + wrap_delta(sample, last, track_len) as f32 * spacing,
            None if sample > track_len / 2 => (sample as f32 - track_len as f32) * spacing,
            None => sample as f32 * spacing,
        };
        self.last = Some(sample);
        self.distance
    }
}

/// Signed sample delta `now - before` on a closed loop of `len` samples.
pub fn wrap_delta(now: usize, before: usize, len: usize) -> i64 {
    let len = len.max(1) as i64;
    let mut d = (now as i64 - before as i64) % len;
    if d > len / 2 {
        d -= len;
    } else if d < -len / 2 {
        d += len;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(sample: usize, is_ai: bool) -> Entry {
        Entry {
            sample,
            is_ai,
            off_track: -1.0,
        }
    }

    #[test]
    fn wrap_delta_crosses_the_line() {
        assert_eq!(wrap_delta(2, 98, 100), 4);
        assert_eq!(wrap_delta(98, 2, 100), -4);
        assert_eq!(wrap_delta(10, 7, 100), 3);
    }

    #[test]
    fn swap_needs_hysteresis_and_credits_the_ai() {
        let mut stats = RaceStats::default();
        // Player at 10, AI at 12 (1 m spacing): AI leads.
        stats.observe(&[e(10, false), e(12, true)], 100, 1.0);
        // Player draws level: no swap inside the hysteresis band.
        stats.observe(&[e(13, false), e(12, true)], 100, 1.0);
        assert_eq!(stats.position_changes, 0);
        // Player clearly ahead: one change, not credited to the AI.
        stats.observe(&[e(15, false), e(13, true)], 100, 1.0);
        assert_eq!((stats.position_changes, stats.ai_overtakes), (1, 0));
        // AI re-passes, then both cross the start/finish wrap.
        stats.observe(&[e(40, false), e(45, true)], 100, 1.0);
        assert_eq!((stats.position_changes, stats.ai_overtakes), (2, 1));
        stats.observe(&[e(70, false), e(80, true)], 100, 1.0);
        stats.observe(&[e(95, false), e(3, true)], 100, 1.0);
        assert_eq!(stats.position_changes, 2);
        assert_eq!(stats.order(), vec![1, 0]);
        assert!((stats.spread() - 8.0).abs() < 1e-4);
    }

    #[test]
    fn odometer_unwraps_laps() {
        let mut odo = Odometer::default();
        assert_eq!(odo.update(98, 100, 2.0), -4.0);
        assert_eq!(odo.update(3, 100, 2.0), 6.0);
        assert_eq!(odo.update(45, 100, 2.0), 90.0);
        assert_eq!(odo.update(90, 100, 2.0), 180.0);
        assert_eq!(odo.update(99, 100, 2.0), 198.0);
        assert_eq!(odo.update(1, 100, 2.0), 202.0);
    }

    #[test]
    fn grid_behind_the_line_is_not_a_lap_ahead() {
        let mut stats = RaceStats::default();
        stats.observe(&[e(0, false), e(95, true)], 100, 1.0);
        assert_eq!(stats.order(), vec![0, 1]);
    }

    #[test]
    fn contacts_and_off_ribbon_count_entries_only() {
        let mut stats = RaceStats::default();
        let mut pairs = HashSet::new();
        pairs.insert((0, 1));
        stats.observe_contacts(&pairs);
        stats.observe_contacts(&pairs);
        assert_eq!(stats.contacts, 1);
        stats.observe_contacts(&HashSet::new());
        stats.observe_contacts(&pairs);
        assert_eq!(stats.contacts, 2);

        let mut off = e(5, true);
        stats.observe(&[e(5, false), off], 100, 1.0);
        off.off_track = 0.8;
        stats.observe(&[e(5, false), off], 100, 1.0);
        off.off_track = 0.2; // still outside the edge: same excursion
        stats.observe(&[e(5, false), off], 100, 1.0);
        assert_eq!(stats.ai_off_ribbon, 1);
        off.off_track = -0.5;
        stats.observe(&[e(5, false), off], 100, 1.0);
        off.off_track = 0.9;
        stats.observe(&[e(5, false), off], 100, 1.0);
        assert_eq!((stats.ai_off_ribbon, stats.player_off_ribbon), (2, 0));
        stats.record_recovery(true);
        stats.record_recovery(false);
        assert_eq!((stats.ai_recoveries, stats.player_recoveries), (1, 1));
    }
}
