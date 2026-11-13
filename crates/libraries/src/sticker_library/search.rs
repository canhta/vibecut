//! Case-insensitive search over the flag catalog, in catalog order.
use super::flags::{COUNTRIES, Country, REGION_GROUPS, REGIONS};

/// Results returned when a caller gives no limit.
pub const DEFAULT_SEARCH_LIMIT: usize = 100;

/// One flag row a search returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagMatch {
    /// Upper-case ISO or regional code; the sticker id value.
    pub code: &'static str,
    pub name: &'static str,
    pub region: Option<&'static str>,
    pub languages: &'static [&'static str],
    pub flag_colors: &'static [&'static str],
}

/// A page of flag matches with the total before the limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagPage {
    pub items: Vec<FlagMatch>,
    pub total: usize,
    pub has_more: bool,
}

/// Searches flags by name, code, region or continent keyword. An empty query matches everything.
///
/// A query equal to a region id, a region alias or a continent keyword filters by region;
/// any other query is a substring match on the country name or code.
pub fn search_flags(query: &str, limit: usize) -> FlagPage {
    let query = query.trim().to_lowercase();
    let regions = resolve_regions(&query);
    let matches: Vec<&Country> = COUNTRIES
        .iter()
        .filter(|country| {
            if query.is_empty() {
                return true;
            }
            match &regions {
                Some(ids) => country.region.is_some_and(|region| ids.contains(&region)),
                None => {
                    country.name.to_lowercase().contains(&query)
                        || country.code.to_lowercase().contains(&query)
                }
            }
        })
        .collect();
    let total = matches.len();
    let items = matches
        .into_iter()
        .take(limit)
        .map(|country| FlagMatch {
            code: country.code,
            name: country.name,
            region: country.region,
            languages: country.languages,
            flag_colors: country.flag_colors,
        })
        .collect::<Vec<_>>();
    let has_more = total > items.len();
    FlagPage {
        items,
        total,
        has_more,
    }
}

fn resolve_regions(query: &str) -> Option<Vec<&'static str>> {
    if query.is_empty() {
        return None;
    }
    if let Some((_, ids)) = REGION_GROUPS.iter().find(|(key, _)| *key == query) {
        return Some(ids.to_vec());
    }
    let ids: Vec<&'static str> = REGIONS
        .iter()
        .filter(|region| {
            region.id.to_lowercase() == query || region.aliases.iter().any(|alias| *alias == query)
        })
        .map(|region| region.id)
        .collect();
    (!ids.is_empty()).then_some(ids)
}
