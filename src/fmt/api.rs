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

#[cfg(test)]
mod tests {
    use super::to_native_thread;
    use crate::types::{ApiUser, IssueResponse, Label, Reactions};

    #[test]
    fn to_native_thread_maps_labels() {
        let issue = IssueResponse {
            id: 1,
            node_id: "n".to_string(),
            number: 1,
            title: "t".to_string(),
            body: Some("b".to_string()),
            body_html: Some("<p>b</p>".to_string()),
            state: "open".to_string(),
            locked: false,
            user: ApiUser {
                login: "alice".to_string(),
                id: 2,
                avatar_url: "https://avatars/a".to_string(),
                html_url: "https://github.com/alice".to_string(),
                r#type: "User".to_string(),
            },
            labels: vec![Label {
                id: 9,
                name: "bug".to_string(),
                color: "d73a4a".to_string(),
                description: String::new(),
            }],
            comments: 0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            closed_at: None,
            author_association: "NONE".to_string(),
            reactions: Reactions::default(),
            url: String::new(),
            html_url: String::new(),
            comments_url: String::new(),
        };

        let native = to_native_thread(&issue);
        assert_eq!(native.labels.len(), 1);
        assert_eq!(native.labels[0].id, 9);
        assert_eq!(native.labels[0].name, "bug");
        assert_eq!(native.labels[0].color, "d73a4a");
    }
}
