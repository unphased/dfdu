//! The columns of the list and how wide each is. [`layout`] decides, for the width of the list
//! and the columns the rows on screen have something to show in ([`ColSet`]), which columns are
//! drawn and how wide. The name always comes last and takes what is left, at least [`NAME_MIN`]
//! cells while the other columns can make room: optional columns not being sorted go first,
//! and the bar shrinks step by step. A row's cells are aligned with [`right`] and [`left`], and
//! the cursor row is filled with [`cursor`].

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// A column of the list, in the order they are drawn from left to right.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Col {
    /// The mark (`●`) and the entry's state glyph.
    Gutter,
    Size,
    Bar,
    /// The share of the directory's total.
    Share,
    Items,
    /// The modification date.
    Mtime,
    /// The change in size since the last recorded scan.
    Delta,
    /// `↔`: some of its data is also used outside the scan.
    Shared,
    Category,
    /// The name and the notes after it.
    Name,
}

impl Col {
    /// The blank cells before the column, after the column `after` it.
    pub fn gap(self, after: Option<Col>) -> u16 {
        match (self, after) {
            (_, None) | (Col::Size, _) => 0,
            // The flag belongs to the name it is next to.
            (Col::Name, Some(Col::Shared)) => 1,
            _ => 2,
        }
    }

    /// The column header, at most as wide as the column; empty for the gutter and the bar.
    pub fn title(self) -> &'static str {
        match self {
            Col::Gutter | Col::Bar => "",
            Col::Size => "size",
            Col::Share => "share",
            Col::Items => "items",
            Col::Mtime => "modified",
            // With the baseline date added by the list.
            Col::Delta => "Δ",
            Col::Shared => "↔",
            Col::Category => "category",
            Col::Name => "name",
        }
    }
}

/// The name never gets fewer cells than this while another column could make room.
pub const NAME_MIN: u16 = 30;
/// The name gets up to this many cells, or half the width if less, before the bar shrinks below
/// 12 cells and the items and the change since the last scan are shown.
const NAME_ROOM: u16 = 50;
/// The widths the bar shrinks through as the list narrows.
const BAR_STEPS: [u16; 5] = [24, 16, 12, 6, 0];
/// The widest category column.
pub const CATEGORY_MAX: u16 = 12;

/// Fixed widths of the columns that have one.
const SIZE: u16 = 10;
const GUTTER: u16 = 2;
const SHARE: u16 = 6;
const ITEMS: u16 = 6;
const MTIME: u16 = 10;
const DELTA: u16 = 10;

/// The optional columns the rows on screen have something to show in.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ColSet {
    pub items: bool,
    pub mtime: bool,
    /// The optional column required by the active sort, regardless of `c`.
    pub sorted: Option<Col>,
    /// Some row changed noticeably since the last recorded scan.
    pub delta: bool,
    /// Some row has data also used outside the scan.
    pub shared: bool,
    /// The widest category on screen, 0 when no row has one.
    pub category: u16,
}

/// Which optional columns the user asked for with `c`, as ncdu's `c` and `m` do.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum Columns {
    /// Items and the change since the last scan, as the width allows.
    #[default]
    Items,
    /// Items, the modification date and the change since the last scan.
    ItemsMtime,
    /// Neither: size, bar, share and name.
    None,
}

impl Columns {
    /// The next choice of `c`.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Columns::Items => Columns::ItemsMtime,
            Columns::ItemsMtime => Columns::None,
            Columns::None => Columns::Items,
        }
    }

    /// The status line after `c`.
    pub fn describe(self) -> &'static str {
        match self {
            Columns::Items => "columns: items and change since the last scan",
            Columns::ItemsMtime => "columns: items, modified and change since the last scan",
            Columns::None => "columns: size and share only",
        }
    }

    pub fn items(self) -> bool {
        self != Columns::None
    }

    pub fn mtime(self) -> bool {
        self == Columns::ItemsMtime
    }

    /// The change since the last recorded scan, when there is one.
    pub fn delta(self) -> bool {
        self != Columns::None
    }
}

/// One step that makes room for the name, and the room it makes room for.
#[derive(Copy, Clone)]
enum Step {
    Drop(Col),
    Bar(u16),
}

/// The steps in order, each taken while the name is narrower than its goal: the first ones
/// while it is under its preferred room ([`Goal::Room`]), the rest only under [`NAME_MIN`].
const STEPS: [(Step, Goal); 9] = [
    (Step::Drop(Col::Delta), Goal::Room),
    (Step::Bar(BAR_STEPS[1]), Goal::Room),
    (Step::Drop(Col::Items), Goal::Room),
    (Step::Bar(BAR_STEPS[2]), Goal::Room),
    (Step::Bar(BAR_STEPS[3]), Goal::Min),
    (Step::Drop(Col::Mtime), Goal::Min),
    (Step::Drop(Col::Category), Goal::Min),
    (Step::Bar(BAR_STEPS[4]), Goal::Min),
    (Step::Drop(Col::Shared), Goal::Min),
];

#[derive(Copy, Clone)]
enum Goal {
    Room,
    Min,
}

/// The columns drawn on a list `width` cells wide, left to right, with the cells each takes
/// (not counting the gap before it, [`Col::gap`]). The gaps and widths add up to `width`, but
/// for a list too narrow even for the size, share and active sort column.
pub fn layout(width: u16, present: ColSet) -> Vec<(Col, u16)> {
    let mut cols: Vec<(Col, u16)> = [
        (Col::Gutter, GUTTER),
        (Col::Size, SIZE),
        (Col::Bar, BAR_STEPS[0]),
        (Col::Share, SHARE),
        (Col::Items, ITEMS),
        (
            Col::Mtime,
            if present.sorted == Some(Col::Mtime) {
                19
            } else {
                MTIME
            },
        ),
        (Col::Delta, DELTA),
        (Col::Shared, 1),
        (Col::Category, present.category.min(CATEGORY_MAX)),
    ]
    .into_iter()
    .filter(|&(c, w)| match c {
        Col::Items => present.items || present.sorted == Some(Col::Items),
        Col::Mtime => present.mtime || present.sorted == Some(Col::Mtime),
        Col::Delta => present.delta,
        Col::Shared => present.shared,
        Col::Category => w > 0,
        _ => true,
    })
    .collect();
    let room = (width / 2).clamp(NAME_MIN, NAME_ROOM);
    for (step, goal) in STEPS {
        let goal = match goal {
            Goal::Room => room,
            Goal::Min => NAME_MIN,
        };
        if name_width(width, &cols) >= goal {
            continue;
        }
        match step {
            Step::Drop(c) => cols.retain(|&(col, _)| col != c || Some(col) == present.sorted),
            Step::Bar(w) => {
                for col in &mut cols {
                    if col.0 == Col::Bar {
                        col.1 = col.1.min(w);
                    }
                }
                cols.retain(|&(c, w)| c != Col::Bar || w > 0);
            }
        }
    }
    // The value being sorted matters more than the share when space is tight.
    if present.sorted.is_some() && name_width(width, &cols) < NAME_MIN {
        cols.retain(|&(c, _)| c != Col::Share);
    }
    let name = name_width(width, &cols);
    cols.push((Col::Name, name));
    cols
}

/// The cells left for the name after `cols` and their gaps.
fn name_width(width: u16, cols: &[(Col, u16)]) -> u16 {
    let used = gaps(cols).fold(0u16, |used, (gap, _, w)| used.saturating_add(gap + w));
    let last = cols.last().map(|&(c, _)| c);
    width.saturating_sub(used + Col::Name.gap(last))
}

/// The columns `cols` from left to right, each with the blank cells before it ([`Col::gap`]):
/// `(gap, column, width)`.
pub fn gaps(cols: &[(Col, u16)]) -> impl Iterator<Item = (u16, Col, u16)> + '_ {
    let mut prev = None;
    cols.iter().map(move |&(c, w)| {
        let gap = c.gap(prev);
        prev = Some(c);
        (gap, c, w)
    })
}

/// `n` blank cells.
pub fn blank(n: u16) -> Span<'static> {
    Span::raw(" ".repeat(n.into()))
}

/// `s` right-aligned in `w` cells.
pub fn right(s: String, w: u16, style: Style) -> Span<'static> {
    let pad = usize::from(w).saturating_sub(Span::raw(&s).width());
    Span::styled(format!("{}{s}", " ".repeat(pad)), style)
}

/// `s` left-aligned in `w` cells, cut to fit.
pub fn left(s: String, w: u16, style: Style) -> Span<'static> {
    let s = crate::format::cut_end(&s, w.into());
    let pad = usize::from(w).saturating_sub(Span::raw(&s).width());
    Span::styled(format!("{s}{}", " ".repeat(pad)), style)
}

/// The cursor row: `line` filled to `width` cells in the cursor style, with dim text in the
/// normal text colour so it stays readable on the cursor's background.
pub fn cursor(t: &Theme, line: Line<'static>, width: u16) -> Line<'static> {
    let dim = t.fg_dim;
    let mut spans = line.spans;
    for s in &mut spans {
        let is_dim = (dim.fg.is_some() && s.style.fg == dim.fg)
            || (!dim.add_modifier.is_empty() && s.style.add_modifier.contains(dim.add_modifier));
        if is_dim {
            s.style.fg = t.fg.fg;
            s.style.add_modifier = s.style.add_modifier.difference(dim.add_modifier);
        }
    }
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(
        " ".repeat(usize::from(width).saturating_sub(used)),
    ));
    Line::from(spans).style(t.cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(category: u16) -> ColSet {
        ColSet {
            items: true,
            mtime: false,
            sorted: None,
            delta: true,
            shared: true,
            category,
        }
    }

    /// The columns and their widths, as `size10 bar24 ... name50`.
    fn shown(width: u16, present: ColSet) -> String {
        let cols = layout(width, present);
        let used: u16 = gaps(&cols).map(|(gap, _, w)| gap + w).sum();
        assert_eq!(used, width, "{cols:?}");
        cols.iter()
            .map(|(c, w)| format!("{c:?}{w}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn wide_lists_show_every_column() {
        assert_eq!(
            shown(120, all(0)),
            "Gutter2 Size10 Bar24 Share6 Items6 Delta10 Shared1 Name50"
        );
        // Wider still, the name gets the rest.
        assert_eq!(
            shown(160, all(0)),
            "Gutter2 Size10 Bar24 Share6 Items6 Delta10 Shared1 Name90"
        );
    }

    #[test]
    fn the_change_goes_first_then_the_bar_shrinks() {
        assert_eq!(
            shown(100, all(0)),
            "Gutter2 Size10 Bar16 Share6 Items6 Shared1 Name50"
        );
        assert_eq!(
            shown(80, all(0)),
            "Gutter2 Size10 Bar12 Share6 Shared1 Name42"
        );
        assert_eq!(
            shown(64, all(0)),
            "Gutter2 Size10 Bar6 Share6 Shared1 Name32"
        );
        assert_eq!(shown(48, all(0)), "Gutter2 Size10 Share6 Name26");
    }

    #[test]
    fn the_name_keeps_its_minimum_while_columns_can_go() {
        for width in 54..200 {
            let cols = layout(width, all(9));
            let name = cols.last().expect("name");
            assert_eq!(name.0, Col::Name);
            assert!(name.1 >= NAME_MIN, "{width}: {cols:?}");
        }
        // Below that only the size, the share and the name are left.
        assert_eq!(shown(40, all(9)), "Gutter2 Size10 Share6 Name18");
        let tiny = layout(10, all(9));
        assert_eq!(tiny.last(), Some(&(Col::Name, 0)), "{tiny:?}");
    }

    #[test]
    fn a_category_shows_while_the_name_keeps_its_minimum() {
        assert_eq!(
            shown(120, all(5)),
            "Gutter2 Size10 Bar24 Share6 Items6 Shared1 Category5 Name54"
        );
        assert_eq!(
            shown(80, all(8)),
            "Gutter2 Size10 Bar12 Share6 Shared1 Category8 Name31"
        );
        // At 64 columns it goes before the bar does.
        assert_eq!(
            shown(64, all(8)),
            "Gutter2 Size10 Bar6 Share6 Shared1 Name32"
        );
        // Never wider than the widest allowed.
        assert_eq!(
            shown(200, all(30)),
            "Gutter2 Size10 Bar24 Share6 Items6 Delta10 Shared1 Category12 Name115"
        );
    }

    #[test]
    fn columns_without_anything_to_show_take_no_room() {
        let bare = ColSet::default();
        assert_eq!(shown(120, bare), "Gutter2 Size10 Bar24 Share6 Name72");
        let mtime = ColSet {
            mtime: true,
            ..all(0)
        };
        // The date the user asked for stays longer than the change and the items.
        assert_eq!(
            shown(120, mtime),
            "Gutter2 Size10 Bar24 Share6 Items6 Mtime10 Shared1 Name50"
        );
        assert_eq!(
            shown(80, mtime),
            "Gutter2 Size10 Bar12 Share6 Mtime10 Shared1 Name30"
        );
    }

    #[test]
    fn the_active_sort_column_takes_priority_over_bars_and_share() {
        for sorted in [Col::Items, Col::Mtime] {
            for width in 40..160 {
                let present = ColSet {
                    sorted: Some(sorted),
                    ..ColSet::default()
                };
                let cols = layout(width, present);
                assert!(cols.iter().any(|&(c, _)| c == sorted), "{width}: {cols:?}");
                let used: u16 = gaps(&cols).map(|(gap, _, w)| gap + w).sum();
                assert_eq!(used, width, "{cols:?}");
                assert!(cols.last().unwrap().1 > 0, "{cols:?}");
            }
        }
        assert_eq!(
            shown(
                48,
                ColSet {
                    sorted: Some(Col::Mtime),
                    ..ColSet::default()
                }
            ),
            "Gutter2 Size10 Mtime19 Name13"
        );
    }

    #[test]
    fn c_cycles_the_optional_columns() {
        let mut c = Columns::default();
        assert!(c.items() && !c.mtime() && c.delta());
        c = c.next();
        assert!(c.items() && c.mtime() && c.delta());
        c = c.next();
        assert!(!c.items() && !c.mtime() && !c.delta());
        assert_eq!(c.next(), Columns::default());
    }
}
