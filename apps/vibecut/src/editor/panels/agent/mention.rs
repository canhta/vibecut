//! Lets `@` in the chat input name a timeline item or a media file; the model sees its id.
use crate::i18n::{Locale, Text, text};
use project::{ItemParams, Project};

/// What a candidate names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Item,
    Media,
}

/// One thing the creator may pick: the name shown in the list and typed into the input,
/// and the id text the model reads in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Mention {
    pub(super) target: Target,
    pub(super) label: String,
    pub(super) token: String,
}

/// The ids' own JSON spelling, so the model reads the same value `timeline_read` shows.
fn id_text(id: &impl serde::Serialize) -> String {
    serde_json::to_string(id).unwrap_or_default()
}

fn unique(taken: &mut Vec<String>, name: String) -> String {
    let same = taken.iter().filter(|known| **known == name).count();
    taken.push(name.clone());
    if same == 0 {
        name
    } else {
        format!("{name} ({})", same + 1)
    }
}

/// Every clip, text and caption of the active scene, then every media file, named for the
/// creator. Equal names get a number so each label picks exactly one thing.
pub(super) fn candidates(project: &Project, locale: Locale) -> Vec<Mention> {
    let mut taken = Vec::new();
    let mut out = Vec::new();
    for track in project.data().scene().timeline.iter() {
        for item in &track.items {
            let name = match (&item.content, &item.source_asset) {
                (_, Some(asset)) => project
                    .data()
                    .assets
                    .iter()
                    .find(|known| &known.id() == asset)
                    .and_then(|known| known.path().file_name())
                    .map(|name| name.to_string_lossy().into_owned()),
                (ItemParams::Text(params), None) => {
                    let value = params.content.as_str();
                    Some(if value.is_empty() {
                        text(locale, Text::TextItem).to_owned()
                    } else {
                        value.chars().take(24).collect()
                    })
                }
                (ItemParams::Caption(_), None) => Some(text(locale, Text::Caption).to_owned()),
                _ => None,
            };
            if let Some(name) = name {
                out.push(Mention {
                    target: Target::Item,
                    label: unique(&mut taken, name.replace('\n', " ")),
                    token: format!("[item {}]", id_text(&item.id)),
                });
            }
        }
    }
    for asset in &project.data().assets {
        if let Some(name) = asset.path().file_name() {
            out.push(Mention {
                target: Target::Media,
                label: unique(&mut taken, name.to_string_lossy().into_owned()),
                token: format!("[media {}]", id_text(&asset.id())),
            });
        }
    }
    out
}

/// The word being typed after the last `@`, when it is still a search and not a picked label.
pub(super) fn active_query<'a>(input: &'a str, picked: &[Mention]) -> Option<&'a str> {
    let at = input.rfind('@')?;
    let tail = input.get(at + 1..)?;
    if tail.contains('\n') || tail.chars().count() > 40 {
        return None;
    }
    if picked
        .iter()
        .any(|mention| tail.starts_with(&mention.label))
    {
        return None;
    }
    Some(tail)
}

/// The picked mentions whose label matches the search, at most `limit`.
pub(super) fn matching<'a>(
    all: &'a [Mention],
    query: &str,
    limit: usize,
) -> impl Iterator<Item = &'a Mention> {
    let needle = query.trim().to_lowercase();
    all.iter()
        .filter(move |mention| needle.is_empty() || mention.label.to_lowercase().contains(&needle))
        .take(limit)
}

/// Replace the search after the last `@` with the picked label and a space.
pub(super) fn insert(input: &mut String, picked: &Mention) {
    if let Some(at) = input.rfind('@') {
        input.truncate(at);
        input.push('@');
        input.push_str(&picked.label);
        input.push(' ');
    }
}

/// The message the model reads: every picked `@label` replaced by its id text.
pub(super) fn resolve(input: &str, picked: &[Mention]) -> String {
    let mut out = input.to_owned();
    // Longest labels first, so a label that contains another is replaced whole.
    let mut order: Vec<&Mention> = picked.iter().collect();
    order.sort_by_key(|mention| std::cmp::Reverse(mention.label.len()));
    for mention in order {
        out = out.replace(&format!("@{}", mention.label), &mention.token);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn mention(label: &str, token: &str) -> Mention {
        Mention {
            target: Target::Item,
            label: label.into(),
            token: token.into(),
        }
    }

    #[test]
    fn a_picked_label_reaches_the_model_as_its_id() {
        let picked = [
            mention("intro.mp4", "[item 4]"),
            mention("intro", "[media 9]"),
        ];
        assert_eq!(
            resolve("trim @intro.mp4 and mute @intro", &picked),
            "trim [item 4] and mute [media 9]"
        );
    }

    #[test]
    fn the_search_ends_once_a_label_is_picked() {
        let picked = [mention("my clip.mp4", "[item 1]")];
        assert_eq!(active_query("cut @my c", &picked), Some("my c"));
        assert_eq!(active_query("cut @my clip.mp4 ", &picked), None);
        assert_eq!(active_query("no mention", &picked), None);
        assert_eq!(active_query("a @b\nc", &picked), None);
    }

    #[test]
    fn picking_replaces_the_search_text() {
        let mut input = "cut @my c".to_owned();
        insert(&mut input, &mention("my clip.mp4", "[item 1]"));
        assert_eq!(input, "cut @my clip.mp4 ");
    }

    #[test]
    fn equal_names_get_numbers() {
        let mut taken = Vec::new();
        assert_eq!(unique(&mut taken, "a.mp4".into()), "a.mp4");
        assert_eq!(unique(&mut taken, "a.mp4".into()), "a.mp4 (2)");
        assert_eq!(unique(&mut taken, "a.mp4".into()), "a.mp4 (3)");
    }
}
