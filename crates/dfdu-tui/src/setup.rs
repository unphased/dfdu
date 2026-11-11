//! The browser's settings from the command line, and what [`crate::run`] does with them before
//! and after the browser: the theme resolved for the terminal, mouse capture on and off.

use std::ffi::OsString;
use std::io;

use ratatui::crossterm::event::DisableMouseCapture;
use ratatui::crossterm::{cursor, execute};

use crate::theme::{self, Bars, BuiltinTheme, ColorLevel, Rgb, Theme, ThemeFile};

/// Which theme to use.
#[derive(Clone, Debug, Default)]
pub enum ThemeChoice {
    /// Chosen from the colour level and the terminal's background colour.
    #[default]
    Auto,
    /// A built-in theme.
    Builtin(BuiltinTheme),
    /// A theme file, already parsed.
    File(ThemeFile),
}

impl ThemeChoice {
    /// `auto` or a built-in theme; `None` for any other name.
    #[must_use]
    pub fn builtin(name: &str) -> Option<ThemeChoice> {
        if name == "auto" {
            return Some(ThemeChoice::Auto);
        }
        BuiltinTheme::parse(name).map(ThemeChoice::Builtin)
    }
}

/// How the browser looks and which input it takes, from the command line.
#[derive(Clone, Debug)]
pub struct UiSettings {
    pub theme: ThemeChoice,
    /// The colour level asked for; `None` detects it from the environment.
    pub color: Option<ColorLevel>,
    /// How bars are drawn (`--bars`); `None` leaves it to the theme.
    pub bars: Option<Bars>,
    /// Capture the mouse, so clicks and the wheel reach the browser.
    pub mouse: bool,
    /// Start with the debug panel open, and print its numbers after the browser exits.
    pub debug_stats: bool,
    /// Start with this file type filter (`t`).
    pub types: dfdu_tree::filetype::TypeFilter,
}

impl Default for UiSettings {
    fn default() -> Self {
        UiSettings {
            theme: ThemeChoice::Auto,
            color: None,
            bars: None,
            mouse: true,
            debug_stats: false,
            types: dfdu_tree::filetype::TypeFilter::default(),
        }
    }
}

/// The theme `settings` ask for and the terminal it was resolved for.
#[derive(Debug)]
pub(crate) struct Colors {
    pub theme: Theme,
    /// The colour level detected from the environment; `mono` itself has none.
    pub level: ColorLevel,
    /// The terminal's background colour, when it was queried.
    pub background: Option<Rgb>,
    /// Colour was asked for although `NO_COLOR` is set.
    pub force_color: bool,
}

/// The theme `settings` ask for, resolved for the colour level detected from `env`. `query`
/// asks the terminal for its background colour; it is called only when the answer can change
/// the theme.
pub(crate) fn resolve_theme(
    settings: &UiSettings,
    env: impl Fn(&str) -> Option<OsString>,
    query: impl FnOnce() -> Option<Rgb>,
) -> Colors {
    let detected = theme::detect_color_level(settings.color, &env);
    let explicit = match &settings.theme {
        ThemeChoice::Auto => None,
        ThemeChoice::Builtin(b) => Some(*b),
        ThemeChoice::File(file) => Some(file.base()),
    };
    let background = if theme::should_query_background(explicit, detected.level, &env) {
        query()
    } else {
        None
    };
    let mut theme = if let ThemeChoice::File(file) = &settings.theme {
        Theme::from_file(file, detected.level, background)
    } else {
        let b = theme::choose_theme(explicit, detected.level, background);
        Theme::builtin(b, detected.level, background)
    };
    if let Some(bars) = settings.bars {
        theme.bars = bars;
    }
    Colors {
        theme,
        level: detected.level,
        background,
        force_color: detected.overrides_no_color,
    }
}

/// Leaves the browser's terminal state: mouse capture off (if it was turned on), then raw mode
/// and the alternate screen, then the cursor shown. The terminal's Drop shows the cursor too,
/// but only after the engine has shut down.
pub(crate) fn restore_terminal(mouse: bool) {
    crate::preview::runtime::delete_terminal_image();
    if mouse {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    let _ = execute!(io::stdout(), cursor::Show);
}

/// Takes the terminal back after [`restore_terminal`]: raw mode, the alternate screen and mouse
/// capture (`mouse`) again, and the screen drawn in full with the next frame.
pub(crate) fn resume_terminal(
    terminal: &mut ratatui::DefaultTerminal,
    mouse: bool,
) -> io::Result<()> {
    ratatui::crossterm::terminal::enable_raw_mode()?;
    execute!(
        io::stdout(),
        ratatui::crossterm::terminal::EnterAlternateScreen
    )?;
    if mouse {
        execute!(io::stdout(), ratatui::crossterm::event::EnableMouseCapture)?;
    }
    terminal.hide_cursor()?;
    terminal.clear()
}

/// Restores the terminal ([`restore_terminal`]) when dropped, so that every way out of the
/// browser leaves it as it was. On a panic the hook has restored it already.
pub(crate) struct Restore {
    /// Mouse capture was turned on, or tried.
    pub(crate) mouse: bool,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            restore_terminal(self.mouse);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        move |k| {
            vars.iter()
                .find(|(name, _)| name == k)
                .map(|(_, v)| v.into())
        }
    }

    #[test]
    fn theme_names_are_auto_or_built_in() {
        assert!(matches!(
            ThemeChoice::builtin("auto"),
            Some(ThemeChoice::Auto)
        ));
        for b in BuiltinTheme::ALL {
            assert!(
                matches!(ThemeChoice::builtin(b.name()), Some(ThemeChoice::Builtin(n)) if n == b)
            );
        }
        assert!(ThemeChoice::builtin("solarized").is_none());
    }

    #[test]
    fn auto_theme_follows_the_queried_background() {
        let settings = UiSettings::default();
        let term = env(&[("TERM", "xterm-256color")]);
        let dark = resolve_theme(&settings, &term, || Some((0x1e, 0x1e, 0x2e))).theme;
        let light = resolve_theme(&settings, &term, || Some((0xfd, 0xf6, 0xe3))).theme;
        let unknown = resolve_theme(&settings, &term, || None).theme;
        assert_eq!(
            (dark.name(), light.name(), unknown.name()),
            ("dark", "light", "ansi")
        );
        assert_eq!(dark.level(), ColorLevel::Ansi256);
    }

    #[test]
    fn explicit_theme_skips_the_background_query() {
        let asked = Cell::new(false);
        let query = || {
            asked.set(true);
            Some((0, 0, 0))
        };
        let settings = UiSettings {
            theme: ThemeChoice::Builtin(BuiltinTheme::Light),
            ..UiSettings::default()
        };
        let theme = resolve_theme(&settings, env(&[("COLORTERM", "truecolor")]), query).theme;
        assert_eq!(
            (theme.name(), theme.level()),
            ("light", ColorLevel::TrueColor)
        );
        let file = ThemeFile::parse("name = \"mine\"\nbase = \"light\"\n").expect("theme file");
        let settings = UiSettings {
            theme: ThemeChoice::File(file),
            ..UiSettings::default()
        };
        let theme = resolve_theme(&settings, env(&[("COLORTERM", "truecolor")]), query).theme;
        assert_eq!(theme.name(), "mine");
        assert!(!asked.get());
    }

    #[test]
    fn color_flag_wins_over_no_color() {
        let no_color = env(&[("NO_COLOR", "1"), ("TERM", "xterm-256color")]);
        let colors = resolve_theme(&UiSettings::default(), &no_color, || None);
        assert_eq!(
            (colors.theme.name(), colors.level, colors.force_color),
            ("mono", ColorLevel::None, false)
        );
        let settings = UiSettings {
            color: Some(ColorLevel::Ansi16),
            ..UiSettings::default()
        };
        let colors = resolve_theme(&settings, &no_color, || None);
        assert_eq!(
            (
                colors.theme.name(),
                colors.theme.level(),
                colors.force_color
            ),
            ("ansi", ColorLevel::Ansi16, true)
        );
    }

    #[test]
    fn mono_keeps_the_terminal_color_level() {
        let settings = UiSettings {
            theme: ThemeChoice::Builtin(BuiltinTheme::Mono),
            ..UiSettings::default()
        };
        let colors = resolve_theme(&settings, env(&[("COLORTERM", "truecolor")]), || None);
        assert_eq!(
            (colors.theme.level(), colors.level),
            (ColorLevel::None, ColorLevel::TrueColor)
        );
    }

    #[test]
    fn resolve_theme_keeps_the_queried_background() {
        let term = env(&[("TERM", "xterm-256color")]);
        let colors = resolve_theme(&UiSettings::default(), &term, || Some((0, 0, 0x20)));
        assert_eq!(colors.background, Some((0, 0, 0x20)));
    }
}
