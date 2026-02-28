pub mod api;
pub mod comment;
pub mod issue;
pub mod pagination;
pub mod user;

use crate::types::{CommentResponse, IssueResponse};

#[derive(Debug, Clone, Copy)]
pub enum AcceptMode {
    Raw,
    Html,
    Full,
}

pub fn parse_accept(value: Option<&str>) -> AcceptMode {
    let Some(v) = value else {
        return AcceptMode::Raw;
    };

    for raw_media in v.split(',') {
        let media = raw_media
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();

        if !media.starts_with("application/vnd.github") {
            continue;
        }
        if media.ends_with(".full+json") {
            return AcceptMode::Full;
        }
        if media.ends_with(".html+json") {
            return AcceptMode::Html;
        }
    }

    AcceptMode::Raw
}

pub fn apply_issue_accept(mut issue: IssueResponse, mode: AcceptMode) -> IssueResponse {
    match mode {
        AcceptMode::Raw => {
            issue.body_html = None;
        }
        AcceptMode::Html => {
            issue.body = None;
        }
        AcceptMode::Full => {}
    }
    issue
}

pub fn apply_comment_accept(mut comment: CommentResponse, mode: AcceptMode) -> CommentResponse {
    match mode {
        AcceptMode::Raw => {
            comment.body_html = None;
        }
        AcceptMode::Html => {
            comment.body = None;
        }
        AcceptMode::Full => {}
    }
    comment
}

#[cfg(test)]
mod tests {
    use super::{apply_comment_accept, apply_issue_accept, parse_accept, AcceptMode};
    use crate::types::{ApiUser, CommentResponse, IssueResponse, Reactions};

    #[test]
    fn parse_accept_supports_version_placeholder_html_mode() {
        let mode = parse_accept(Some(
            "application/vnd.github.VERSION.html+json,application/vnd.github.v3+json",
        ));
        assert!(matches!(mode, AcceptMode::Html));
    }

    #[test]
    fn parse_accept_supports_case_insensitive_full_mode() {
        let mode = parse_accept(Some("Application/Vnd.Github.V3.Full+Json"));
        assert!(matches!(mode, AcceptMode::Full));
    }

    #[test]
    fn parse_accept_defaults_to_raw() {
        let mode = parse_accept(None);
        assert!(matches!(mode, AcceptMode::Raw));
    }

    #[test]
    fn apply_issue_accept_hides_expected_fields() {
        let issue = IssueResponse {
            id: 1,
            node_id: "n".to_string(),
            number: 1,
            title: "t".to_string(),
            body: Some("body".to_string()),
            body_html: Some("<p>body</p>".to_string()),
            state: "open".to_string(),
            locked: false,
            user: ApiUser {
                login: "u".to_string(),
                id: 1,
                avatar_url: "a".to_string(),
                html_url: "h".to_string(),
                r#type: "User".to_string(),
            },
            labels: Vec::new(),
            comments: 0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            closed_at: None,
            author_association: "NONE".to_string(),
            reactions: Reactions::default(),
            url: "u".to_string(),
            html_url: "h".to_string(),
            comments_url: "c".to_string(),
        };

        let raw = apply_issue_accept(issue.clone(), AcceptMode::Raw);
        assert!(raw.body.is_some());
        assert!(raw.body_html.is_none());

        let html = apply_issue_accept(issue.clone(), AcceptMode::Html);
        assert!(html.body.is_none());
        assert!(html.body_html.is_some());

        let full = apply_issue_accept(issue, AcceptMode::Full);
        assert!(full.body.is_some());
        assert!(full.body_html.is_some());
    }

    #[test]
    fn apply_comment_accept_hides_expected_fields() {
        let comment = CommentResponse {
            id: 1,
            node_id: "n".to_string(),
            body: Some("body".to_string()),
            body_html: Some("<p>body</p>".to_string()),
            user: ApiUser {
                login: "u".to_string(),
                id: 1,
                avatar_url: "a".to_string(),
                html_url: "h".to_string(),
                r#type: "User".to_string(),
            },
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            html_url: "h".to_string(),
            issue_url: "i".to_string(),
            author_association: "NONE".to_string(),
            reactions: Reactions::default(),
        };

        let raw = apply_comment_accept(comment.clone(), AcceptMode::Raw);
        assert!(raw.body.is_some());
        assert!(raw.body_html.is_none());

        let html = apply_comment_accept(comment.clone(), AcceptMode::Html);
        assert!(html.body.is_none());
        assert!(html.body_html.is_some());

        let full = apply_comment_accept(comment, AcceptMode::Full);
        assert!(full.body.is_some());
        assert!(full.body_html.is_some());
    }
}
