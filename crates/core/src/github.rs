//! The slice of the GitHub REST API that gittles needs: who you are, what you
//! starred, and unstarring.

use anyhow::Result;
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://api.github.com";
pub(crate) const USER_AGENT: &str = "gittles";
const ACCEPT_JSON: &str = "application/vnd.github+json";
/// Only this media type puts `starred_at` on the listing.
const ACCEPT_STAR_JSON: &str = "application/vnd.github.star+json";
const PER_PAGE: usize = 100;

/// A starred repository, flattened out of the API's `{starred_at, repo}` envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Star {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub url: String,
    pub language: String,
    pub stargazers_count: u64,
    pub forks_count: u64,
    pub open_issues_count: u64,
    pub pushed_at: String,
    pub starred_at: String,
}

/// The API shapes. Field names already match the wire format, so these carry no
/// rename plumbing — but `html_url` and the nullable columns still need mapping,
/// which is what [`Star::from`] below is for.
#[derive(Debug, Deserialize)]
struct ApiRepo {
    id: u64,
    name: String,
    full_name: String,
    description: Option<String>,
    html_url: String,
    language: Option<String>,
    stargazers_count: u64,
    forks_count: u64,
    open_issues_count: u64,
    pushed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiStarred {
    starred_at: String,
    repo: ApiRepo,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    login: String,
}

impl From<ApiStarred> for Star {
    fn from(entry: ApiStarred) -> Self {
        let repo = entry.repo;
        Star {
            id: repo.id,
            name: repo.name,
            full_name: repo.full_name,
            // `null` collapses to "" so the UI never has to special-case it.
            description: repo.description.unwrap_or_default(),
            url: repo.html_url,
            language: repo.language.unwrap_or_default(),
            stargazers_count: repo.stargazers_count,
            forks_count: repo.forks_count,
            open_issues_count: repo.open_issues_count,
            pushed_at: repo.pushed_at.unwrap_or_default(),
            starred_at: entry.starred_at,
        }
    }
}

/// Collapse a response body to one line and clip it. These end up on a single
/// status line, so a multi-line error body would wreck the layout.
pub(crate) fn fail(what: &str, status: u16, body: &str) -> anyhow::Error {
    let one_line = body.replace('\n', " ").replace('\r', "");
    let clipped: String = one_line.chars().take(120).collect();
    anyhow::anyhow!("{what} failed (HTTP {status}): {clipped}")
}

/// An authenticated GitHub client.
#[derive(Debug, Clone)]
pub struct GitHub {
    http: reqwest::Client,
    token: String,
}

impl GitHub {
    pub fn new(token: impl Into<String>) -> Result<Self> {
        Ok(GitHub {
            http: reqwest::Client::builder().user_agent(USER_AGENT).build()?,
            token: token.into(),
        })
    }

    async fn get(&self, path: &str, accept: &str) -> Result<(u16, String)> {
        let response = self
            .http
            .get(format!("{API_BASE}{path}"))
            .header("accept", accept)
            .bearer_auth(&self.token)
            .send()
            .await?;

        let status = response.status().as_u16();
        Ok((status, response.text().await?))
    }

    /// `GET /user`
    pub async fn username(&self) -> Result<String> {
        let (status, body) = self.get("/user", ACCEPT_JSON).await?;

        if status != 200 {
            return Err(fail("fetching user", status, &body));
        }

        Ok(serde_json::from_str::<ApiUser>(&body)?.login)
    }

    /// `GET /user/starred`, paginated. `limit` of 0 means "all of them".
    /// `on_progress` is called once per page with the running total and page number.
    pub async fn stars(
        &self,
        limit: usize,
        mut on_progress: impl FnMut(usize, u32),
    ) -> Result<Vec<Star>> {
        let mut stars: Vec<Star> = Vec::new();
        let mut page: u32 = 1;

        loop {
            let (status, body) = self
                .get(
                    &format!("/user/starred?per_page={PER_PAGE}&page={page}"),
                    ACCEPT_STAR_JSON,
                )
                .await?;

            if status != 200 {
                return Err(fail("fetching stars", status, &body));
            }

            let entries: Vec<ApiStarred> = serde_json::from_str(&body)?;
            let count = entries.len();
            stars.extend(entries.into_iter().map(Star::from));

            on_progress(stars.len(), page);

            // A short page is the last page.
            if count < PER_PAGE {
                break;
            }

            if limit > 0 && stars.len() >= limit {
                break;
            }

            page += 1;
        }

        if limit > 0 && stars.len() > limit {
            stars.truncate(limit);
        }

        Ok(stars)
    }

    /// `DELETE /user/starred/{owner}/{repo}` — 204 on success.
    pub async fn unstar(&self, full_name: &str) -> Result<()> {
        let response = self
            .http
            .delete(format!("{API_BASE}/user/starred/{full_name}"))
            .header("accept", ACCEPT_JSON)
            .bearer_auth(&self.token)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status != 204 {
            let body = response.text().await?;
            return Err(fail(&format!("unstarring {full_name}"), status, &body));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but shape-accurate page from `GET /user/starred`.
    const STARRED_PAGE: &str = r#"[
        {
            "starred_at": "2024-03-01T10:00:00Z",
            "repo": {
                "id": 1,
                "name": "zed",
                "full_name": "zed-industries/zed",
                "description": "Code at the speed of thought",
                "html_url": "https://github.com/zed-industries/zed",
                "language": "Rust",
                "stargazers_count": 50000,
                "forks_count": 3000,
                "open_issues_count": 2000,
                "pushed_at": "2024-05-01T12:00:00Z"
            }
        },
        {
            "starred_at": "2024-01-15T08:30:00Z",
            "repo": {
                "id": 2,
                "name": "bare",
                "full_name": "someone/bare",
                "description": null,
                "html_url": "https://github.com/someone/bare",
                "language": null,
                "stargazers_count": 0,
                "forks_count": 0,
                "open_issues_count": 0,
                "pushed_at": null
            }
        }
    ]"#;

    fn parse_page(json: &str) -> Vec<Star> {
        serde_json::from_str::<Vec<ApiStarred>>(json)
            .expect("fixture should parse")
            .into_iter()
            .map(Star::from)
            .collect()
    }

    #[test]
    fn maps_the_api_envelope_onto_star() {
        let stars = parse_page(STARRED_PAGE);
        let first = &stars[0];

        assert_eq!(first.id, 1);
        assert_eq!(first.full_name, "zed-industries/zed");
        // `html_url` is the one field whose name genuinely differs.
        assert_eq!(first.url, "https://github.com/zed-industries/zed");
        // `starred_at` comes from the envelope, not the repo.
        assert_eq!(first.starred_at, "2024-03-01T10:00:00Z");
        assert_eq!(first.stargazers_count, 50000);
    }

    #[test]
    fn nulls_collapse_to_empty_strings() {
        let stars = parse_page(STARRED_PAGE);
        let bare = &stars[1];

        assert_eq!(bare.description, "");
        assert_eq!(bare.language, "");
        assert_eq!(bare.pushed_at, "");
    }

    #[test]
    fn unknown_api_fields_are_ignored() {
        // GitHub adds fields constantly; that must not break a sync.
        let json = STARRED_PAGE.replace(
            r#""id": 1,"#,
            r#""id": 1, "some_brand_new_field": {"nested": true},"#,
        );
        assert_eq!(parse_page(&json).len(), 2);
    }

    #[test]
    fn star_survives_a_store_round_trip() {
        let stars = parse_page(STARRED_PAGE);
        let encoded = serde_json::to_string(&stars).unwrap();
        let decoded: Vec<Star> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(stars, decoded);
    }

    #[test]
    fn error_bodies_are_collapsed_to_one_clipped_line() {
        let error = fail("fetching stars", 401, "line one\r\nline two");
        let text = error.to_string();

        assert_eq!(text, "fetching stars failed (HTTP 401): line one line two");
        assert!(!text.contains('\n'));
    }

    #[test]
    fn error_bodies_clip_at_120_chars_without_splitting_a_char() {
        // A multi-byte body: naive byte slicing at 120 would panic here.
        let body = "é".repeat(400);
        let text = fail("fetching stars", 500, &body).to_string();
        let clipped = text.rsplit(": ").next().unwrap();

        assert_eq!(clipped.chars().count(), 120);
    }
}
