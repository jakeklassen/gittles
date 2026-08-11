//! Filtering stars by a query string.
//!
//! This lives in core rather than beside the browse view: it is about `Star`,
//! not about rendering, and a future TUI would need exactly the same rules.

use crate::github::Star;

/// Case-insensitive substring match across the three fields the CLI searched:
/// full name, description, and language.
pub fn matches(star: &Star, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let query = query.to_lowercase();
    star.full_name.to_lowercase().contains(&query)
        || star.description.to_lowercase().contains(&query)
        || star.language.to_lowercase().contains(&query)
}

/// The indices of `stars` that match, in order. Indices rather than clones so
/// the caller can keep one copy of the list.
pub fn filter(stars: &[Star], query: &str) -> Vec<usize> {
    stars
        .iter()
        .enumerate()
        .filter(|(_, star)| matches(star, query))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn star(full_name: &str, description: &str, language: &str) -> Star {
        Star {
            id: 1,
            name: full_name.split('/').next_back().unwrap_or_default().into(),
            full_name: full_name.into(),
            description: description.into(),
            url: String::new(),
            language: language.into(),
            stargazers_count: 0,
            forks_count: 0,
            open_issues_count: 0,
            pushed_at: String::new(),
            starred_at: String::new(),
        }
    }

    fn corpus() -> Vec<Star> {
        vec![
            star("zed-industries/zed", "Code at the speed of thought", "Rust"),
            star("longbridge/gpui-component", "UI components", "Rust"),
            star(
                "microsoft/TypeScript",
                "A superset of JavaScript",
                "TypeScript",
            ),
        ]
    }

    #[test]
    fn an_empty_query_matches_everything() {
        let stars = corpus();
        assert_eq!(filter(&stars, ""), vec![0, 1, 2]);
    }

    #[test]
    fn matches_on_full_name() {
        let stars = corpus();
        assert_eq!(filter(&stars, "longbridge"), vec![1]);
    }

    #[test]
    fn matches_on_description() {
        let stars = corpus();
        assert_eq!(filter(&stars, "speed of thought"), vec![0]);
    }

    #[test]
    fn matches_on_language() {
        let stars = corpus();
        assert_eq!(filter(&stars, "rust"), vec![0, 1]);
    }

    #[test]
    fn is_case_insensitive_in_both_directions() {
        let stars = corpus();
        assert_eq!(filter(&stars, "TYPESCRIPT"), vec![2]);
        // Query lowercase, haystack mixed case.
        assert_eq!(filter(&stars, "javascript"), vec![2]);
    }

    #[test]
    fn a_query_matching_nothing_yields_nothing() {
        let stars = corpus();
        assert!(filter(&stars, "cobol").is_empty());
    }
}
