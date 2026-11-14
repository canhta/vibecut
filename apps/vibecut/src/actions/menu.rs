//! The native menu bar's layout: which registry action sits in which menu.

use crate::i18n::Text;
use crate::input::Chord;

/// One row of a menu. An action row runs the registry action with that id and shows
/// its chord; the rest are window commands with no chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Action(&'static str),
    Separator,
    Minimize,
    Zoom,
    /// Bring Home forward without leaving the project (`window.home` leaves it).
    Home,
}

#[derive(Clone, Copy, Debug)]
pub struct Menu {
    pub title: Text,
    pub items: &'static [MenuItem],
}

/// The chord a menu row shows and answers to. Only a Command chord: a menu equivalent takes
/// the key before a focused text field, so a plain or Option chord would steal typing. The
/// row's other chords still work in the window through the keymap.
pub fn menu_equivalent(chord: Chord) -> Option<Chord> {
    chord.mods.command.then_some(chord)
}

use MenuItem::{Action, Separator};

/// File, Edit, View, Window and Help, after the app menu the platform provides.
pub const MENUS: &[Menu] = &[
    Menu {
        title: Text::MenuFile,
        items: &[
            Action("file.import"),
            Action("file.export"),
            Separator,
            Action("project.save"),
            Action("project.save_as"),
            Action("project.rename"),
            Separator,
            Action("project.exit"),
        ],
    },
    Menu {
        title: Text::MenuEdit,
        items: &[
            Action("edit.undo"),
            Action("edit.redo"),
            Separator,
            Action("edit.cut"),
            Action("edit.copy"),
            Action("edit.paste"),
            Separator,
            Action("timeline.split"),
        ],
    },
    Menu {
        title: Text::MenuView,
        items: &[
            Action("preview.zoom.fit"),
            Action("preview.zoom.in"),
            Action("preview.zoom.out"),
            Action("preview.fullscreen"),
            Separator,
            Action("layout.reset"),
            Action("appearance.theme"),
            Action("preferences.open"),
        ],
    },
    Menu {
        title: Text::Windows,
        items: &[
            MenuItem::Minimize,
            MenuItem::Zoom,
            Separator,
            MenuItem::Home,
        ],
    },
    Menu {
        title: Text::Help,
        items: &[
            Action("shortcuts.open"),
            Action("app.help"),
            Action("app.feedback"),
        ],
    },
];
