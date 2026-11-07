//! The projects page's persisted view, sort key and direction.

use serde::{Deserialize, Serialize};

/// Projects page presentation state, persisted with the workspace shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectsView {
    #[default]
    Grid,
    List,
}

/// Projects page sort keys. `Modified` is part of the sort menu and info dialog,
/// never displayed on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectsSortKey {
    #[default]
    Created,
    Modified,
    Name,
    Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectsPreferences {
    pub view: ProjectsView,
    pub sort: ProjectsSortKey,
    pub ascending: bool,
}
impl Default for ProjectsPreferences {
    fn default() -> Self {
        Self {
            view: ProjectsView::Grid,
            sort: ProjectsSortKey::Created,
            ascending: false,
        }
    }
}
