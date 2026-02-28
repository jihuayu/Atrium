use crate::types::{
    CommentResponse, IssueResponse, NativeCommentResponse, NativeLabel, NativeReactionSummary,
    NativeThreadResponse, NativeUser, Reactions,
};

pub fn to_native_user_from_issue(issue: &IssueResponse) -> NativeUser {
    NativeUser {
        id: issue.user.id,
        login: issue.user.login.clone(),
        avatar_url: issue.user.avatar_url.clone(),
        email: String::new(),
    }
}

pub fn to_native_user_from_comment(comment: &CommentResponse) -> NativeUser {
    NativeUser {
        id: comment.user.id,
        login: comment.user.login.clone(),
        avatar_url: comment.user.avatar_url.clone(),
        email: String::new(),
    }
}

pub fn to_native_thread(issue: &IssueResponse) -> NativeThreadResponse {
    NativeThreadResponse {
        id: issue.id,
        number: issue.number,
        title: issue.title.clone(),
        body: issue.body.clone().unwrap_or_default(),
        body_html: issue.body_html.clone().unwrap_or_default(),
        state: issue.state.clone(),
        comment_count: issue.comments,
        author: to_native_user_from_issue(issue),
        labels: issue
            .labels
            .iter()
            .map(|label| NativeLabel {
                id: label.id,
                name: label.name.clone(),
                color: label.color.clone(),
            })
            .collect(),
        reactions: to_native_reactions(&issue.reactions),
        created_at: issue.created_at.clone(),
        updated_at: issue.updated_at.clone(),
    }
}

pub fn to_native_comment(comment: &CommentResponse) -> NativeCommentResponse {
    NativeCommentResponse {
        id: comment.id,
        body: comment.body.clone().unwrap_or_default(),
        body_html: comment.body_html.clone().unwrap_or_default(),
        author: to_native_user_from_comment(comment),
        reactions: to_native_reactions(&comment.reactions),
        created_at: comment.created_at.clone(),
        updated_at: comment.updated_at.clone(),
    }
}

pub fn to_native_reactions(reactions: &Reactions) -> NativeReactionSummary {
    NativeReactionSummary {
        plus_one: reactions.plus_one,
        minus_one: reactions.minus_one,
        laugh: reactions.laugh,
        confused: reactions.confused,
        heart: reactions.heart,
        hooray: reactions.hooray,
        rocket: reactions.rocket,
        eyes: reactions.eyes,
        total: reactions.total_count,
    }
}
