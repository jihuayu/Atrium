use base64::{engine::general_purpose, Engine};

use crate::types::{Reactions, RepoRow};

pub fn issue_node_id(issue_id: i64) -> String {
    general_purpose::STANDARD.encode(format!("xtalk:Issue:{}", issue_id))
}

pub fn issue_reactions(base_url: &str, owner: &str, repo: &str, number: i64) -> Reactions {
    Reactions {
        url: format!("{}/repos/{}/{}/issues/{}/reactions", base_url, owner, repo, number),
        ..Default::default()
    }
}

pub fn author_association(repo: &RepoRow, user_id: i64) -> String {
    if repo.admin_user_id == Some(user_id) {
        "OWNER".to_string()
    } else {
        "NONE".to_string()
    }
}
