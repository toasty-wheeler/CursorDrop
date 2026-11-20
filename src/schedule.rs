//! The schedule: which cursors to use on which day.
//!
//! It is a repeating pattern of 1 to 5 weeks (Monday to Sunday). Week 1 starts on `anchor`
//! (a Monday), so with two weeks the pattern goes Week 1, Week 2, Week 1, ... Each day of the
//! pattern has a rule: leave the cursors alone, use one pack variant or preset, pick at
//! random between several, or pick at random from everything. Once a day has started its
//! pick is kept for the rest of that day (also across restarts).

use crate::{Active, App};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MAX_WEEKS: usize = 5;
pub const DAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const DAY_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum DayRule {
    /// Leave the cursors as they are.
    #[default]
    Off,
    /// Always this one (None until one is picked).
    One { pick: Option<Active> },
    /// One of these, at random, each day.
    Several { picks: Vec<Active> },
    /// One of all packs and presets, at random, each day.
    RandomAll,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub enabled: bool,
    /// The Monday Week 1 starts on (days since 1970-01-01).
    pub anchor: i64,
    pub weeks: Vec<[DayRule; 7]>,
    /// The day the schedule last ran and what it picked (kept so a random pick lasts all day).
    #[serde(default)]
    pub today: Option<(i64, Option<Active>)>,
}

impl Default for Schedule {
    fn default() -> Self {
        let t = today();
        Schedule { enabled: false, anchor: t - weekday(t) as i64, weeks: vec![Default::default()], today: None }
    }
}

// ---------------------------------------------------------------- dates

/// Days since 1970-01-01 for a calendar date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The calendar date for days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = Monday ... 6 = Sunday.
pub fn weekday(days: i64) -> usize {
    (days + 3).rem_euclid(7) as usize
}

/// Today in local time, as days since 1970-01-01. CURSORDROP_TODAY=YYYY-MM-DD overrides it
/// (for the self-test).
pub fn today() -> i64 {
    if let Ok(s) = std::env::var("CURSORDROP_TODAY") {
        let p: Vec<i64> = s.split('-').filter_map(|x| x.parse().ok()).collect();
        if p.len() == 3 {
            return days_from_civil(p[0], p[1] as u32, p[2] as u32);
        }
    }
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    days_from_civil(t.wYear as i64, t.wMonth as u32, t.wDay as u32)
}

impl Schedule {
    /// Which week of the pattern a date falls in.
    pub fn week_of(&self, days: i64) -> usize {
        let monday = days - weekday(days) as i64;
        ((monday - self.anchor) / 7).rem_euclid(self.weeks.len().max(1) as i64) as usize
    }

    /// The pattern day (week * 7 + weekday) for a date.
    pub fn slot_of(&self, days: i64) -> usize {
        self.week_of(days) * 7 + weekday(days)
    }

    pub fn rule(&self, slot: usize) -> &DayRule {
        static OFF: DayRule = DayRule::Off;
        self.weeks.get(slot / 7).map_or(&OFF, |w| &w[slot % 7])
    }

    pub fn set_rule(&mut self, slot: usize, rule: DayRule) {
        if let Some(w) = self.weeks.get_mut(slot / 7) {
            w[slot % 7] = rule;
        }
    }
}

// ---------------------------------------------------------------- naming

/// "Weekdays", "Weekends", "Every day", "Tue–Thu", "Monday", "Mon, Wed, Sat"...
pub fn describe_days(days: &[usize]) -> String {
    let mut d: Vec<usize> = days.to_vec();
    d.sort_unstable();
    d.dedup();
    match d.as_slice() {
        [] => return String::new(),
        [0, 1, 2, 3, 4, 5, 6] => return "Every day".into(),
        [0, 1, 2, 3, 4] => return "Weekdays".into(),
        [5, 6] => return "Weekends".into(),
        [one] => return DAY_NAMES[*one].into(),
        _ => {}
    }
    let mut parts = Vec::new();
    let mut rest: &[usize] = &d;
    // Weekdays / Weekends inside a bigger set read better than ranges.
    if rest.starts_with(&[0, 1, 2, 3, 4]) {
        parts.push("Weekdays".to_string());
        rest = &rest[5..];
    }
    if rest == [5, 6] {
        parts.push("Weekends".to_string());
        rest = &[];
    }
    let mut i = 0;
    while i < rest.len() {
        let mut j = i;
        while j + 1 < rest.len() && rest[j + 1] == rest[j] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            parts.push(format!("{}\u{2013}{}", DAY_SHORT[rest[i]], DAY_SHORT[rest[j]]));
        } else {
            for k in i..=j {
                parts.push(DAY_SHORT[rest[k]].to_string());
            }
        }
        i = j + 1;
    }
    parts.join(", ")
}

/// Describes a selection of pattern days ("Weekdays", "Weekends in week 2", ...).
pub fn describe_slots(slots: &[usize], weeks: usize) -> String {
    let per_week: Vec<Vec<usize>> = (0..weeks).map(|w| slots.iter().filter(|s| *s / 7 == w).map(|s| s % 7).collect()).collect();
    if weeks <= 1 {
        return describe_days(&per_week[0]);
    }
    if per_week.iter().all(|d| !d.is_empty() && *d == per_week[0]) {
        return format!("{}, every week", describe_days(&per_week[0]));
    }
    per_week
        .iter()
        .enumerate()
        .filter(|(_, d)| !d.is_empty())
        .map(|(w, d)| format!("{} in week {}", describe_days(d), w + 1))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Runs of days with the same rule in one week: (days, rule). Weekdays and weekends are
/// grouped separately so they read as "Weekdays" / "Weekends" when they match.
pub fn week_groups(week: &[DayRule; 7]) -> Vec<(Vec<usize>, DayRule)> {
    if week.iter().all(|r| *r == week[0]) {
        return vec![((0..7).collect(), week[0].clone())];
    }
    let mut out: Vec<(Vec<usize>, DayRule)> = Vec::new();
    for range in [0..5, 5..7] {
        for d in range {
            match out.last_mut() {
                Some((days, rule)) if *rule == week[d] && d != 5 && days.last().is_some_and(|&l| l + 1 == d) => days.push(d),
                _ => out.push((vec![d], week[d].clone())),
            }
        }
    }
    out
}

// ---------------------------------------------------------------- choices

/// Small xorshift randomness (no extra crates needed for picking a cursor set).
fn random_below(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut x = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x % n.max(1) as u64) as usize
}

impl App {
    /// Whether a scheduled choice still exists (packs and presets can be removed).
    pub fn choice_valid(&self, a: &Active) -> bool {
        match &a.preset {
            Some(id) => self.preset(id).is_some(),
            None => self.find(&a.archive).is_some_and(|i| a.scheme < i.schemes.len()),
        }
    }

    /// Everything the schedule can choose from: presets, then every pack variant.
    /// (choice, name, group)
    pub fn all_choices(&self) -> Vec<(Active, String, String)> {
        let mut out = Vec::new();
        for p in self.presets() {
            out.push((Active::preset(&p.id), p.name.clone(), "Presets".to_string()));
        }
        for inst in &self.state.installed {
            let group = Path::new(&inst.archive).file_stem().unwrap_or_default().to_string_lossy().to_string();
            for (i, s) in inst.schemes.iter().enumerate() {
                out.push((Active::pack(&inst.archive, i), s.display().to_string(), group.clone()));
            }
        }
        out
    }

    /// A choice's name as the schedule shows it.
    pub fn choice_name(&self, a: &Active) -> String {
        match &a.preset {
            Some(id) => self.preset(id).map(|p| p.name.clone()).unwrap_or_else(|| "(removed preset)".into()),
            None => match self.find(&a.archive).and_then(|i| i.schemes.get(a.scheme).map(|s| (i, s))) {
                Some((i, s)) if i.schemes.len() > 1 => s.display().to_string(),
                Some((_, s)) => s.display().to_string(),
                None => "(removed pack)".into(),
            },
        }
    }

    /// A choice's normal pointer, for previews.
    pub fn choice_preview(&self, a: &Active) -> Option<String> {
        match self.slot(&a.sheet(), 0) {
            crate::edit::Slot::File(f) => Some(f),
            _ => None,
        }
    }

    /// One line about a rule: "Classic Black", "Random: A, B", "Random from all", "No change".
    pub fn rule_summary(&self, rule: &DayRule) -> String {
        match rule {
            DayRule::Off | DayRule::One { pick: None } => "No change".into(),
            DayRule::One { pick: Some(a) } => self.choice_name(a),
            DayRule::Several { picks } if picks.is_empty() => "No change".into(),
            DayRule::Several { picks } => {
                let names: Vec<String> = picks.iter().map(|a| self.choice_name(a)).collect();
                format!("Random: {}", names.join(", "))
            }
            DayRule::RandomAll => "Random from all your packs and presets".into(),
        }
    }

    /// What a rule comes to today (random rules pick now).
    fn resolve(&self, rule: &DayRule) -> Option<Active> {
        match rule {
            DayRule::Off => None,
            DayRule::One { pick } => pick.clone().filter(|a| self.choice_valid(a)),
            DayRule::Several { picks } => {
                let valid: Vec<&Active> = picks.iter().filter(|a| self.choice_valid(a)).collect();
                (!valid.is_empty()).then(|| valid[random_below(valid.len())].clone())
            }
            DayRule::RandomAll => {
                let all = self.all_choices();
                (!all.is_empty()).then(|| all[random_below(all.len())].0.clone())
            }
        }
    }

    /// Applies today's cursors if the schedule is on and today hasn't been handled yet
    /// (`force`: pick again now, e.g. after today's rule was changed). Returns what was used.
    pub fn run_schedule(&mut self, force: bool) -> Option<String> {
        let s = &self.state.schedule;
        if !s.enabled {
            return None;
        }
        let day = today();
        if !force && s.today.as_ref().is_some_and(|(d, _)| *d == day) {
            return None;
        }
        let rule = s.rule(s.slot_of(day)).clone();
        let pick = self.resolve(&rule);
        self.state.schedule.today = Some((day, pick.clone()));
        self.save();
        let target = pick?;
        let already = self.ours_is_current() && self.state.active.as_ref() == Some(&target);
        if already {
            return None;
        }
        match self.apply(target) {
            Ok(name) => {
                self.log(&format!("schedule: {name}"));
                Some(name)
            }
            Err(e) => {
                self.log(&format!("schedule failed: {e}"));
                None
            }
        }
    }

    /// The rule for a date and what it comes to (today's actual pick for today).
    pub fn day_info(&self, day: i64) -> (DayRule, Option<Active>) {
        let s = &self.state.schedule;
        let rule = s.rule(s.slot_of(day)).clone();
        let shown = match (&s.today, &rule) {
            (Some((d, pick)), _) if *d == day && s.enabled => pick.clone(),
            (_, DayRule::One { pick }) => pick.clone(),
            (_, DayRule::Several { picks }) => picks.first().cloned(),
            _ => None,
        };
        (rule, shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday(0), 3); // a Thursday
        let d = days_from_civil(2026, 11, 20);
        assert_eq!(civil_from_days(d), (2026, 11, 20));
        assert_eq!(weekday(d), 4); // Friday
        assert_eq!(days_from_civil(2028, 3, 1) - days_from_civil(2028, 2, 1), 29);
    }

    #[test]
    fn day_names() {
        assert_eq!(describe_days(&[0, 1, 2, 3, 4]), "Weekdays");
        assert_eq!(describe_days(&[5, 6]), "Weekends");
        assert_eq!(describe_days(&[0, 1, 2, 3, 4, 5, 6]), "Every day");
        assert_eq!(describe_days(&[1, 2, 3]), "Tue\u{2013}Thu");
        assert_eq!(describe_days(&[0, 2]), "Mon, Wed");
        assert_eq!(describe_days(&[0, 1, 2, 3, 4, 5]), "Weekdays, Sat");
        assert_eq!(describe_days(&[2]), "Wednesday");
        assert_eq!(describe_slots(&[0, 1, 2, 3, 4, 7, 8, 9, 10, 11], 2), "Weekdays, every week");
        assert_eq!(describe_slots(&[12, 13], 2), "Weekends in week 2");
    }

    #[test]
    fn rotation() {
        let mut s = Schedule { enabled: true, anchor: days_from_civil(2026, 11, 16), weeks: vec![Default::default(); 2], today: None };
        let mon = days_from_civil(2026, 11, 16);
        assert_eq!(s.week_of(mon), 0);
        assert_eq!(s.week_of(mon + 7), 1);
        assert_eq!(s.week_of(mon + 14), 0);
        assert_eq!(s.week_of(mon - 1), 1); // the Sunday before is in week 2's place
        s.weeks.push(Default::default());
        assert_eq!(s.week_of(mon + 14), 2);
        let groups = week_groups(&[
            DayRule::Off, DayRule::Off, DayRule::Off, DayRule::Off, DayRule::Off, DayRule::RandomAll, DayRule::RandomAll,
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, vec![0, 1, 2, 3, 4]);
        assert_eq!(groups[1].0, vec![5, 6]);
    }
}

impl App {
    /// Turns the schedule on (applying today's cursors straight away) or off.
    pub fn toggle_schedule(&mut self) {
        self.state.schedule.enabled = !self.state.schedule.enabled;
        self.state.schedule.today = None;
        self.save();
        if self.state.schedule.enabled {
            self.run_schedule(true);
        }
        crate::gfx::invalidate(self.ui.hwnd);
    }

    /// After the rules of `slots` changed: if today is one of them, pick today's cursors again.
    pub fn schedule_changed(&mut self, slots: &[usize]) {
        self.save();
        let s = &self.state.schedule;
        if s.enabled && slots.contains(&s.slot_of(today())) {
            self.run_schedule(true);
        }
    }
}
