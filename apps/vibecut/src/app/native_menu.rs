//! The macOS menu bar: the registry's `MENUS` as native menus that queue the same actions.

use crate::actions::KeybindingPreferences;
use crate::i18n::Locale;

/// A menu row the workspace must run, queued by the native menu in click order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Action(&'static str),
    Home,
}

pub(super) use mac::Menu;

mod mac {
    use super::{Entry, KeybindingPreferences, Locale};
    use crate::actions::{self, Chord, MENUS, MenuItem};
    use crate::i18n::{self, Text};
    use muda::{MenuEvent, PredefinedMenuItem, accelerator::Accelerator};

    const HOME: &str = "window.home.show";

    #[derive(Default)]
    pub(in crate::app) struct Menu {
        /// The bar in use and what it was built from; a new language or chord rebuilds it.
        installed: Option<(muda::Menu, Vec<String>)>,
        failed: Vec<String>,
    }

    impl Menu {
        /// Build the bar on first use and again when the language or a shown chord changes.
        pub(in crate::app) fn sync(&mut self, locale: Locale, keys: &KeybindingPreferences) {
            // AppKit menus exist on the main thread only; kittest runs elsewhere.
            if objc2::MainThreadMarker::new().is_none() {
                return;
            }
            let mut signature = vec![format!("{locale:?}")];
            signature.extend(
                MENUS
                    .iter()
                    .flat_map(|menu| menu.items)
                    .filter_map(|item| match item {
                        MenuItem::Action(id) => Some(chord(keys, id).map(|c| c.canonical())),
                        _ => None,
                    })
                    .map(Option::unwrap_or_default),
            );
            if self
                .installed
                .as_ref()
                .is_some_and(|(_, shown)| *shown == signature)
                || self.failed == signature
            {
                return;
            }
            match build(locale, keys) {
                Ok(menu) => self.installed = Some((menu, signature)),
                Err(error) => {
                    // Retry only when the inputs change, not every frame.
                    tracing::warn!(category = "menu_bar_unavailable", %error, "Menu bar unavailable");
                    self.failed = signature;
                }
            }
        }

        /// The rows clicked since the last call.
        pub(in crate::app) fn take(&mut self) -> Vec<Entry> {
            let mut clicked = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = event.id.as_ref();
                if id == HOME {
                    clicked.push(Entry::Home);
                } else if let Some(action) = actions::action(id) {
                    clicked.push(Entry::Action(action.id));
                }
            }
            clicked
        }
    }

    /// The action's first effective chord.
    fn chord(keys: &KeybindingPreferences, id: &str) -> Option<Chord> {
        actions::action(id)
            .and_then(|spec| keys.effective_chords(spec).into_iter().next())
            .and_then(|text| Chord::parse(&text).ok())
    }

    /// `Cmd+Alt+Shift+K` as the accelerator it shows and matches; keys with no single
    /// character (arrows, Escape) show none.
    fn accelerator(chord: Chord) -> Option<Accelerator> {
        let key = chord.key.symbol_or_name();
        if key.chars().count() != 1 {
            return None;
        }
        let mut text = String::new();
        if chord.mods.command {
            text.push_str("Cmd+");
        }
        if chord.mods.alt {
            text.push_str("Alt+");
        }
        if chord.mods.shift {
            text.push_str("Shift+");
        }
        text.push_str(&key.to_uppercase());
        text.parse().ok()
    }

    fn build(locale: Locale, keys: &KeybindingPreferences) -> muda::Result<muda::Menu> {
        let text = |t: Text| i18n::text(locale, t);
        let name = text(Text::AppName);
        let bar = muda::Menu::new();
        let app = muda::Submenu::new(name, true);
        app.append_items(&[
            &PredefinedMenuItem::about(Some(&format!("About {name}")), None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(&format!("Hide {name}"))),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some(&format!("Quit {name}"))),
        ])?;
        bar.append(&app)?;
        for menu in MENUS {
            let submenu = muda::Submenu::new(text(menu.title), true);
            for item in menu.items {
                match item {
                    MenuItem::Separator => submenu.append(&PredefinedMenuItem::separator())?,
                    MenuItem::Minimize => submenu.append(&PredefinedMenuItem::minimize(Some(
                        text(Text::MenuMinimize),
                    )))?,
                    MenuItem::Zoom => {
                        submenu.append(&PredefinedMenuItem::maximize(Some(text(Text::MenuZoom))))?
                    }
                    MenuItem::Home => submenu.append(&muda::MenuItem::with_id(
                        HOME,
                        text(Text::Home),
                        true,
                        None,
                    ))?,
                    MenuItem::Action(id) => {
                        let Some(spec) = actions::action(id) else {
                            continue;
                        };
                        submenu.append(&muda::MenuItem::with_id(
                            *id,
                            text(spec.label),
                            true,
                            chord(keys, id).and_then(accelerator),
                        ))?;
                    }
                }
            }
            bar.append(&submenu)?;
            match menu.title {
                Text::Windows => submenu.set_as_windows_menu_for_nsapp(),
                Text::Help => submenu.set_as_help_menu_for_nsapp(),
                _ => {}
            }
        }
        bar.init_for_nsapp();
        Ok(bar)
    }
}
