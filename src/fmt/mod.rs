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
    if v.contains("application/vnd.github.v3.full+json") {
        AcceptMode::Full
    } else if v.contains("application/vnd.github.v3.html+json") {
        AcceptMode::Html
    } else {
        AcceptMode::Raw
    }
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
