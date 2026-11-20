//! The schedule page, laid out like a phone's calendar app: the month on top (tap days to
//! select them; every date shows what it gets), the days of the repeating pattern listed
//! underneath (Weekdays / Weekends per week, with + to add another week), and on the right
//! the cursors for the selected days: no change, one, a random pick from several, or a random
//! pick from everything.

use super::*;
use crate::schedule::{self, DayRule, MAX_WEEKS, Schedule};

pub(super) const G_CALENDAR: &str = "\u{E787}";
const G_SHUFFLE: &str = "\u{E8B1}";
const G_ADD: &str = "\u{E710}";
const G_RADIO_ON: &str = "\u{ECCB}";
const G_RADIO_OFF: &str = "\u{ECCA}";

const MODES: [&str; 4] = ["No change", "One", "Random pick", "Random from all"];
const QUICK: [&str; 4] = ["Weekdays", "Weekends", "Every day", "Clear"];
const CAL_ROW_H: f32 = 34.0;
const TASK_ROW_H: f32 = 50.0;
const WEEK_HEAD_H: f32 = 30.0;
const OPTION_ROW_H: f32 = 42.0;
const OPTION_GROUP_H: f32 = 28.0;

/// The schedule page's own state.
#[derive(Default)]
pub struct SchedUi {
    /// The month shown (year, month 1-12); (0, 0) = this month.
    pub month: (i64, u32),
    /// Selected days of the pattern (week * 7 + weekday), sorted.
    pub sel: Vec<usize>,
    pub list_scroll: f32,
    pub list_max: f32,
    pub list_area: R,
    pub opt_scroll: f32,
    pub opt_max: f32,
    pub opt_area: R,
    /// What the ids of the last paint's elements stand for.
    pub cal_days: Vec<i64>,
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

/// What the page shows, read from the app before painting.
pub struct SchedModel {
    pub sched: Schedule,
    pub today: i64,
    pub today_name: Option<String>,
    /// Per pattern day: (preview of the normal pointer, random?).
    pub looks: Vec<(Option<String>, bool)>,
    pub summaries: Vec<String>,
    pub choices: Vec<ChoiceRow>,
}

impl SchedModel {
    pub fn new(app: &App) -> SchedModel {
        let sched = app.state.schedule.clone();
        let today = schedule::today();
        let slots = sched.weeks.len() * 7;
        let mut looks = Vec::with_capacity(slots);
        let mut summaries = Vec::with_capacity(slots);
        for slot in 0..slots {
            let rule = sched.rule(slot);
            let preview = match rule {
                DayRule::One { pick: Some(a) } => app.choice_preview(a),
                DayRule::Several { picks } => picks.first().and_then(|a| app.choice_preview(a)),
                _ => None,
            };
            let random = matches!(rule, DayRule::Several { picks } if picks.len() > 1) || *rule == DayRule::RandomAll;
            looks.push((preview, random));
            summaries.push(app.rule_summary(rule));
        }
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
        SchedModel { sched, today, today_name, looks, summaries, choices }
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

// ---------------------------------------------------------------- painting

pub(super) fn draw(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, content: R, margin: f32) {
    let p = pt.p;
    let weeks = m.sched.weeks.len();

    // Header.
    let y = pt.s(20.0);
    pt.icon_button(R::new(margin, y, pt.s(36.0), pt.s(36.0)), Kind::Back, G_BACK, None, false);
    let tx = margin + pt.s(50.0);
    pt.g.label("Schedule", &pt.f.title, p.text, R::new(tx, y - pt.s(2.0), pt.s(560.0), pt.s(28.0)));
    let repeat = if weeks == 1 { "repeats every week".to_string() } else { format!("repeats every {weeks} weeks") };
    let status = match (m.sched.enabled, &m.today_name) {
        (true, Some(n)) => format!("On   \u{00B7}   today: {n}   \u{00B7}   {repeat}"),
        (true, None) => format!("On   \u{00B7}   no change today   \u{00B7}   {repeat}"),
        (false, _) => format!("Off   \u{00B7}   turn on \u{201C}Follow the schedule\u{201D} below to use it   \u{00B7}   {repeat}"),
    };
    pt.g.label(&status, &pt.f.caption, p.subtext, R::new(tx, y + pt.s(26.0), pt.s(700.0), pt.s(20.0)));

    let left_w = (content.w * 0.5).min(pt.s(560.0));
    let left = R::new(content.x, content.y, left_w, content.h);
    let right = R::new(left.right() + pt.s(16.0), content.y, content.right() - left.right() - pt.s(16.0), content.h);
    let cal = draw_calendar(pt, su, m, left);
    draw_tasks(pt, su, m, R::new(left.x, cal.bottom() + pt.s(12.0), left.w, left.bottom() - cal.bottom() - pt.s(12.0)));
    draw_editor(pt, su, m, right);
}

/// The month: a header with arrows, weekday names, six weeks of dates, and quick selects.
fn draw_calendar(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, area: R) -> R {
    let p = pt.p;
    let weeks = m.sched.weeks.len();
    let (ty, tm, _) = schedule::civil_from_days(m.today);
    if su.month == (0, 0) {
        su.month = (ty, tm);
    }
    let (year, month) = su.month;
    let row_h = pt.s(CAL_ROW_H);
    let card_h = pt.s(44.0 + 22.0) + 6.0 * row_h + pt.s(52.0);
    let card = R::new(area.x, area.y, area.w, card_h.min(area.h));
    pt.g.fill_round(card, pt.s(12.0), p.card);

    // Month name and navigation.
    let head = R::new(card.x + pt.s(16.0), card.y + pt.s(8.0), card.w - pt.s(32.0), pt.s(32.0));
    pt.g.label(&format!("{} {year}", schedule::MONTHS[month as usize - 1]), &pt.f.title, p.text, head);
    let next = R::new(head.right() - pt.s(30.0), head.y + pt.s(2.0), pt.s(30.0), pt.s(28.0));
    let prev = R::new(next.x - pt.s(36.0), next.y, pt.s(30.0), pt.s(28.0));
    pt.tab_arrow(next, Kind::CalNext, G_RIGHT, true);
    pt.tab_arrow(prev, Kind::CalPrev, G_LEFT, true);
    if (year, month) != (ty, tm) {
        let today_btn = R::new(prev.x - pt.s(76.0), prev.y, pt.s(68.0), pt.s(28.0));
        let hovered = pt.add(Kind::CalToday, 0, today_btn);
        pt.g.fill_round(today_btn, pt.s(9.0), if hovered { p.segment_on } else { p.segment });
        pt.g.text("Today", &pt.f.caption, p.text, today_btn, Align::Center, Align::Center);
    }

    // Weekday names, then the dates; with several weeks a gutter says which week each row is.
    let gutter = if weeks > 1 { pt.s(30.0) } else { 0.0 };
    let gx = card.x + pt.s(10.0) + gutter;
    let col_w = (card.w - pt.s(20.0) - gutter) / 7.0;
    let wy = card.y + pt.s(44.0);
    for d in 0..7 {
        let c = if d >= 5 { with_alpha(pt.accent, 220) } else { p.subtext };
        pt.g.text(schedule::DAY_SHORT[d], &pt.f.caption, c, R::new(gx + d as f32 * col_w, wy, col_w, pt.s(20.0)), Align::Center, Align::Center);
    }
    let first = schedule::days_from_civil(year, month, 1);
    let start = first - schedule::weekday(first) as i64;
    su.cal_days.clear();
    let top = wy + pt.s(22.0);
    let today_slot = m.sched.slot_of(m.today);
    for row in 0..6 {
        let ry = top + row as f32 * row_h;
        let monday = start + row * 7;
        if weeks > 1 {
            let w = m.sched.week_of(monday);
            pt.g.text(&format!("W{}", w + 1), &pt.f.tiny, p.subtext, R::new(card.x + pt.s(8.0), ry, gutter, row_h), Align::Center, Align::Center);
        }
        for d in 0..7 {
            let day = monday + d as i64;
            let (_, dm, dd) = schedule::civil_from_days(day);
            let cell = R::new(gx + d as f32 * col_w, ry, col_w, row_h).inset(pt.s(2.0), pt.s(2.0));
            let id = su.cal_days.len();
            su.cal_days.push(day);
            let slot = m.sched.slot_of(day);
            let hovered = pt.add(Kind::CalDay, id, cell);
            let selected = su.sel.contains(&slot);
            if selected {
                pt.g.fill_round(cell, pt.s(8.0), with_alpha(pt.accent, if pt.dark { 70 } else { 55 }));
            } else if hovered {
                pt.g.fill_round(cell, pt.s(8.0), p.hover);
            }
            let is_today = day == m.today;
            if is_today {
                pt.g.stroke_round(cell, pt.s(8.0), pt.accent, pt.s(1.6));
            }
            let in_month = dm == month;
            let num_colour = if is_today { pt.accent } else if in_month { p.text } else { with_alpha(p.subtext, 120) };
            let font = if is_today { &pt.f.section } else { &pt.f.caption };
            pt.g.label(&dd.to_string(), font, num_colour, R::new(cell.x + pt.s(6.0), cell.y + pt.s(2.0), cell.w, pt.s(16.0)));
            // What the day gets: its pointer (and a shuffle mark for random days).
            if let Some((preview, random)) = m.looks.get(slot) {
                let alpha_ok = in_month || selected;
                if let Some(path) = preview {
                    if alpha_ok {
                        pt.draw_cursor(path, cell.right() - pt.s(12.0), cell.bottom() - pt.s(11.0), pt.s(18.0), None, false);
                    }
                }
                if *random {
                    let c = if alpha_ok { pt.accent } else { with_alpha(pt.accent, 110) };
                    pt.g.text(G_SHUFFLE, &pt.f.glyph, c, R::new(cell.x + pt.s(4.0), cell.bottom() - pt.s(16.0), pt.s(14.0), pt.s(14.0)), Align::Center, Align::Center);
                }
            }
            let _ = today_slot;
        }
    }

    // Quick selects.
    let cy = top + 6.0 * row_h + pt.s(10.0);
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

/// Under the calendar: each week's days grouped by what they get, like a day's tasks.
fn draw_tasks(pt: &mut Painter, su: &mut SchedUi, m: &SchedModel, area: R) {
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
        height += if weeks > 1 { pt.s(WEEK_HEAD_H) } else { 0.0 } + g.len() as f32 * pt.s(TASK_ROW_H);
    }
    height += pt.s(48.0);
    su.list_max = (height - inner.h).max(0.0);
    su.list_scroll = su.list_scroll.clamp(0.0, su.list_max);
    su.groups.clear();

    let today_slot = m.sched.slot_of(m.today);
    pt.g.clip(inner);
    let mut y = inner.y - su.list_scroll;
    for (w, week) in groups.iter().enumerate() {
        if weeks > 1 {
            let this = if today_slot / 7 == w { "   \u{00B7}   THIS WEEK" } else { "" };
            pt.g.label(&format!("WEEK {}{this}", w + 1), &pt.f.section, p.subtext, R::new(inner.x + pt.s(10.0), y + pt.s(6.0), inner.w - pt.s(60.0), pt.s(20.0)));
            let x = R::new(inner.right() - pt.s(34.0), y + pt.s(3.0), pt.s(26.0), pt.s(24.0));
            if x.intersects(&inner) {
                let hovered = pt.add(Kind::RemoveWeek, w, x);
                if hovered {
                    pt.g.fill_round(x, pt.s(6.0), p.hover);
                }
                pt.g.text(G_CLOSE, &pt.f.glyph, p.subtext, x, Align::Center, Align::Center);
            }
            y += pt.s(WEEK_HEAD_H);
        }
        for (days, rule) in week {
            let slots: Vec<usize> = days.iter().map(|d| w * 7 + d).collect();
            let r = R::new(inner.x, y, inner.w, pt.s(TASK_ROW_H - 4.0));
            y += pt.s(TASK_ROW_H);
            let id = su.groups.len();
            su.groups.push(slots.clone());
            if !r.intersects(&inner) {
                continue;
            }
            let visible = R::new(r.x, r.y.max(inner.y), r.w, r.bottom().min(inner.bottom()) - r.y.max(inner.y));
            let hovered = pt.add(Kind::Task, id, visible);
            let selected = su.sel == slots;
            if selected {
                pt.g.fill_round(r, pt.s(9.0), with_alpha(pt.accent, if pt.dark { 60 } else { 45 }));
            } else if hovered {
                pt.g.fill_round(r, pt.s(9.0), p.hover);
            }
            // A coloured bar like a calendar event; brighter on today's line.
            let bar = if *rule == DayRule::Off { p.line } else { pt.accent };
            pt.g.fill_round(R::new(r.x + pt.s(8.0), r.y + pt.s(9.0), pt.s(4.0), r.h - pt.s(18.0)), pt.s(2.0), bar);
            let tx = r.x + pt.s(22.0);
            let name = schedule::describe_days(days);
            let today_here = slots.contains(&today_slot);
            pt.g.label(&name, &pt.f.body, p.text, R::new(tx, r.y + pt.s(4.0), r.w - pt.s(110.0), pt.s(20.0)));
            if today_here {
                let nw = pt.g.measure(&name, &pt.f.body);
                let chip = R::new(tx + nw + pt.s(8.0), r.y + pt.s(6.0), pt.s(44.0), pt.s(16.0));
                pt.g.stroke_round(chip, pt.s(8.0), pt.accent, pt.s(1.2));
                pt.g.text("Today", &pt.f.tiny, pt.accent, chip, Align::Center, Align::Center);
            }
            let summary = m.summaries.get(slots[0]).cloned().unwrap_or_default();
            pt.g.label(&summary, &pt.f.caption, p.subtext, R::new(tx, r.y + pt.s(24.0), r.w - pt.s(80.0), pt.s(18.0)));
            if let Some((preview, random)) = m.looks.get(slots[0]) {
                if let Some(path) = preview {
                    pt.draw_cursor(path, r.right() - pt.s(26.0), r.y + r.h / 2.0, pt.s(26.0), Some(inner), false);
                }
                if *random {
                    pt.g.text(G_SHUFFLE, &pt.f.glyph, pt.accent, R::new(r.right() - pt.s(56.0), r.y, pt.s(18.0), r.h), Align::Center, Align::Center);
                }
            }
        }
    }
    // The + under the last weekend adds a week to the pattern.
    let add = R::new(inner.x + pt.s(4.0), y + pt.s(6.0), inner.w - pt.s(8.0), pt.s(36.0));
    if add.intersects(&inner) {
        if weeks < MAX_WEEKS {
            let hovered = pt.add(Kind::AddWeek, 0, add);
            if hovered {
                pt.g.fill_round(add, pt.s(10.0), p.hover);
            }
            pt.g.dashed_round(add, pt.s(10.0), with_alpha(pt.accent, 170), pt.s(1.3), 0.0);
            let label = format!("Add week {}   \u{00B7}   then it repeats every {} weeks", weeks + 1, weeks + 1);
            let lw = pt.g.measure(&label, &pt.f.caption) + pt.s(26.0);
            let lx = add.x + (add.w - lw) / 2.0;
            pt.g.text(G_ADD, &pt.f.glyph, pt.accent, R::new(lx, add.y, pt.s(18.0), add.h), Align::Center, Align::Center);
            pt.g.label(&label, &pt.f.caption, pt.accent, R::new(lx + pt.s(24.0), add.y, lw, add.h));
        } else {
            pt.g.text("5 weeks is the most a pattern can have", &pt.f.caption, p.subtext, add, Align::Center, Align::Center);
        }
    }
    pt.g.unclip();
    pt.plain_scrollbar(inner, su.list_scroll, su.list_max);
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
        let hint = "Tap dates on the calendar (as many as you like),\nuse Weekdays / Weekends, or tap a line under it.";
        for (i, line) in hint.lines().enumerate() {
            pt.g.text(line, &pt.f.caption, p.subtext, R::new(area.x + pt.s(20.0), mid + pt.s(6.0) + i as f32 * pt.s(18.0), area.w - pt.s(40.0), pt.s(18.0)), Align::Center, Align::Center);
        }
        su.opt_area = R::default();
        return;
    }
    let weeks = m.sched.weeks.len();
    pt.g.label(&schedule::describe_slots(&su.sel, weeks), &pt.f.title, p.text, R::new(x0, area.y + pt.s(16.0), iw, pt.s(28.0)));
    let n = su.sel.len();
    let sub = format!("{n} day{} of the pattern   \u{00B7}   {}", if n == 1 { "" } else { "s" }, if weeks == 1 { "every week".to_string() } else { format!("repeats every {weeks} weeks") });
    pt.g.label(&sub, &pt.f.caption, p.subtext, R::new(x0, area.y + pt.s(44.0), iw, pt.s(18.0)));

    let common = common_rule(&m.sched, &su.sel);
    let mode = common.as_ref().map(mode_of);
    // Mode: a segmented control.
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
        // Radio for One, checkbox for Random pick.
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

pub(super) fn action(app: &mut App, kind: Kind, id: usize) {
    let weeks = app.state.schedule.weeks.len();
    let su = &mut app.ui.sched;
    match kind {
        Kind::CalPrev | Kind::CalNext => {
            let (y, mo) = su.month;
            su.month = match (kind, mo) {
                (Kind::CalPrev, 1) => (y - 1, 12),
                (Kind::CalPrev, _) => (y, mo - 1),
                (_, 12) => (y + 1, 1),
                _ => (y, mo + 1),
            };
        }
        Kind::CalToday => su.month = (0, 0),
        Kind::CalDay => {
            if let Some(&day) = su.cal_days.get(id) {
                let slot = app.state.schedule.slot_of(day);
                let su = &mut app.ui.sched;
                match su.sel.iter().position(|s| *s == slot) {
                    Some(i) => {
                        su.sel.remove(i);
                    }
                    None => {
                        su.sel.push(slot);
                        su.sel.sort_unstable();
                    }
                }
                su.opt_scroll = 0.0;
            }
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
            let all: Vec<usize> = (0..(weeks + 1) * 7).collect();
            app.schedule_changed(&all);
        }
        Kind::RemoveWeek if weeks > 1 && id < weeks => {
            app.state.schedule.weeks.remove(id);
            app.ui.sched.sel.clear();
            let all: Vec<usize> = (0..weeks * 7).collect();
            app.schedule_changed(&all);
        }
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
    if su.opt_area.contains(x, y) {
        su.opt_scroll = (su.opt_scroll - step).clamp(0.0, su.opt_max);
    } else if su.list_area.contains(x, y) {
        su.list_scroll = (su.list_scroll - step).clamp(0.0, su.list_max);
    }
}
