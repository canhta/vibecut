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
    /// The app menu's About row runs the registry action of the same id.
    const ABOUT: &str = "app.about";

    #[derive(Default)]
    pub(in crate::app) struct Menu {
        /// The bar in use and what it was built from; a new language or chord rebuilds it.
        installed: Option<(muda::Menu, Vec<String>)>,
        failed: Vec<String>,
        /// Clicks queued by the bar's handler, which wakes the loop: a click is no window event,
        /// so an idle loop would otherwise run it only at the next input.
        clicks: Option<std::sync::mpsc::Receiver<MenuEvent>>,
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
            if self.clicks.is_none() {
                let send = self.open_clicks();
                MenuEvent::set_event_handler(Some(move |event| {
                    let _ = send.send(event);
                }));
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

        /// The queue clicks arrive on; its sender wakes the loop bound to the calling thread.
        pub(super) fn open_clicks(&mut self) -> wake::Sender<MenuEvent> {
            let (send, receive) = wake::channel();
            self.clicks = Some(receive);
            send
        }

        /// The rows clicked since the last call.
        pub(in crate::app) fn take(&mut self) -> Vec<Entry> {
            let mut clicked = Vec::new();
            while let Some(Ok(event)) = self.clicks.as_ref().map(|clicks| clicks.try_recv()) {
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

    /// The action's first effective chord, when it may be a menu equivalent.
    fn chord(keys: &KeybindingPreferences, id: &str) -> Option<Chord> {
        actions::action(id)
            .and_then(|spec| keys.effective_chords(spec).into_iter().next())
            .and_then(|text| Chord::parse(&text).ok())
            .and_then(actions::menu_equivalent)
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
            // Our own About dialog replaces the platform panel: it lists licences and terms.
            &muda::MenuItem::with_id(
                ABOUT,
                i18n::format(locale, Text::AboutMenuItem, &[("name", name)]),
                true,
                None,
            ),
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn a_menu_click_wakes_the_loop_and_queues_its_row() {
        let woken = Arc::new(AtomicUsize::new(0));
        let wake = wake::Wake::new();
        let count = woken.clone();
        wake.set(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        wake.bind();
        let mut menu = mac::Menu::default();
        let click = menu.open_clicks();
        click
            .send(muda::MenuEvent {
                id: "window.home.show".into(),
            })
            .unwrap();
        assert!(woken.load(Ordering::SeqCst) > 0);
        assert_eq!(menu.take(), [Entry::Home]);
    }
}
