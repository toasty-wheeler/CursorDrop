//! The schedule page, laid out like a phone's calendar app: a month's worth of weeks on top
//! (no dates: week 1 is each weekday's first time in the month, up to week 5) where days are
//! tapped to select them, the days listed underneath (Weekdays / Weekends per week, and the
//! special days that come back every year), and on the right the cursors for the selected
//! days: no change, one, a random pick from several, or a random pick from everything.

use super::*;
use crate::schedule::{self, DayRule, MAX_WEEKS, SPECIAL, Schedule};

pub(super) const G_CALENDAR: &str = "\u{E787}";
const G_SHUFFLE: &str = "\u{E8B1}";
const G_ADD: &str = "\u{E710}";
const G_RADIO_ON: &str = "\u{ECCB}";
const G_RADIO_OFF: &str = "\u{ECCA}";
const G_STAR: &str = "\u{E734}";

const MODES: [&str; 4] = ["No change", "One", "Random pick", "Random from all"];
const QUICK: [&str; 4] = ["Weekdays", "Weekends", "Every day", "Clear"];
const GRID_ROW_H: f32 = 36.0;
const TASK_ROW_H: f32 = 50.0;
const HEAD_H: f32 = 30.0;
const OPTION_ROW_H: f32 = 42.0;
const OPTION_GROUP_H: f32 = 28.0;

/// The schedule page's own state.
#[derive(Default)]
pub struct SchedUi {
    /// Selected days: pattern days (week * 7 + weekday) and special days (SPECIAL + index).
    pub sel: Vec<usize>,
    /// The date picker, open on this month (1-12).
    pub date_picker: Option<u32>,
    pub list_scroll: f32,
    pub list_max: f32,
    pub list_area: R,
    pub opt_scroll: f32,
    pub opt_max: f32,
    pub opt_area: R,
    /// What the ids of the last paint's elements stand for.
    pub groups: Vec<Vec<usize>>,
    pub choices: Vec<Active>,
}

/// One choice the selected days can use.
pub struct ChoiceRow {
    pub choice: Active,
    pub name: String,
    pub group: String,
    pub preview: Option<String>,
}

/// How a day looks: its rule in words, its pointer, and whether it's random.
#[derive(Clone)]
struct Look {
    summary: String,
    preview: Option<String>,
    random: bool,
}

/// What the page shows, read from the app before painting.
pub struct SchedModel {
    pub sched: Schedule,
    pub today: i64,
    pub today_name: Option<String>,
    looks: Vec<Look>,
    special_looks: Vec<Look>,
    pub choices: Vec<ChoiceRow>,
}

fn look(app: &App, rule: &DayRule) -> Look {
    let preview = match rule {
        DayRule::One { pick: Some(a) } => app.choice_preview(a),
        DayRule::Several { picks } => picks.first().and_then(|a| app.choice_preview(a)),
        _ => None,
    };
    let random = matches!(rule, DayRule::Several { picks } if picks.len() > 1) || *rule == DayRule::RandomAll;
    Look { summary: app.rule_summary(rule), preview, random }
}

impl SchedModel {
    pub fn new(app: &App) -> SchedModel {
        let sched = app.state.schedule.clone();
        let today = schedule::today();
        let looks = (0..sched.weeks.len() * 7).map(|s| look(app, sched.rule(s))).collect();
        let special_looks = sched.special.iter().map(|s| look(app, &s.rule)).collect();
        let today_name = match &sched.today {
            Some((d, Some(a))) if *d == today && sched.enabled => Some(app.choice_name(a)),
            _ => None,
        };
        let choices = app
            .all_choices()
            .into_iter()
            .map(|(choice, name, group)| {
                let preview = app.choice_preview(&choice);
                ChoiceRow { choice, name, group, preview }
            })
            .collect();
        SchedModel { sched, today, today_name, looks, special_looks, choices }
    }

    fn look(&self, slot: usize) -> Option<&Look> {
        if slot >= SPECIAL { self.special_looks.get(slot - SPECIAL) } else { self.looks.get(slot) }
    }
}

/// The selection's common rule, or None if the selected days differ.
fn common_rule(sched: &Schedule, sel: &[usize]) -> Option<DayRule> {
    let first = sched.rule(*sel.first()?).clone();
    sel.iter().all(|s| *sched.rule(*s) == first).then_some(first)
}

fn mode_of(rule: &DayRule) -> usize {
    match rule {
        DayRule::Off => 0,
        DayRule::One { .. } => 1,
        DayRule::Several { .. } => 2,
        DayRule::RandomAll => 3,
    }
}

fn quick_slots(which: usize, weeks: usize) -> Vec<usize> {
    let days: &[usize] = match which {
        0 => &[0, 1, 2, 3, 4],
        1 => &[5, 6],
        2 => &[0, 1, 2, 3, 4, 5, 6],
        _ => &[],
    };
    (0..weeks).flat_map(|w| days.iter().map(move |d| w * 7 + d)).collect()
}

/// "1, 2, 1, 2, 1": which row each week of a month uses.
fn month_turns(weeks: usize) -> String {
    (0..5).map(|w| (w % weeks.max(1) + 1).to_string()).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------- painting

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, content: R, margin: f32, w: f32, h: f32) {
    let p = pt.p;

    // Header.
    let y = pt.s(20.0);
    pt.icon_button(R::new(margin, y, pt.s(36.0), pt.s(36.0)), Kind::Back, G_BACK, None, false);
    let tx = margin + pt.s(50.0);
    pt.g.label("Schedule", &pt.f.title, p.text, R::new(tx, y - pt.s(2.0), pt.s(560.0), pt.s(28.0)));
    let status = match (m.sched.enabled, &m.today_name) {
        (true, Some(n)) => format!("On   \u{00B7}   today: {n}   \u{00B7}   repeats every month"),
        (true, None) => "On   \u{00B7}   no change today   \u{00B7}   repeats every month".to_string(),
        (false, _) => "Off   \u{00B7}   turn on \u{201C}Follow the schedule\u{201D} below to use it".to_string(),
    };
    pt.g.label(&status, &pt.f.caption, p.subtext, R::new(tx, y + pt.s(26.0), pt.s(700.0), pt.s(20.0)));

    let left_w = (content.w * 0.5).min(pt.s(560.0));
    let left = R::new(content.x, content.y, left_w, content.h);
    let right = R::new(left.right() + pt.s(16.0), content.y, content.right() - left.right() - pt.s(16.0), content.h);
    let grid = draw_weeks(pt, su, m, left);
    draw_days(pt, su, m, R::new(left.x, grid.bottom() + pt.s(12.0), left.w, left.bottom() - grid.bottom() - pt.s(12.0)));
    draw_editor(pt, su, m, right);
    if let Some(month) = su.date_picker {
        draw_date_picker(pt, m, month, w, h);
    }
}

/// The weeks of a month: Mon-Sun columns, one row per week of the pattern, + under the weekend.
fn draw_weeks(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, area: R) -> R {
    let p = pt.p;
    let weeks = m.sched.weeks.len();
    let row_h = pt.s(GRID_ROW_H);
    let card_h = pt.s(58.0 + 22.0) + weeks as f32 * row_h + pt.s(44.0 + 50.0);
    let card = R::new(area.x, area.y, area.w, card_h.min(area.h));
    pt.g.fill_round(card, pt.s(12.0), p.card);

    let x0 = card.x + pt.s(16.0);
    pt.g.label("Every month", &pt.f.title, p.text, R::new(x0, card.y + pt.s(8.0), card.w - pt.s(32.0), pt.s(28.0)));
    pt.g.label(
        "Week 1 is the 1st\u{2013}7th of each month, week 2 the 8th\u{2013}14th, and so on",
        &pt.f.caption,
        p.subtext,
        R::new(x0, card.y + pt.s(36.0), card.w - pt.s(32.0), pt.s(18.0)),
    );

    let gutter = pt.s(58.0);
    let gx = card.x + pt.s(10.0) + gutter;
    let col_w = (card.w - pt.s(20.0) - gutter) / 7.0;
    let ny = card.y + pt.s(58.0);
    for d in 0..7 {
        let c = if d >= 5 { with_alpha(pt.accent, 220) } else { p.subtext };
        pt.g.text(schedule::DAY_SHORT[d], &pt.f.caption, c, R::new(gx + d as f32 * col_w, ny, col_w, pt.s(20.0)), Align::Center, Align::Center);
    }
    let top = ny + pt.s(22.0);
    let today_slot = (m.sched.special_on(m.today).is_none()).then(|| m.sched.pattern_slot_of(m.today));
    for wk in 0..weeks {
        let ry = top + wk as f32 * row_h;
        pt.g.label(&format!("Week {}", wk + 1), &pt.f.caption, p.subtext, R::new(card.x + pt.s(14.0), ry, gutter, row_h));
        for d in 0..7 {
            let slot = wk * 7 + d;
            let cell = R::new(gx + d as f32 * col_w, ry, col_w, row_h).inset(pt.s(2.0), pt.s(2.0));
            let hovered = pt.add(Kind::PatternCell, slot, cell);
            let selected = su.sel.contains(&slot);
            let look = m.look(slot);
            let bg = if selected {
                with_alpha(pt.accent, if pt.dark { 70 } else { 55 })
            } else if hovered {
                p.hover
            } else {
                with_alpha(p.segment, if pt.dark { 90 } else { 120 })
            };
            pt.g.fill_round(cell, pt.s(8.0), bg);
            if today_slot == Some(slot) {
                pt.g.stroke_round(cell, pt.s(8.0), pt.accent, pt.s(1.6));
            }
            if let Some(l) = look {
                if let Some(path) = &l.preview {
                    pt.draw_cursor(path, cell.x + cell.w / 2.0 + pt.s(4.0), cell.y + cell.h / 2.0, pt.s(22.0), None, false);
                }
                if l.random {
                    pt.g.text(G_SHUFFLE, &pt.f.glyph, pt.accent, R::new(cell.x + pt.s(4.0), cell.bottom() - pt.s(16.0), pt.s(14.0), pt.s(14.0)), Align::Center, Align::Center);
                }
            }
        }
    }

    // Under the weekend: + adds a week. On the left: how the weeks take turns in a month.
    let py = top + weeks as f32 * row_h + pt.s(4.0);
    let plus = R::new(gx + 5.0 * col_w + pt.s(2.0), py, col_w * 2.0 - pt.s(4.0), pt.s(32.0));
    if weeks < MAX_WEEKS {
        let hovered = pt.add(Kind::AddWeek, 0, plus);
        if hovered {
            pt.g.fill_round(plus, pt.s(8.0), p.hover);
        }
        pt.g.dashed_round(plus, pt.s(8.0), with_alpha(pt.accent, 170), pt.s(1.3), 0.0);
        let label = format!("Week {}", weeks + 1);
        let lw = pt.g.measure(&label, &pt.f.caption) + pt.s(22.0);
        let lx = plus.x + (plus.w - lw) / 2.0;
        pt.g.text(G_ADD, &pt.f.glyph, pt.accent, R::new(lx, plus.y, pt.s(16.0), plus.h), Align::Center, Align::Center);
        pt.g.label(&label, &pt.f.caption, pt.accent, R::new(lx + pt.s(20.0), plus.y, lw, plus.h));
    }
    let turns = if weeks == 1 {
        "Every week is the same".to_string()
    } else if weeks == MAX_WEEKS {
        "Each week of the month has its own row".to_string()
    } else {
        format!("Weeks of a month use rows {}", month_turns(weeks))
    };
    pt.g.label(&turns, &pt.f.caption, p.subtext, R::new(card.x + pt.s(14.0), py, plus.x - card.x - pt.s(20.0), pt.s(32.0)));

    // Quick selects.
    let cy = py + pt.s(42.0);
    let mut x = card.x + pt.s(14.0);
    for (i, label) in QUICK.iter().enumerate() {
        let bw = pt.g.measure(label, &pt.f.caption) + pt.s(26.0);
        let r = R::new(x, cy, bw, pt.s(30.0));
        let on = i < 3 && !su.sel.is_empty() && su.sel == quick_slots(i, weeks);
        let hovered = pt.add(Kind::QuickSel, i, r);
        pt.g.fill_round(r, pt.s(15.0), if on { pt.accent } else if hovered { p.segment_on } else { p.segment });
        pt.g.text(label, &pt.f.caption, if on { on_accent(pt.accent) } else { p.text }, r, Align::Center, Align::Center);
        x = r.right() + pt.s(8.0);
    }
    card
}

/// One line in the day list.
#[allow(clippy::too_many_arguments)]
fn draw_line(pt: &mut Painter, su: &mut SchedUi, inner: R, r: R, slots: Vec<usize>, name: &str, look: Option<&Look>, today: bool, remove: Option<(Kind, usize)>) {
    let p = pt.p;
    let id = su.groups.len();
    su.groups.push(slots.clone());
    if !r.intersects(&inner) {
        return;
    }
    let visible = R::new(r.x, r.y.max(inner.y), r.w, r.bottom().min(inner.bottom()) - r.y.max(inner.y));
    let hovered = pt.add(Kind::Task, id, visible);
    if su.sel == slots {
        pt.g.fill_round(r, pt.s(9.0), with_alpha(pt.accent, if pt.dark { 60 } else { 45 }));
    } else if hovered {
        pt.g.fill_round(r, pt.s(9.0), p.hover);
    }
    // A coloured bar like a calendar event.
    let off = look.is_none_or(|l| l.summary == "No change");
    pt.g.fill_round(R::new(r.x + pt.s(8.0), r.y + pt.s(9.0), pt.s(4.0), r.h - pt.s(18.0)), pt.s(2.0), if off { p.line } else { pt.accent });
    let tx = r.x + pt.s(22.0);
    pt.g.label(name, &pt.f.body, p.text, R::new(tx, r.y + pt.s(4.0), r.w - pt.s(130.0), pt.s(20.0)));
    if today {
        let nw = pt.g.measure(name, &pt.f.body);
        let chip = R::new(tx + nw + pt.s(8.0), r.y + pt.s(6.0), pt.s(44.0), pt.s(16.0));
        pt.g.stroke_round(chip, pt.s(8.0), pt.accent, pt.s(1.2));
        pt.g.text("Today", &pt.f.tiny, pt.accent, chip, Align::Center, Align::Center);
    }
    let mut right = r.right();
    if let Some((kind, rid)) = remove {
        let x = R::new(r.right() - pt.s(32.0), r.y + (r.h - pt.s(24.0)) / 2.0, pt.s(26.0), pt.s(24.0));
        let h = pt.add(kind, rid, x);
        if h {
            pt.g.fill_round(x, pt.s(6.0), p.hover);
        }
        pt.g.text(G_CLOSE, &pt.f.glyph, p.subtext, x, Align::Center, Align::Center);
        right = x.x;
    }
    if let Some(l) = look {
        pt.g.label(&l.summary, &pt.f.caption, p.subtext, R::new(tx, r.y + pt.s(24.0), right - tx - pt.s(50.0), pt.s(18.0)));
        if let Some(path) = &l.preview {
            pt.draw_cursor(path, right - pt.s(24.0), r.y + r.h / 2.0, pt.s(26.0), Some(inner), false);
        }
        if l.random {
            pt.g.text(G_SHUFFLE, &pt.f.glyph, pt.accent, R::new(right - pt.s(54.0), r.y, pt.s(18.0), r.h), Align::Center, Align::Center);
        }
    }
}

/// Under the weeks: each week's days grouped by what they get, then the special days.
fn draw_days(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, area: R) {
    let p = pt.p;
    if area.h < pt.s(40.0) {
        return;
    }
    pt.g.fill_round(area, pt.s(12.0), p.card);
    let inner = area.inset(pt.s(6.0), pt.s(6.0));
    su.list_area = inner;
    let weeks = m.sched.weeks.len();
    let groups: Vec<Vec<(Vec<usize>, DayRule)>> = m.sched.weeks.iter().map(schedule::week_groups).collect();
    let mut height = 0.0;
    for g in &groups {
        height += if weeks > 1 { pt.s(HEAD_H) } else { 0.0 } + g.len() as f32 * pt.s(TASK_ROW_H);
    }
    height += pt.s(HEAD_H) + m.sched.special.len() as f32 * pt.s(TASK_ROW_H) + pt.s(48.0);
    su.list_max = (height - inner.h).max(0.0);
    su.list_scroll = su.list_scroll.clamp(0.0, su.list_max);
    su.groups.clear();

    let today_slot = m.sched.slot_of(m.today);
    pt.g.clip(inner);
    let mut y = inner.y - su.list_scroll;
    let head = |pt: &mut Painter, y: f32, text: &str| {
        pt.g.label(text, &pt.f.section, p.subtext, R::new(inner.x + pt.s(10.0), y + pt.s(6.0), inner.w - pt.s(60.0), pt.s(20.0)));
    };
    for (wk, week) in groups.iter().enumerate() {
        if weeks > 1 {
            head(pt, y, &format!("WEEK {}", wk + 1));
            let x = R::new(inner.right() - pt.s(34.0), y + pt.s(3.0), pt.s(26.0), pt.s(24.0));
            if x.intersects(&inner) {
                if pt.add(Kind::RemoveWeek, wk, x) {
                    pt.g.fill_round(x, pt.s(6.0), p.hover);
                }
                pt.g.text(G_CLOSE, &pt.f.glyph, p.subtext, x, Align::Center, Align::Center);
            }
            y += pt.s(HEAD_H);
        }
        for (days, _) in week {
            let slots: Vec<usize> = days.iter().map(|d| wk * 7 + d).collect();
            let r = R::new(inner.x, y, inner.w, pt.s(TASK_ROW_H - 4.0));
            y += pt.s(TASK_ROW_H);
            let today = slots.contains(&today_slot);
            let name = schedule::describe_days(days);
            let look = m.look(slots[0]).cloned();
            draw_line(pt, su, inner, r, slots, &name, look.as_ref(), today, None);
        }
    }

    // Special days, every year.
    head(pt, y, "SPECIAL DAYS   \u{00B7}   EVERY YEAR");
    y += pt.s(HEAD_H);
    for (i, s) in m.sched.special.iter().enumerate() {
        let r = R::new(inner.x, y, inner.w, pt.s(TASK_ROW_H - 4.0));
        y += pt.s(TASK_ROW_H);
        let slot = SPECIAL + i;
        let look = m.look(slot).cloned();
        draw_line(pt, su, inner, r, vec![slot], &schedule::date_name(s.month, s.day), look.as_ref(), today_slot == slot, Some((Kind::RemoveSpecial, i)));
    }
    let add = R::new(inner.x + pt.s(4.0), y + pt.s(4.0), inner.w - pt.s(8.0), pt.s(36.0));
    if add.intersects(&inner) {
        if pt.add(Kind::AddSpecial, 0, add) {
            pt.g.fill_round(add, pt.s(10.0), p.hover);
        }
        pt.g.dashed_round(add, pt.s(10.0), with_alpha(pt.accent, 170), pt.s(1.3), 0.0);
        let label = "Add a date (a holiday, a birthday\u{2026})";
        let lw = pt.g.measure(label, &pt.f.caption) + pt.s(26.0);
        let lx = add.x + (add.w - lw) / 2.0;
        pt.g.text(G_ADD, &pt.f.glyph, pt.accent, R::new(lx, add.y, pt.s(18.0), add.h), Align::Center, Align::Center);
        pt.g.label(label, &pt.f.caption, pt.accent, R::new(lx + pt.s(24.0), add.y, lw, add.h));
    }
    pt.g.unclip();
    pt.plain_scrollbar(inner, su.list_scroll, su.list_max);
}

/// A month and its days, to pick a date that comes back every year.
fn draw_date_picker(pt: &mut Painter, m: &SchedModel, month: u32, w: f32, h: f32) {
    let p = pt.p;
    pt.add(Kind::DateVeil, 0, R::new(0.0, 0.0, w, h));
    pt.g.fill_round(R::new(0.0, 0.0, w, h), 0.0, with_alpha(p.bg, 175));
    let (cw, ch) = (pt.s(400.0), pt.s(392.0));
    let card = R::new((w - cw) / 2.0, (h - ch) / 2.0, cw, ch);
    pt.add(Kind::DateVeil, 1, card);
    pt.g.fill_round(R::new(card.x, card.y + pt.s(4.0), card.w, card.h), pt.s(16.0), p.shadow);
    pt.g.fill_round(card, pt.s(16.0), with_alpha(p.card, 255));
    pt.g.stroke_round(card, pt.s(16.0), p.line, pt.s(1.0));
    let x0 = card.x + pt.s(22.0);
    pt.g.label("Pick a date", &pt.f.title, p.text, R::new(x0, card.y + pt.s(16.0), card.w - pt.s(44.0), pt.s(28.0)));
    pt.g.label("It gets its own cursors every year, whatever day of the week it is.", &pt.f.caption, p.subtext, R::new(x0, card.y + pt.s(44.0), card.w - pt.s(44.0), pt.s(18.0)));

    // ‹ Month ›
    let my = card.y + pt.s(74.0);
    let prev = R::new(x0, my, pt.s(32.0), pt.s(30.0));
    let next = R::new(card.right() - pt.s(22.0) - pt.s(32.0), my, pt.s(32.0), pt.s(30.0));
    pt.tab_arrow(prev, Kind::CalPrev, G_LEFT, true);
    pt.tab_arrow(next, Kind::CalNext, G_RIGHT, true);
    pt.g.text(schedule::MONTHS[month as usize - 1], &pt.f.body, p.text, R::new(prev.right(), my, next.x - prev.right(), pt.s(30.0)), Align::Center, Align::Center);

    // The days, seven to a row.
    let (_, tm, td) = schedule::civil_from_days(m.today);
    let gy = my + pt.s(42.0);
    let gw = card.w - pt.s(44.0);
    let cell_w = gw / 7.0;
    let cell_h = pt.s(38.0);
    for day in 1..=schedule::days_in_month_any_year(month) {
        let i = (day - 1) as usize;
        let r = R::new(x0 + (i % 7) as f32 * cell_w, gy + (i / 7) as f32 * cell_h, cell_w, cell_h).inset(pt.s(2.0), pt.s(2.0));
        let hovered = pt.add(Kind::CalDay, day as usize, r);
        let taken = m.sched.special.iter().any(|s| s.month == month && s.day == day);
        if taken {
            pt.g.fill_round(r, pt.s(8.0), with_alpha(pt.accent, if pt.dark { 70 } else { 55 }));
        } else if hovered {
            pt.g.fill_round(r, pt.s(8.0), p.hover);
        }
        if month == tm && day == td {
            pt.g.stroke_round(r, pt.s(8.0), pt.accent, pt.s(1.5));
        }
        pt.g.text(&day.to_string(), &pt.f.body, p.text, r, Align::Center, Align::Center);
        if taken {
            pt.g.text(G_STAR, &pt.f.tiny, pt.accent, R::new(r.right() - pt.s(13.0), r.y + pt.s(1.0), pt.s(12.0), pt.s(12.0)), Align::Center, Align::Center);
        }
    }
    let cancel = R::new(card.right() - pt.s(22.0) - pt.s(96.0), card.bottom() - pt.s(18.0) - pt.s(32.0), pt.s(96.0), pt.s(32.0));
    let hovered = pt.add(Kind::DateVeil, 0, cancel);
    pt.g.fill_round(cancel, pt.s(8.0), if hovered { p.hover } else { with_alpha(p.card, 255) });
    pt.g.stroke_round(cancel, pt.s(8.0), p.line, pt.s(1.0));
    pt.g.text("Cancel", &pt.f.caption, p.text, cancel, Align::Center, Align::Center);
}

/// On the right: what the selected days get.
fn draw_editor(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, area: R) {
    let p = pt.p;
    pt.g.fill_round(area, pt.s(12.0), p.card);
    su.choices.clear();
    let x0 = area.x + pt.s(20.0);
    let iw = area.w - pt.s(40.0);
    if su.sel.is_empty() {
        let mid = area.y + area.h / 2.0;
        pt.g.text(G_CALENDAR, &pt.f.big_icon, pt.accent, R::new(area.x, mid - pt.s(70.0), area.w, pt.s(40.0)), Align::Center, Align::Center);
        pt.g.text("Pick days to give them cursors", &pt.f.body, p.text, R::new(area.x, mid - pt.s(22.0), area.w, pt.s(22.0)), Align::Center, Align::Center);
        let hint = "Tap days in the weeks (as many as you like), use Weekdays /\nWeekends, tap a line under them, or add a special date.";
        for (i, line) in hint.lines().enumerate() {
            pt.g.text(line, &pt.f.caption, p.subtext, R::new(area.x + pt.s(20.0), mid + pt.s(6.0) + i as f32 * pt.s(18.0), area.w - pt.s(40.0), pt.s(18.0)), Align::Center, Align::Center);
        }
        su.opt_area = R::default();
        return;
    }
    let weeks = m.sched.weeks.len();
    pt.g.label(&schedule::describe_slots(&su.sel, weeks, &m.sched.special), &pt.f.title, p.text, R::new(x0, area.y + pt.s(16.0), iw, pt.s(28.0)));
    let n = su.sel.len();
    let specials = su.sel.iter().filter(|s| **s >= SPECIAL).count();
    let sub = match (specials, n - specials) {
        (0, k) => format!("{k} day{} of the weeks   \u{00B7}   repeats every month", if k == 1 { "" } else { "s" }),
        (_, 0) => "Every year, whatever day of the week".to_string(),
        (_, k) => format!("{k} day{} of the weeks and {specials} special date{}", if k == 1 { "" } else { "s" }, if specials == 1 { "" } else { "s" }),
    };
    pt.g.label(&sub, &pt.f.caption, p.subtext, R::new(x0, area.y + pt.s(44.0), iw, pt.s(18.0)));

    let common = common_rule(&m.sched, &su.sel);
    let mode = common.as_ref().map(mode_of);
    let seg = R::new(x0, area.y + pt.s(72.0), iw, pt.s(34.0));
    pt.g.fill_round(seg, pt.s(9.0), p.segment);
    let sw = seg.w / MODES.len() as f32;
    for (i, label) in MODES.iter().enumerate() {
        let r = R::new(seg.x + sw * i as f32, seg.y, sw, seg.h);
        let hovered = pt.add(Kind::Mode, i, r);
        let pill = r.inset(pt.s(2.0), pt.s(2.0));
        if mode == Some(i) {
            pt.g.fill_round(R::new(pill.x, pill.y + pt.s(1.0), pill.w, pill.h), pt.s(7.0), p.shadow);
            pt.g.fill_round(pill, pt.s(7.0), p.segment_on);
        } else if hovered {
            pt.g.fill_round(pill, pt.s(7.0), with_alpha(p.segment_on, 110));
        }
        let font = if mode == Some(i) { &pt.f.section } else { &pt.f.caption };
        pt.g.text(label, font, p.text, r.inset(pt.s(4.0), 0.0), Align::Center, Align::Center);
    }

    let body = R::new(area.x + pt.s(8.0), seg.bottom() + pt.s(12.0), area.w - pt.s(16.0), area.bottom() - seg.bottom() - pt.s(20.0));
    let text_at = |pt: &Painter, t: &str, i: usize| {
        pt.g.label(t, &pt.f.caption, p.subtext, R::new(x0, body.y + pt.s(4.0) + i as f32 * pt.s(18.0), iw, pt.s(18.0)));
    };
    let Some(rule) = common else {
        text_at(pt, "These days have different cursors. Pick an option above to give them all the same.", 0);
        su.opt_area = R::default();
        return;
    };
    match &rule {
        DayRule::Off => {
            text_at(pt, "These days keep whatever cursors you have.", 0);
            su.opt_area = R::default();
            return;
        }
        DayRule::RandomAll => {
            text_at(pt, &format!("Each of these days one of all {} packs, variants and presets", m.choices.len()), 0);
            text_at(pt, "is picked at random (and kept for the whole day).", 1);
            su.opt_area = R::default();
            return;
        }
        _ => {}
    }
    let several = matches!(rule, DayRule::Several { .. });
    let chosen: Vec<&Active> = match &rule {
        DayRule::One { pick } => pick.iter().collect(),
        DayRule::Several { picks } => picks.iter().collect(),
        _ => vec![],
    };
    let hint = if several {
        format!("Tick the ones to pick from; each day one of them is used ({} ticked).", chosen.len())
    } else {
        "Pick the cursors these days get.".to_string()
    };
    text_at(pt, &hint, 0);

    let list = R::new(body.x, body.y + pt.s(26.0), body.w, body.h - pt.s(26.0));
    su.opt_area = list;
    if m.choices.is_empty() {
        pt.g.text("Add a cursor pack or save a preset first.", &pt.f.body, p.subtext, list, Align::Center, Align::Center);
        return;
    }
    let mut groups = 0;
    let mut last = "";
    for c in &m.choices {
        if c.group != last {
            groups += 1;
            last = &c.group;
        }
    }
    let height = groups as f32 * pt.s(OPTION_GROUP_H) + m.choices.len() as f32 * pt.s(OPTION_ROW_H);
    su.opt_max = (height - list.h).max(0.0);
    su.opt_scroll = su.opt_scroll.clamp(0.0, su.opt_max);
    pt.g.clip(list);
    let mut y = list.y - su.opt_scroll;
    let mut last = String::new();
    for (i, c) in m.choices.iter().enumerate() {
        su.choices.push(c.choice.clone());
        if c.group != last {
            pt.g.label(&c.group.to_uppercase(), &pt.f.section, p.subtext, R::new(list.x + pt.s(12.0), y + pt.s(6.0), list.w - pt.s(24.0), pt.s(20.0)));
            y += pt.s(OPTION_GROUP_H);
            last = c.group.clone();
        }
        let r = R::new(list.x, y, list.w - pt.s(10.0), pt.s(OPTION_ROW_H - 4.0));
        y += pt.s(OPTION_ROW_H);
        if !r.intersects(&list) {
            continue;
        }
        let visible = R::new(r.x, r.y.max(list.y), r.w, r.bottom().min(list.bottom()) - r.y.max(list.y));
        let hovered = pt.add(Kind::Choice, i, visible);
        let on = chosen.iter().any(|a| **a == c.choice);
        if on {
            pt.g.fill_round(r, pt.s(9.0), with_alpha(pt.accent, if pt.dark { 50 } else { 35 }));
        } else if hovered {
            pt.g.fill_round(r, pt.s(9.0), p.hover);
        }
        let mark = R::new(r.x + pt.s(10.0), r.y + (r.h - pt.s(20.0)) / 2.0, pt.s(20.0), pt.s(20.0));
        if several {
            if on {
                pt.g.fill_round(mark, pt.s(5.0), pt.accent);
                pt.g.text(G_CHECK, &pt.f.glyph, on_accent(pt.accent), mark, Align::Center, Align::Center);
            } else {
                pt.g.stroke_round(mark.inset(pt.s(1.0), pt.s(1.0)), pt.s(5.0), p.subtext, pt.s(1.4));
            }
        } else {
            pt.g.text(if on { G_RADIO_ON } else { G_RADIO_OFF }, &pt.f.icon, if on { pt.accent } else { p.subtext }, mark, Align::Center, Align::Center);
        }
        if let Some(path) = &c.preview {
            pt.draw_cursor(path, r.x + pt.s(52.0), r.y + r.h / 2.0, pt.s(24.0), Some(list), false);
        }
        pt.g.label(&c.name, &pt.f.body, p.text, R::new(r.x + pt.s(74.0), r.y, r.w - pt.s(84.0), r.h));
    }
    pt.g.unclip();
    pt.plain_scrollbar(list, su.opt_scroll, su.opt_max);
}

// ---------------------------------------------------------------- actions

fn edit_selection(app: &mut App, f: impl Fn(&DayRule) -> DayRule) {
    let sel = app.ui.sched.sel.clone();
    let Some(&first) = sel.first() else { return };
    let base = common_rule(&app.state.schedule, &sel).unwrap_or_else(|| app.state.schedule.rule(first).clone());
    let new = f(&base);
    for s in &sel {
        app.state.schedule.set_rule(*s, new.clone());
    }
    app.schedule_changed(&sel);
}

/// After a change that can move today to another rule (weeks or dates added or removed).
fn today_changed(app: &mut App) {
    let slot = app.state.schedule.slot_of(schedule::today());
    app.schedule_changed(&[slot]);
}

pub(super) fn action(app: &mut App, kind: Kind, id: usize) {
    let weeks = app.state.schedule.weeks.len();
    let su = &mut app.ui.sched;
    match kind {
        Kind::PatternCell => {
            match su.sel.iter().position(|s| *s == id) {
                Some(i) => {
                    su.sel.remove(i);
                }
                None => {
                    su.sel.push(id);
                    su.sel.sort_unstable();
                }
            }
            su.opt_scroll = 0.0;
        }
        Kind::QuickSel => {
            su.sel = quick_slots(id, weeks);
            su.opt_scroll = 0.0;
        }
        Kind::Task => {
            if let Some(slots) = su.groups.get(id).cloned() {
                su.sel = if su.sel == slots { vec![] } else { slots };
                su.opt_scroll = 0.0;
            }
        }
        Kind::AddWeek if weeks < MAX_WEEKS => {
            // The new week starts as a copy of the last one; its weekend is selected so it can
            // be changed straight away.
            let last = app.state.schedule.weeks.last().cloned().unwrap_or_default();
            app.state.schedule.weeks.push(last);
            app.ui.sched.sel = vec![weeks * 7 + 5, weeks * 7 + 6];
            today_changed(app);
        }
        Kind::RemoveWeek if weeks > 1 && id < weeks => {
            app.state.schedule.weeks.remove(id);
            app.ui.sched.sel.clear();
            today_changed(app);
        }
        Kind::AddSpecial => {
            let (_, month, _) = schedule::civil_from_days(schedule::today());
            su.date_picker = Some(month);
        }
        Kind::RemoveSpecial => {
            if id < app.state.schedule.special.len() {
                app.state.schedule.special.remove(id);
                app.ui.sched.sel.clear();
                today_changed(app);
            }
        }
        Kind::CalPrev | Kind::CalNext => {
            if let Some(mo) = su.date_picker {
                su.date_picker = Some(match (kind, mo) {
                    (Kind::CalPrev, 1) => 12,
                    (Kind::CalPrev, _) => mo - 1,
                    (_, 12) => 1,
                    _ => mo + 1,
                });
            }
        }
        Kind::CalDay => {
            if let Some(month) = su.date_picker.take() {
                let slot = app.state.schedule.add_special(month, id as u32);
                app.ui.sched.sel = vec![slot];
                app.ui.sched.opt_scroll = 0.0;
                today_changed(app);
            }
        }
        Kind::DateVeil if id == 0 => su.date_picker = None,
        Kind::Mode => edit_selection(app, |base| match id {
            0 => DayRule::Off,
            1 => DayRule::One {
                pick: match base {
                    DayRule::One { pick } => pick.clone(),
                    DayRule::Several { picks } => picks.first().cloned(),
                    _ => None,
                },
            },
            2 => DayRule::Several {
                picks: match base {
                    DayRule::Several { picks } => picks.clone(),
                    DayRule::One { pick: Some(p) } => vec![p.clone()],
                    _ => vec![],
                },
            },
            _ => DayRule::RandomAll,
        }),
        Kind::Choice => {
            let Some(choice) = su.choices.get(id).cloned() else { return };
            edit_selection(app, |base| match base {
                DayRule::Several { picks } => {
                    let mut picks = picks.clone();
                    match picks.iter().position(|a| *a == choice) {
                        Some(i) => {
                            picks.remove(i);
                        }
                        None => picks.push(choice.clone()),
                    }
                    DayRule::Several { picks }
                }
                _ => DayRule::One { pick: Some(choice.clone()) },
            });
        }
        _ => {}
    }
}

/// Wheel over the day list or the choices.
pub(super) fn wheel(su: &mut SchedUi, x: f32, y: f32, step: f32) {
    if su.date_picker.is_some() {
        return;
    }
    if su.opt_area.contains(x, y) {
        su.opt_scroll = (su.opt_scroll - step).clamp(0.0, su.opt_max);
    } else if su.list_area.contains(x, y) {
        su.list_scroll = (su.list_scroll - step).clamp(0.0, su.list_max);
    }
}

/// Esc: closes the date picker if it's open (returns true then).
pub(super) fn escape(su: &mut SchedUi) -> bool {
    su.date_picker.take().is_some()
}
