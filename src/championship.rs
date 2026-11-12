//! Short Mars championship: three races on the flagship circuit, points by
//! finishing order, standings that persist between sessions through the same
//! `ghost::Store` (a `.ron` file next to the ghosts, or localStorage).

use serde::{Deserialize, Serialize};

use crate::ghost::Store;

/// Races in one championship.
pub const RACES: u32 = 3;
/// Points for P1..P4; a fifth place or lower scores nothing.
pub const POINTS: [u32; 4] = [10, 6, 4, 3];
/// Bump when the saved layout changes; older saves are ignored.
pub const FORMAT: u32 = 1;
/// Largest field we accept from a save (player + AI).
const MAX_RACERS: usize = 8;

pub fn points_for(place: usize) -> u32 {
    place
        .checked_sub(1)
        .and_then(|i| POINTS.get(i))
        .copied()
        .unwrap_or(0)
}

/// Storage key: `championship-<circuit>` (`.ron` on disk, `redline:` prefix on the web).
pub fn key(circuit: &str) -> String {
    format!("championship-{circuit}")
}

/// Stable display name for a racer id: the AI ids follow the fixed grid order.
pub fn name(racer: usize) -> String {
    if racer == 0 {
        "You".to_string()
    } else {
        format!("AI {racer}")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Racer id: 0 is the player, 1.. the AI in grid order.
    pub racer: usize,
    pub points: u32,
    /// Best finishing place so far (1 = a win); 0 before the first race.
    pub best_finish: usize,
    /// Fastest lap across the championship, tiebreak after best finish.
    pub best_lap: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standings {
    pub format: u32,
    pub circuit: String,
    /// Races already scored (0..=RACES).
    pub races: u32,
    pub entries: Vec<Entry>,
}

impl Standings {
    pub fn new(circuit: &str) -> Self {
        Self {
            format: FORMAT,
            circuit: circuit.to_string(),
            races: 0,
            entries: Vec::new(),
        }
    }

    pub fn complete(&self) -> bool {
        self.races >= RACES
    }

    /// Score one race. `order` is the classification, winner first, each
    /// racer with its best lap of that race. Returns this race's points.
    pub fn record(&mut self, order: &[(usize, Option<f32>)]) -> Vec<(usize, u32)> {
        if self.complete() {
            return Vec::new();
        }
        self.races += 1;
        let mut awarded = Vec::new();
        for (i, &(racer, lap)) in order.iter().enumerate() {
            let entry = match self.entries.iter().position(|e| e.racer == racer) {
                Some(index) => &mut self.entries[index],
                None => {
                    self.entries.push(Entry {
                        racer,
                        points: 0,
                        best_finish: 0,
                        best_lap: None,
                    });
                    self.entries.last_mut().unwrap()
                }
            };
            let points = points_for(i + 1);
            entry.points += points;
            if entry.best_finish == 0 || i + 1 < entry.best_finish {
                entry.best_finish = i + 1;
            }
            if let Some(lap) = lap.filter(|l| l.is_finite() && *l > 0.0) {
                entry.best_lap = Some(entry.best_lap.map_or(lap, |b| b.min(lap)));
            }
            awarded.push((racer, points));
        }
        awarded
    }

    /// Table order: points, then best race finish, then best lap, then id.
    pub fn table(&self) -> Vec<Entry> {
        let mut rows = self.entries.clone();
        rows.sort_by(|a, b| {
            let finish = |e: &Entry| {
                if e.best_finish == 0 {
                    usize::MAX
                } else {
                    e.best_finish
                }
            };
            let lap = |e: &Entry| e.best_lap.unwrap_or(f32::INFINITY);
            b.points
                .cmp(&a.points)
                .then(finish(a).cmp(&finish(b)))
                .then(lap(a).total_cmp(&lap(b)))
                .then(a.racer.cmp(&b.racer))
        });
        rows
    }

    pub fn champion(&self) -> Option<usize> {
        if !self.complete() {
            return None;
        }
        self.table().first().map(|e| e.racer)
    }
}

/// Parse saved standings; another circuit, an old format, corrupt or
/// impossible data is skipped with a warning, never a panic.
pub fn decode(text: &str, circuit: &str) -> Option<Standings> {
    let table: Standings = ron::from_str(text)
        .map_err(|err| log::warn!("ignoring championship for {circuit}: {err}"))
        .ok()?;
    let mut ids: Vec<usize> = table.entries.iter().map(|e| e.racer).collect();
    ids.sort_unstable();
    ids.dedup();
    let sane = table.format == FORMAT
        && table.circuit == circuit
        && table.races <= RACES
        && table.entries.len() <= MAX_RACERS
        && ids.len() == table.entries.len()
        && table.entries.iter().all(|e| {
            e.points <= POINTS[0] * table.races
                && e.best_finish <= MAX_RACERS
                && (table.races > 0 || e.points == 0)
                && e.best_lap.is_none_or(|l| l.is_finite() && l > 0.0)
        });
    if !sane {
        log::warn!("ignoring championship for {circuit}: wrong circuit, format or data");
    }
    sane.then_some(table)
}

/// Championship state for the session plus where it persists.
pub struct Session {
    /// None keeps the championship in memory only (smoke, plain scripts).
    store: Option<Box<dyn Store>>,
    pub table: Standings,
    /// Points scored in the race just finished, winner first.
    pub last: Vec<(usize, u32)>,
    /// Storage failures log one warning per session; the rest go to debug.
    warned: bool,
}

impl Session {
    /// Resume `circuit`'s saved championship, or start a fresh one.
    pub fn new(store: Option<Box<dyn Store>>, circuit: &str) -> Self {
        let table = store
            .as_ref()
            .and_then(|s| s.read(&key(circuit)))
            .and_then(|text| decode(&text, circuit))
            .unwrap_or_else(|| Standings::new(circuit));
        Self {
            store,
            table,
            last: Vec::new(),
            warned: false,
        }
    }

    /// 1-based number of the race to run next (or the last one, once complete).
    pub fn race_number(&self) -> u32 {
        (self.table.races + 1).min(RACES)
    }

    pub fn in_progress(&self) -> bool {
        self.table.races > 0 && !self.table.complete()
    }

    fn warn(&mut self, what: &str, err: &str) {
        let place = self
            .store
            .as_ref()
            .map_or_else(String::new, |s| s.describe(&key(&self.table.circuit)));
        if std::mem::replace(&mut self.warned, true) {
            log::debug!("championship {what} {place}: {err}");
        } else {
            log::warn!("championship {what} {place}: {err}; keeping it in memory");
        }
    }

    /// Score a finished race and save; a failed write keeps it in memory.
    pub fn record(&mut self, order: &[(usize, Option<f32>)]) {
        self.last = self.table.record(order);
        if self.last.is_empty() {
            return;
        }
        let k = key(&self.table.circuit);
        let text = ron::to_string(&self.table).map_err(|err| err.to_string());
        if let Some(store) = self.store.as_mut() {
            match text.and_then(|text| store.write(&k, &text)) {
                Ok(()) => log::info!("championship saved to {}", store.describe(&k)),
                Err(err) => self.warn("not saved to", &err),
            }
        }
    }

    /// A finished championship makes way for a new one (zero points).
    pub fn start(&mut self) {
        if self.table.complete() {
            self.table = Standings::new(&self.table.circuit);
            self.last.clear();
        }
    }

    /// Menu "Reset": forget the saved file / localStorage entry and the
    /// in-memory standings.
    pub fn reset(&mut self) {
        let k = key(&self.table.circuit);
        if let Some(Err(err)) = self.store.as_mut().map(|s| s.remove(&k)) {
            self.warn("not removed from", &err);
        }
        self.table = Standings::new(&self.table.circuit);
        self.last.clear();
        log::info!("championship reset for {}", self.table.circuit);
    }

    pub fn summary(&self) -> String {
        let rows: Vec<String> = self
            .table
            .table()
            .iter()
            .map(|e| format!("{}={}", name(e.racer), e.points))
            .collect();
        format!(
            "Championship: races={}/{} standings=[{}] last={:?} champion={} file={}",
            self.table.races,
            RACES,
            rows.join(", "),
            self.last,
            self.table.champion().map_or("-".to_string(), name),
            self.store
                .as_ref()
                .map_or("-".to_string(), |s| s.describe(&key(&self.table.circuit))),
        )
    }
}

/// `?championship=1` opts a scripted web run into championship mode and
/// localStorage, like `--championship` on native.
#[cfg(any(target_arch = "wasm32", test))]
pub fn web_opt_in(search: &str) -> bool {
    search
        .trim_start_matches('?')
        .split('&')
        .any(|pair| matches!(pair, "championship" | "championship=1"))
}

/// Persist for play; scripts only with the explicit flag. Never in smoke.
pub fn persists(scripted: bool, smoke: bool, explicit: bool) -> bool {
    !smoke && (!scripted || explicit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    const MARS: &str = "mars-equator";

    /// Shared map standing in for the data dir / localStorage.
    #[derive(Clone, Default)]
    struct FakeStore {
        items: Rc<RefCell<HashMap<String, String>>>,
        throws: bool,
    }

    impl Store for FakeStore {
        fn read(&self, key: &str) -> Option<String> {
            self.items.borrow().get(key).cloned()
        }
        fn write(&mut self, key: &str, text: &str) -> Result<(), String> {
            if self.throws {
                return Err("QuotaExceededError".to_string());
            }
            self.items.borrow_mut().insert(key.into(), text.into());
            Ok(())
        }
        fn remove(&mut self, key: &str) -> Result<(), String> {
            if self.throws {
                return Err("SecurityError".to_string());
            }
            self.items.borrow_mut().remove(key);
            Ok(())
        }
        fn describe(&self, key: &str) -> String {
            format!("fake[{key}]")
        }
    }

    fn open(store: &FakeStore) -> Session {
        Session::new(Some(Box::new(store.clone())), MARS)
    }

    fn race(order: &[usize]) -> Vec<(usize, Option<f32>)> {
        order.iter().map(|&r| (r, Some(20.0 + r as f32))).collect()
    }

    fn points(table: &Standings) -> Vec<(usize, u32)> {
        table.table().iter().map(|e| (e.racer, e.points)).collect()
    }

    #[test]
    fn points_follow_the_finishing_order() {
        assert_eq!(
            (1..=5).map(points_for).collect::<Vec<_>>(),
            [10, 6, 4, 3, 0]
        );
        assert_eq!(points_for(0), 0);
        let mut table = Standings::new(MARS);
        assert_eq!(
            table.record(&race(&[2, 0, 3, 1])),
            [(2, 10), (0, 6), (3, 4), (1, 3)]
        );
        // A smaller field (web: two AI) scores whoever finished.
        let mut web = Standings::new(MARS);
        assert_eq!(web.record(&race(&[0, 2, 1])), [(0, 10), (2, 6), (1, 4)]);
    }

    #[test]
    fn three_races_total_and_crown_a_champion() {
        let mut table = Standings::new(MARS);
        table.record(&race(&[0, 1, 2, 3]));
        assert!(table.champion().is_none());
        table.record(&race(&[1, 0, 2, 3]));
        table.record(&race(&[0, 2, 1, 3]));
        assert!(table.complete());
        assert_eq!(points(&table), [(0, 26), (1, 20), (2, 14), (3, 9)]);
        assert_eq!(table.champion(), Some(0));
        // A fourth race is not scored.
        assert!(table.record(&race(&[3, 2, 1, 0])).is_empty());
        assert_eq!(table.races, RACES);
    }

    #[test]
    fn ties_break_on_best_finish_then_best_lap() {
        // A: 10+3+3 = 16 with a win; B: 6+6+4 = 16, best P2.
        let (a, b, c, d) = (1, 2, 3, 0);
        let mut table = Standings::new(MARS);
        table.record(&race(&[a, b, c, d]));
        table.record(&race(&[c, b, d, a]));
        table.record(&race(&[c, d, b, a]));
        assert_eq!(points(&table), [(c, 24), (a, 16), (b, 16), (d, 13)]);
        assert_eq!(table.champion(), Some(c));
        // Equal points and best finish: the faster best lap ranks first.
        let mut even = Standings::new(MARS);
        even.record(&[(1, Some(21.0)), (0, Some(20.0))]);
        even.record(&[(0, Some(20.5)), (1, Some(22.0))]);
        assert_eq!(points(&even), [(0, 16), (1, 16)]);
        assert_eq!(even.table()[0].best_lap, Some(20.0));
        // No laps at all: racer id keeps the order stable.
        let mut bare = Standings::new(MARS);
        bare.record(&[(2, None), (1, None)]);
        bare.record(&[(1, None), (2, None)]);
        assert_eq!(points(&bare), [(1, 16), (2, 16)]);
    }

    #[test]
    fn standings_persist_and_resume_mid_championship() {
        let store = FakeStore::default();
        let mut tab = open(&store);
        assert_eq!((tab.race_number(), tab.in_progress()), (1, false));
        tab.record(&race(&[1, 0, 2, 3]));
        assert!(
            store
                .items
                .borrow()
                .contains_key("championship-mars-equator")
        );
        // "Quit and reopen": a new session over the same store resumes race 2.
        let mut again = open(&store);
        assert_eq!((again.race_number(), again.in_progress()), (2, true));
        assert_eq!(again.table, tab.table);
        again.start();
        assert_eq!(again.table.races, 1, "start resumes, never zeroes mid-way");
        again.record(&race(&[0, 1, 2, 3]));
        again.record(&race(&[0, 2, 1, 3]));
        let done = open(&store);
        assert_eq!(done.table.champion(), Some(0));
        assert_eq!(done.race_number(), RACES);
        // Starting after completion zeroes the points for a fresh championship.
        let mut fresh = open(&store);
        fresh.start();
        assert_eq!((fresh.table.races, fresh.table.entries.len()), (0, 0));
        fresh.record(&race(&[3, 2, 1, 0]));
        assert_eq!(open(&store).table.races, 1);
    }

    #[test]
    fn bad_payloads_are_skipped() {
        let store = FakeStore::default();
        open(&store).record(&race(&[0, 1, 2, 3]));
        let good = store.items.borrow()["championship-mars-equator"].clone();
        let load = |text: &str| {
            store.items.borrow_mut().insert(key(MARS), text.to_string());
            open(&store).table
        };
        assert_eq!(load(&good).races, 1);
        let fresh = Standings::new(MARS);
        for bad in [
            good.replace("\"mars-equator\"", "\"phobos-needle\""),
            good.replace("format:1", "format:0"),
            good.replace("races:1", "races:7"),
            good.replace("points:10", "points:99"),
            good.replace("best_lap:Some(20.0)", "best_lap:Some(NaN)"),
            good.replace("best_lap:Some(20.0)", "best_lap:Some(-1.0)"),
            good.replace("racer:1", "racer:0"),
            good[..good.len() / 2].to_string(),
            String::new(),
            "{\"not\":\"ron\"}".to_string(),
        ] {
            assert_ne!(bad, good);
            assert_eq!(load(&bad), fresh, "{bad}");
        }
    }

    #[test]
    fn reset_clears_storage_and_memory() {
        let store = FakeStore::default();
        let mut tab = open(&store);
        tab.record(&race(&[0, 1]));
        tab.reset();
        assert!(store.items.borrow().is_empty());
        assert_eq!((tab.table.races, tab.last.len()), (0, 0));
        assert!(tab.table.entries.is_empty());
        assert_eq!(open(&store).table.races, 0);
        // Resetting with nothing saved is fine.
        tab.reset();
    }

    #[test]
    fn storage_failures_keep_the_session_standings() {
        let store = FakeStore {
            throws: true,
            ..FakeStore::default()
        };
        let mut tab = open(&store);
        tab.record(&race(&[0, 1]));
        tab.record(&race(&[1, 0]));
        assert_eq!(tab.table.races, 2);
        assert!(tab.warned);
        tab.reset();
        assert_eq!(tab.table.races, 0);
        // No store at all (plain scripts): memory only.
        let mut memory = Session::new(None, MARS);
        memory.record(&race(&[0]));
        assert_eq!(memory.table.races, 1);
    }

    #[test]
    fn scripts_and_smoke_skip_persist_unless_opted_in() {
        assert!(persists(false, false, false), "play persists");
        assert!(!persists(true, false, false), "scripted race");
        assert!(persists(true, false, true), "--championship script");
        assert!(!persists(false, true, false), "smoke");
        assert!(!persists(false, true, true), "smoke never");
        assert!(web_opt_in("?script=race&championship=1"));
        assert!(web_opt_in("championship"));
        assert!(!web_opt_in("?championship=0"));
        assert!(!web_opt_in("?ghost=1"));
        assert_eq!(name(0), "You");
        assert_eq!(name(2), "AI 2");
    }
}
