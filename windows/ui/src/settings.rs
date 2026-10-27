// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The Settings page of the flyout: plain data, no Win32. The gear opens it inside the same window.
//!
//! The app describes the page as sections of rows (`UiSettings`); the flyout lays it out
//! (`settings_layout`), draws it (`settings_draw`) and turns clicks and keys into `FlyoutHit::Setting`
//! hits that carry the row's id and a part of its control. To add a section, push a
//! `SettingsSection` of rows; to add a control that exists already (toggle, choice, stepper, link,
//! value, text field), build its row; nothing else in this crate changes.

/// The id of a row; the app picks them and reads them back from a hit.
pub type RowId = u16;

/// The whole row (a toggle, a link, a text field).
pub const PART_ROW: u8 = 0;
/// The minus button of a stepper.
pub const PART_MINUS: u8 = 1;
/// The plus button of a stepper.
pub const PART_PLUS: u8 = 2;
/// Segment `k` of a choice is `PART_SEGMENT + k`.
pub const PART_SEGMENT: u8 = 3;

/// What a row holds at its right end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A switch.
    Toggle { on: bool },
    /// Two to four segments, one chosen.
    Choice {
        options: Vec<String>,
        selected: usize,
    },
    /// Minus, the value as text, plus. `value` moves by `step` inside `min..=max`.
    Stepper {
        value: i32,
        min: i32,
        max: i32,
        step: i32,
        /// The value as shown ("50%").
        text: String,
    },
    /// A row that opens something; `action` is the small text before the arrow ("Abrir"), may be empty.
    Link { action: String },
    /// A value shown and not changed here.
    Value { text: String },
    /// A text field. `editing` shows the caret; the flyout then sends the keys as `FlyoutHit::Edit`.
    Text {
        value: String,
        placeholder: String,
        editing: bool,
    },
    /// The About block: the mark, the name and version, and the credit.
    Identity {
        name: String,
        version: String,
        credit: String,
        author: String,
    },
    /// Only the label, quiet, across the whole card.
    Note,
}

/// One row: a label, an optional second line and the control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsRow {
    pub id: RowId,
    pub label: String,
    /// The secondary line under the label.
    pub detail: Option<String>,
    pub kind: RowKind,
    /// A disabled row is drawn quiet, takes no click and no focus.
    pub enabled: bool,
}

impl SettingsRow {
    fn new(id: RowId, label: &str, kind: RowKind) -> SettingsRow {
        SettingsRow {
            id,
            label: label.to_string(),
            detail: None,
            kind,
            enabled: true,
        }
    }

    pub fn toggle(id: RowId, label: &str, on: bool) -> SettingsRow {
        SettingsRow::new(id, label, RowKind::Toggle { on })
    }

    pub fn choice(id: RowId, label: &str, options: &[&str], selected: usize) -> SettingsRow {
        SettingsRow::new(
            id,
            label,
            RowKind::Choice {
                options: options.iter().map(|s| s.to_string()).collect(),
                selected: selected.min(options.len().saturating_sub(1)),
            },
        )
    }

    pub fn stepper(
        id: RowId,
        label: &str,
        value: i32,
        (min, max, step): (i32, i32, i32),
        text: &str,
    ) -> SettingsRow {
        SettingsRow::new(
            id,
            label,
            RowKind::Stepper {
                value,
                min,
                max,
                step,
                text: text.to_string(),
            },
        )
    }

    pub fn link(id: RowId, label: &str, action: &str) -> SettingsRow {
        SettingsRow::new(
            id,
            label,
            RowKind::Link {
                action: action.to_string(),
            },
        )
    }

    pub fn value(id: RowId, label: &str, text: &str) -> SettingsRow {
        SettingsRow::new(
            id,
            label,
            RowKind::Value {
                text: text.to_string(),
            },
        )
    }

    pub fn text(id: RowId, label: &str, value: &str, editing: bool) -> SettingsRow {
        SettingsRow::new(
            id,
            label,
            RowKind::Text {
                value: value.to_string(),
                placeholder: String::new(),
                editing,
            },
        )
    }

    pub fn note(id: RowId, label: &str) -> SettingsRow {
        SettingsRow::new(id, label, RowKind::Note)
    }

    pub fn identity(
        id: RowId,
        name: &str,
        version: &str,
        credit: &str,
        author: &str,
    ) -> SettingsRow {
        SettingsRow::new(
            id,
            name,
            RowKind::Identity {
                name: name.to_string(),
                version: version.to_string(),
                credit: credit.to_string(),
                author: author.to_string(),
            },
        )
    }

    pub fn with_detail(mut self, detail: &str) -> SettingsRow {
        self.detail = Some(detail.to_string());
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> SettingsRow {
        self.enabled = enabled;
        self
    }

    /// True when the row takes the keyboard focus.
    pub fn focusable(&self) -> bool {
        self.enabled
            && matches!(
                self.kind,
                RowKind::Toggle { .. }
                    | RowKind::Choice { .. }
                    | RowKind::Stepper { .. }
                    | RowKind::Link { .. }
                    | RowKind::Text { .. }
            )
    }

    /// True while a text field of this row is being edited.
    pub fn is_editing(&self) -> bool {
        matches!(self.kind, RowKind::Text { editing: true, .. })
    }
}

/// A titled card of rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsSection {
    pub title: String,
    pub rows: Vec<SettingsRow>,
}

impl SettingsSection {
    pub fn new(title: &str, rows: Vec<SettingsRow>) -> SettingsSection {
        SettingsSection {
            title: title.to_string(),
            rows,
        }
    }
}

/// A key the page understands besides editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    Tab,
    ShiftTab,
    Up,
    Down,
    Left,
    Right,
    Space,
    Enter,
}

/// A key typed into a text field being edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Char(char),
    Backspace,
    Enter,
    Escape,
}

/// The Settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiSettings {
    /// The header: "Configurações".
    pub title: String,
    pub sections: Vec<SettingsSection>,
    /// The row that has the keyboard focus (shown with a ring); `None` until a key moves it.
    pub focus: Option<RowId>,
}

impl UiSettings {
    pub fn new(title: &str, sections: Vec<SettingsSection>) -> UiSettings {
        UiSettings {
            title: title.to_string(),
            sections,
            focus: None,
        }
    }

    /// Every row, top to bottom.
    pub fn rows(&self) -> impl Iterator<Item = &SettingsRow> {
        self.sections.iter().flat_map(|s| s.rows.iter())
    }

    pub fn row(&self, id: RowId) -> Option<&SettingsRow> {
        self.rows().find(|r| r.id == id)
    }

    /// The ids that take the focus, top to bottom.
    pub fn focusable(&self) -> Vec<RowId> {
        self.rows()
            .filter(|r| r.focusable())
            .map(|r| r.id)
            .collect()
    }

    /// True while a text field is being edited (the keys then go to it).
    pub fn editing(&self) -> bool {
        self.rows().any(SettingsRow::is_editing)
    }

    /// Moves the focus to the next (or previous) focusable row, wrapping at the ends; from no focus it
    /// goes to the first (or last). Returns the new focus.
    pub fn move_focus(&mut self, forward: bool) -> Option<RowId> {
        let order = self.focusable();
        if order.is_empty() {
            self.focus = None;
            return None;
        }
        let at = self
            .focus
            .and_then(|f| order.iter().position(|&id| id == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => order.len() - 1,
            (Some(i), true) => (i + 1) % order.len(),
            (Some(i), false) => (i + order.len() - 1) % order.len(),
        };
        self.focus = Some(order[next]);
        self.focus
    }

    /// What a key does on the focused row: the `(id, part)` of the click it stands for, or `None`
    /// (no focus, a key the row ignores, or a move that the bounds forbid). Tab, Shift+Tab, Up and
    /// Down move the focus (`move_focus`), they never act.
    pub fn activate(&self, key: NavKey) -> Option<(RowId, u8)> {
        let row = self.row(self.focus?)?;
        if !row.focusable() {
            return None;
        }
        let act = matches!(key, NavKey::Space | NavKey::Enter);
        match &row.kind {
            RowKind::Toggle { on } => match key {
                NavKey::Space | NavKey::Enter => Some((row.id, PART_ROW)),
                NavKey::Left if *on => Some((row.id, PART_ROW)),
                NavKey::Right if !*on => Some((row.id, PART_ROW)),
                _ => None,
            },
            RowKind::Choice { options, selected } => {
                let to = match key {
                    NavKey::Left => selected.checked_sub(1)?,
                    NavKey::Right => *selected + 1,
                    NavKey::Space | NavKey::Enter => (*selected + 1) % options.len().max(1),
                    _ => return None,
                };
                (to < options.len() && to != *selected)
                    .then_some((row.id, PART_SEGMENT + u8::try_from(to).ok()?))
            }
            RowKind::Stepper {
                value, min, max, ..
            } => match key {
                NavKey::Left if value > min => Some((row.id, PART_MINUS)),
                NavKey::Right if value < max => Some((row.id, PART_PLUS)),
                _ => None,
            },
            RowKind::Link { .. } | RowKind::Text { .. } => act.then_some((row.id, PART_ROW)),
            _ => None,
        }
    }
}

/// The value of a stepper after a minus or plus click, kept inside its bounds; `None` when the part is
/// not a stepper button or the value cannot move that way.
pub fn stepped(kind: &RowKind, part: u8) -> Option<i32> {
    let RowKind::Stepper {
        value,
        min,
        max,
        step,
        ..
    } = kind
    else {
        return None;
    };
    match part {
        PART_MINUS if value > min => Some((value - step).max(*min)),
        PART_PLUS if value < max => Some((value + step).min(*max)),
        _ => None,
    }
}

/// The option a click on a choice's segment picks; `None` when the part is not one of its segments.
pub fn segment_of(kind: &RowKind, part: u8) -> Option<usize> {
    let RowKind::Choice { options, .. } = kind else {
        return None;
    };
    let k = usize::from(part.checked_sub(PART_SEGMENT)?);
    (k < options.len()).then_some(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> UiSettings {
        UiSettings::new(
            "Configurações",
            vec![
                SettingsSection::new(
                    "Aparência",
                    vec![
                        SettingsRow::choice(1, "Tema", &["Automático", "Claro", "Escuro"], 0),
                        SettingsRow::toggle(2, "Animação do ícone", true),
                    ],
                ),
                SettingsSection::new(
                    "Ícone",
                    vec![
                        SettingsRow::stepper(3, "Verde", 50, (25, 95, 5), "50%"),
                        SettingsRow::value(4, "Atalho", "Ctrl+Alt+B"),
                        SettingsRow::link(5, "Código", ""),
                        SettingsRow::toggle(6, "Desligado", false).with_enabled(false),
                        SettingsRow::note(7, "Sem rede"),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn only_enabled_controls_take_the_focus() {
        assert_eq!(page().focusable(), vec![1, 2, 3, 5]);
    }

    #[test]
    fn tab_walks_the_rows_and_wraps_in_both_directions() {
        let mut p = page();
        assert_eq!(p.move_focus(true), Some(1));
        assert_eq!(p.move_focus(true), Some(2));
        assert_eq!(p.move_focus(true), Some(3));
        assert_eq!(p.move_focus(true), Some(5));
        assert_eq!(p.move_focus(true), Some(1), "wraps to the first");
        assert_eq!(p.move_focus(false), Some(5), "and back to the last");
        let mut q = page();
        assert_eq!(
            q.move_focus(false),
            Some(5),
            "Shift+Tab from nothing is the last"
        );
        let mut empty = UiSettings::new("x", vec![]);
        assert_eq!(empty.move_focus(true), None);
    }

    #[test]
    fn a_focus_on_a_row_that_left_the_page_starts_over() {
        let mut p = page();
        p.focus = Some(99);
        assert_eq!(p.move_focus(true), Some(1));
    }

    #[test]
    fn space_and_enter_flip_a_toggle_and_arrows_set_it() {
        let mut p = page();
        p.focus = Some(2);
        assert_eq!(p.activate(NavKey::Space), Some((2, PART_ROW)));
        assert_eq!(p.activate(NavKey::Enter), Some((2, PART_ROW)));
        assert_eq!(
            p.activate(NavKey::Left),
            Some((2, PART_ROW)),
            "on: left turns it off"
        );
        assert_eq!(p.activate(NavKey::Right), None, "already on");
        assert_eq!(p.activate(NavKey::Tab), None);
        assert_eq!(p.activate(NavKey::Down), None);
    }

    #[test]
    fn arrows_move_a_choice_inside_its_bounds_and_space_cycles() {
        let mut p = page();
        p.focus = Some(1);
        assert_eq!(p.activate(NavKey::Left), None, "first option");
        assert_eq!(p.activate(NavKey::Right), Some((1, PART_SEGMENT + 1)));
        assert_eq!(p.activate(NavKey::Space), Some((1, PART_SEGMENT + 1)));
        if let RowKind::Choice { selected, .. } = &mut p.sections[0].rows[0].kind {
            *selected = 2;
        }
        assert_eq!(p.activate(NavKey::Right), None, "last option");
        assert_eq!(p.activate(NavKey::Left), Some((1, PART_SEGMENT + 1)));
        assert_eq!(
            p.activate(NavKey::Space),
            Some((1, PART_SEGMENT)),
            "wraps to the first"
        );
    }

    #[test]
    fn arrows_press_the_stepper_buttons_until_the_bounds() {
        let mut p = page();
        p.focus = Some(3);
        assert_eq!(p.activate(NavKey::Left), Some((3, PART_MINUS)));
        assert_eq!(p.activate(NavKey::Right), Some((3, PART_PLUS)));
        assert_eq!(p.activate(NavKey::Space), None);
        if let RowKind::Stepper { value, .. } = &mut p.sections[1].rows[0].kind {
            *value = 25;
        }
        assert_eq!(p.activate(NavKey::Left), None, "at the minimum");
        if let RowKind::Stepper { value, .. } = &mut p.sections[1].rows[0].kind {
            *value = 95;
        }
        assert_eq!(p.activate(NavKey::Right), None, "at the maximum");
    }

    #[test]
    fn a_link_opens_on_space_or_enter_only_and_no_focus_means_no_action() {
        let mut p = page();
        p.focus = Some(5);
        assert_eq!(p.activate(NavKey::Enter), Some((5, PART_ROW)));
        assert_eq!(p.activate(NavKey::Left), None);
        p.focus = None;
        assert_eq!(p.activate(NavKey::Space), None);
        p.focus = Some(4);
        assert_eq!(
            p.activate(NavKey::Space),
            None,
            "a shown value does not act"
        );
    }

    #[test]
    fn the_stepper_moves_by_its_step_and_stops_at_the_bounds() {
        let k = |value| RowKind::Stepper {
            value,
            min: 5,
            max: 30,
            step: 5,
            text: String::new(),
        };
        assert_eq!(stepped(&k(10), PART_PLUS), Some(15));
        assert_eq!(stepped(&k(10), PART_MINUS), Some(5));
        assert_eq!(stepped(&k(5), PART_MINUS), None);
        assert_eq!(stepped(&k(30), PART_PLUS), None);
        assert_eq!(
            stepped(&k(28), PART_PLUS),
            Some(30),
            "never past the maximum"
        );
        assert_eq!(stepped(&k(10), PART_ROW), None);
        assert_eq!(stepped(&RowKind::Note, PART_PLUS), None);
    }

    #[test]
    fn a_segment_part_names_its_option() {
        let k = RowKind::Choice {
            options: vec!["a".into(), "b".into()],
            selected: 0,
        };
        assert_eq!(segment_of(&k, PART_SEGMENT), Some(0));
        assert_eq!(segment_of(&k, PART_SEGMENT + 1), Some(1));
        assert_eq!(segment_of(&k, PART_SEGMENT + 2), None);
        assert_eq!(segment_of(&k, PART_ROW), None);
        assert_eq!(segment_of(&RowKind::Note, PART_SEGMENT), None);
    }

    #[test]
    fn a_text_field_in_edit_mode_takes_over_the_keys() {
        let mut p = page();
        assert!(!p.editing());
        p.sections[1]
            .rows
            .push(SettingsRow::text(8, "Nome", "G703", true));
        assert!(p.editing());
        assert!(p.row(8).unwrap().is_editing() && p.focusable().contains(&8));
    }
}
