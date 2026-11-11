//! Carrying out the actions of the registry ([`run`]): what each does to the state of the
//! browser, and the effects it leaves. Some are only the front end's (quitting, the help, the
//! command palette, the debug panel, the list's columns, the type picker and the prompt of mark
//! by pattern and of go to folder, moving the focus to a pane, the clipboard): [`run`] leaves
//! those to it ([`Ran::FrontEnd`]). `Parent` at the scan root goes up past it in two presses
//! soon after each other ([`goto::up_at_root`]). In a read-only browser an action that deletes
//! only says so.

use std::time::Instant;

use dfdu_engine::{Command, Mode, SizeMode};

use crate::actions::{Action, first_key};
use crate::effect::Effect;
use crate::format::{human, plural};
use crate::order::{SortKey, SortSpec};
use crate::snapshot::Mark;
use crate::state::{READ_ONLY, SessionState};
use crate::view::{self, Then, View};
use crate::{cleanup, confirm, details, dupes, filter, largest, marks, selected_path, treemap};
use crate::{goto, tree, type_filter};

/// What [`run`] did with an action.
#[derive(Debug)]
#[must_use]
#[expect(
    clippy::large_enum_variant,
    reason = "returned once per action and taken apart at once, never stored"
)]
pub enum Ran {
    /// The session carried it out; the front end follows what is left.
    Done(Then),
    /// It is the front end's: it opens or closes something only the front end shows, or quits.
    FrontEnd,
}

impl Ran {
    fn nothing() -> Self {
        Ran::Done(Then::nothing())
    }
}

/// Carries out `a` on the browser (no overlay open) at `now`, as its key does.
pub fn run(s: &mut SessionState, a: Action, now: Instant) -> Ran {
    // Any action but a second `Parent` at the scan root disarms the first.
    let armed = s.take_arm();
    if s.read_only && a.deletes() {
        s.status = Some(READ_ONLY.into());
        return Ran::nothing();
    }
    let page = isize::try_from(s.page_rows()).unwrap_or(isize::MAX);
    let before = s.selected_index();
    let mut then = Then::nothing();
    match a {
        Action::Quit
        | Action::Columns
        | Action::Help
        | Action::Palette
        | Action::Debug
        | Action::Types
        | Action::FocusDetails
        | Action::CopyPath
        | Action::YankPeer
        | Action::GoTo
        | Action::PrevMatch
        | Action::NextMatch => return Ran::FrontEnd,
        Action::MarkPattern if !s.read_only => return Ran::FrontEnd,
        Action::MarkPattern => s.status = Some(READ_ONLY.into()),
        Action::Up => s.move_by(-1),
        Action::Down => s.move_by(1),
        Action::PageUp => s.move_by(-page),
        Action::PageDown => s.move_by(page),
        Action::HalfPageUp => s.move_by(-(page / 2).max(1)),
        Action::HalfPageDown => s.move_by((page / 2).max(1)),
        Action::First => s.move_to(0),
        Action::Last => s.move_to(s.list.rows.len().saturating_sub(1)),
        Action::Open if tree::act::right(s) => {}
        Action::Open | Action::Enter => {
            let dir = s.selected_index().map(|row| s.list.rows[row]);
            if let Some(i) = dir.filter(|&i| s.snap.entries[i].is_dir) {
                then.entered = Some(s.enter(i));
            }
        }
        Action::Parent if tree::act::left(s) => {}
        Action::Parent if s.snap.at_root && !s.snap.root_path.is_empty() => {
            then.reopen = goto::up_at_root(s, armed, now);
        }
        Action::Parent => then.entered = s.up(),
        Action::SortSize => sort_by(s, SortKey::Size),
        Action::SortName => sort_by(s, SortKey::Name),
        Action::SortItems => sort_by(s, SortKey::Items),
        Action::SortMtime => sort_by(s, SortKey::Mtime),
        Action::Resort => s.request_sort(),
        Action::ToggleApparent => toggle_size(s, SizeMode::Apparent),
        Action::ToggleReferenced => toggle_size(s, SizeMode::Referenced),
        Action::ToggleHidden => {
            s.hide_hidden = !s.hide_hidden;
            s.request_sort();
        }
        Action::ToggleTree => tree::act::toggle(s),
        Action::ExpandChain => tree::act::chain(s),
        Action::CycleFold => tree::act::cycle_fold(s),
        Action::Rescan => {
            let node = match s.selected_entry() {
                Some(e) if e.is_dir => Some(e.node),
                _ => s.snap.dir_node,
            };
            if let Some(node) = node {
                then.effects.push(Effect::Send(Command::Rescan { node }));
                s.status = Some("rescan requested".into());
            }
        }
        Action::EnterMount => then.effects.extend(enter_mount(s)),
        Action::SkipStalled => {
            if s.snap.stalled > 0 {
                then.effects.push(Effect::Send(Command::AbandonAllStalled));
                s.status = Some("skipping directories that are not responding".into());
            }
        }
        Action::Trash => then.effects = delete(s, Mode::Trash),
        Action::DeletePermanent => then.effects = delete(s, Mode::Permanent),
        Action::Undo => match &s.snap.undo {
            Some(u) if !s.snap.undoing => {
                then.effects
                    .push(Effect::Send(Command::Undo { plan: u.plan }));
                s.status = Some(format!(
                    "putting back {} item{} ({})…",
                    u.items,
                    plural(u.items as u64),
                    human(u.bytes)
                ));
            }
            _ => s.status = Some("nothing to undo".into()),
        },
        Action::Errors => then.effects.push(Effect::ShowErrors),
        Action::Close => {
            if !marks::range::end(s)
                && !filter::clear(s)
                && !type_filter::clear(s)
                && s.current_view() != View::Files
            {
                then = view::switch(s, View::Files);
            }
        }
        Action::ToggleMark => marks::toggle(s),
        Action::ClearMarks => marks::clear(s),
        Action::RangeMark => marks::range::toggle(s),
        Action::InvertMarks => marks::invert(s),
        Action::OpenPath => then.effects.extend(selected_path::open(s)),
        Action::Filter => filter::open(s),
        Action::Details => details::toggle(s),
        Action::ViewFiles => then = view::switch(s, View::Files),
        Action::ViewMap => then = view::switch(s, View::Map),
        Action::ViewCleanup => then = view::switch(s, View::Cleanup),
        Action::ViewDupes => then = view::switch(s, View::Duplicates),
        Action::ViewLargest => then = view::switch(s, View::Largest),
        Action::PrevView => then = view::step(s, -1),
        Action::NextView => then = view::step(s, 1),
        Action::DupesKeep => then = dupes::act(s, dupes::Intent::CycleKeep),
        Action::TickSuggested => match s.current_view() {
            View::Cleanup => then = cleanup::act(s, cleanup::Intent::TickSuggested),
            View::Duplicates => then = dupes::act(s, dupes::Intent::TickSuggested),
            _ => only_in(s, a, &[View::Cleanup, View::Duplicates]),
        },
        Action::CompareAgain if dupes::active(s) => {
            then = dupes::act(s, dupes::Intent::CompareAgain);
        }
        Action::CleanupFilter if cleanup::active(s) => {
            then = cleanup::act(s, cleanup::Intent::CycleFilter);
        }
        Action::CleanupCommand if cleanup::active(s) => then.effects = cleanup::pane::open(s),
        Action::CompareAgain => only_in(s, a, &[View::Duplicates]),
        Action::CleanupFilter | Action::CleanupCommand => only_in(s, a, &[View::Cleanup]),
        Action::TreemapDeeper => treemap::deeper(s, 1),
        Action::TreemapShallower => treemap::deeper(s, -1),
        Action::TreemapColor => treemap::cycle_color(s),
    }
    if a.moves() {
        marks::range::follow(s, before);
    }
    Ran::Done(then)
}

/// A sort key: the key's default direction, or the other direction if it is already in use.
fn sort_by(s: &mut SessionState, key: SortKey) {
    s.sort = if s.sort.key == key {
        SortSpec {
            key,
            desc: !s.sort.desc,
        }
    } else {
        SortSpec::for_key(key)
    };
    s.request_sort();
}

/// Switches to `mode`, or back to disk usage if `mode` is shown already.
fn toggle_size(s: &mut SessionState, mode: SizeMode) {
    s.size_mode = if s.size_mode == mode {
        SizeMode::Disk
    } else {
        mode
    };
    s.request_sort();
}

/// Asks the engine to scan into the selected mount the scan stayed out of.
fn enter_mount(s: &mut SessionState) -> Option<Effect> {
    match s.selected_entry() {
        Some(e) if matches!(e.mark, Mark::OtherFs | Mark::Network) => {
            let name = String::from_utf8_lossy(&e.name).into_owned();
            let node = e.node;
            s.status = Some(format!("scanning {name}/ on its own filesystem…"));
            Some(Effect::Send(Command::Enter { node }))
        }
        _ => {
            s.status = Some("m scans a mount the scan stayed out of; select one first".into());
            None
        }
    }
}

/// The status line after the key of `a`, which acts only in `views`, in another view.
fn only_in(s: &mut SessionState, a: Action, views: &[View]) {
    let names: Vec<String> = (views.iter())
        .map(|v| format!("{} ({})", v.name(), first_key(v.action())))
        .collect();
    s.status = Some(format!("{} works in {}", first_key(a), names.join(" and ")));
}

/// `d` and `D` (in `mode`): in a review view (Cleanup, Duplicates, Largest), the delete
/// confirmation for what is ticked among what the view lists, and only that; with nothing
/// ticked there, or in Duplicates with every copy of a group ticked, the status line says why
/// instead. In the other views, the list's delete.
pub fn delete(s: &mut SessionState, mode: Mode) -> Vec<Effect> {
    let listed = match s.current_view() {
        View::Files | View::Map => return confirm::open(s, mode),
        View::Largest => largest::listed(s),
        View::Cleanup => cleanup::listed(s),
        View::Duplicates => dupes::listed(s),
    };
    let ticked = (s.marks.items().iter())
        .filter_map(|m| m.target.node)
        .any(|n| listed.contains(&n));
    if !ticked {
        s.status = Some(format!(
            "tick items with {} first: {} and {} delete the ticked ones",
            first_key(Action::ToggleMark),
            first_key(Action::Trash),
            first_key(Action::DeletePermanent)
        ));
        return Vec::new();
    }
    if s.current_view() == View::Duplicates
        && let Some(name) = dupes::whole_group_ticked(s)
    {
        s.status = Some(format!(
            "every copy of {name} is ticked: untick one to keep it ({} unticks)",
            first_key(Action::ToggleMark)
        ));
        return Vec::new();
    }
    confirm::open_only(s, mode, &listed)
}

#[cfg(test)]
mod tests;
