//! Remembers each window's size and position inside the bounded preference document.
use crate::preferences::{Preferences, WindowGeometry};

pub(super) fn remember_geometry(
    preferences: &mut Preferences,
    key: String,
    mut geometry: WindowGeometry,
) {
    if geometry.maximized
        && let Some(previous) = preferences.windows.get(&key)
    {
        geometry.size = previous.size;
        geometry.position = previous.position;
    }
    if preferences.windows.get(&key) == Some(&geometry) {
        return;
    }
    let previous = preferences.windows.clone();
    if !preferences.windows.contains_key(&key)
        && preferences.windows.len() >= 64
        && let Some(oldest) = preferences
            .windows
            .keys()
            .find(|k| *k != "home" && *k != "shorts")
            .cloned()
    {
        preferences.windows.remove(&oldest);
    }
    preferences.windows.insert(key.clone(), geometry);
    // Geometry is presentation history: trim it before allowing a resize to
    // make the bounded preference document unreadable or block later edits.
    while preferences.validate().is_err() {
        let evict = preferences
            .windows
            .keys()
            .find(|k| **k != key && *k != "home" && *k != "shorts")
            .or_else(|| preferences.windows.keys().find(|k| **k != key))
            .cloned();
        if let Some(evict) = evict {
            preferences.windows.remove(&evict);
        } else {
            preferences.windows = previous;
            break;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn geometry_history_yields_to_the_preference_byte_bound_and_preserves_normal_size() {
        let mut preferences = Preferences::default();
        for index in 0..128 {
            preferences.sections.insert(format!("{index:064}"), false);
        }
        for index in 0..64 {
            let key = format!("{index:064}");
            preferences.windows.insert(key.clone(), Default::default());
            if preferences.validate().is_err() {
                preferences.windows.remove(&key);
                break;
            }
        }
        let sections = preferences.sections.clone();
        let before = preferences.windows.len();
        let key = "x".repeat(64);
        let geometry = crate::preferences::WindowGeometry {
            size: [12345.678, 12345.678],
            position: Some([-12345.678, -23_456.79]),
            maximized: false,
        };
        remember_geometry(&mut preferences, key.clone(), geometry.clone());
        assert!(preferences.validate().is_ok());
        assert_eq!(preferences.sections, sections);
        assert_eq!(preferences.windows.get(&key), Some(&geometry));
        assert!(
            preferences.windows.len() <= before,
            "history eviction must create room for the new geometry"
        );
        let maximized = crate::preferences::WindowGeometry {
            size: [16000.0, 16000.0],
            position: Some([0.0, 0.0]),
            maximized: true,
        };
        remember_geometry(&mut preferences, key.clone(), maximized);
        let restored = preferences.windows.get(&key).unwrap();
        assert_eq!(restored.size, geometry.size);
        assert_eq!(restored.position, geometry.position);
        assert!(restored.maximized);
    }
}
