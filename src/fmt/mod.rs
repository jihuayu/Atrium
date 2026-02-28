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
    use super::{parse_accept, AcceptMode};

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
}
