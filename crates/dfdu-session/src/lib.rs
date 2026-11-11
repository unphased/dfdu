//! What a front end shows besides drawing and input, with no terminal or window types. The
//! session driver ([`Session`], `session`) holds the engine, the state of a browser and the
//! jobs beside them, reads the model when due and carries out the actions (`act`).
//!
//! The state of a browser (`state`, [`SessionState`]) with the view shown (`view`), its marks
//! and the marks pane (`marks`) and name filter (`filter`), the directory snapshot copied out of
//! the engine model (`snapshot`), the directories tree mode expands (`tree`), the order of the
//! rows (`order`) and the fuzzy matching of what is typed (`fuzzy`). What the features read
//! from the model besides the snapshot (`extras`), and what is left to carry out after a key
//! (`effect`). From outside the UI: the scan history (`history_link`) with what grew since the
//! last scan (`growth`), the scan cache (`cache_link`), the mounts and links the scan did not
//! enter (`outside`), the selected path (`selected_path`) and the file manager that shows it
//! (`file_manager`). What is opened and how it starts (`startup`), and scans on another host
//! through ssh (`remote`). The views: the treemap's layout, depth and lower levels (`treemap`),
//! the cleanup view (`cleanup`), the duplicates comparison with its keep rules and view
//! (`dupes`), and the largest files below a directory (`largest`). The actions with their keys
//! and labels (`actions`), the delete confirmation (`confirm`), the details of an entry
//! (`details`), the debug counters (`debug`), the list of what could not be read
//! (`error_list`), the file type filter (`type_filter`), mark by pattern (`mark_pattern`), go to
//! folder and up past the scan root (`goto`), the text for numbers (`format`), the theme's
//! tokens, built-in themes and theme files (`palette`), and the single-line text input (`line`).

pub mod act;
pub mod actions;
pub mod cache_link;
pub mod cleanup;
pub mod confirm;
pub mod debug;
pub mod details;
pub mod dupes;
pub mod effect;
pub mod elevate;
pub mod error_list;
pub mod extras;
pub mod file_manager;
pub mod filter;
pub mod format;
pub mod fuzzy;
pub mod goto;
pub mod growth;
pub mod history_link;
pub mod largest;
pub mod line;
pub mod mark_pattern;
pub mod marks;
pub mod match_index;
pub mod order;
pub mod outside;
pub mod palette;
pub mod remote;
pub mod selected_path;
pub mod session;
pub mod snapshot;
pub mod startup;
pub mod state;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
pub mod tree;
pub mod treemap;
pub mod type_filter;
pub mod view;

pub use session::{Acted, FrontEnd, Session, SessionOptions, Tick};
pub use state::SessionState;
