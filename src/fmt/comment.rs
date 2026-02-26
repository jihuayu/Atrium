use base64::{engine::general_purpose, Engine};
use serde::{Deserialize, Serialize};

use crate::types::Reactions;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReactionCounts {
    #[serde(default)]
    pub plus_one: i64,
    #[serde(default)]
    pub minus_one: i64,
    #[serde(default)]
    pub laugh: i64,
    #[serde(default)]
    pub confused: i64,
    #[serde(default)]
    pub heart: i64,
    #[serde(default)]
    pub hooray: i64,
    #[serde(default)]
    pub rocket: i64,
    #[serde(default)]
    pub eyes: i64,
    #[serde(default)]
    pub total: i64,
}

impl ReactionCounts {
    pub fn apply_delta(&mut self, content: &str, delta: i64) {
        match content {
            "+1" => self.plus_one += delta,
            "-1" => self.minus_one += delta,
            "laugh" => self.laugh += delta,
            "confused" => self.confused += delta,
            "heart" => self.heart += delta,
            "hooray" => self.hooray += delta,
            "rocket" => self.rocket += delta,
            "eyes" => self.eyes += delta,
            _ => {}
        }
        self.total = self.plus_one
            + self.minus_one
            + self.laugh
            + self.confused
            + self.heart
            + self.hooray
            + self.rocket
            + self.eyes;
        if self.total < 0 {
            self.total = 0;
        }
    }
}

pub fn comment_node_id(comment_id: i64) -> String {
    general_purpose::STANDARD.encode(format!("xtalk:Comment:{}", comment_id))
}

pub fn to_reactions(
    base_url: &str,
    owner: &str,
    repo: &str,
    comment_id: i64,
    raw_json: &str,
) -> Reactions {
    let counts: ReactionCounts = serde_json::from_str(raw_json).unwrap_or_default();
    Reactions {
        url: format!(
            "{}/repos/{}/{}/issues/comments/{}/reactions",
            base_url, owner, repo, comment_id
        ),
        total_count: counts.total,
        plus_one: counts.plus_one,
        minus_one: counts.minus_one,
        laugh: counts.laugh,
        confused: counts.confused,
        heart: counts.heart,
        hooray: counts.hooray,
        rocket: counts.rocket,
        eyes: counts.eyes,
    }
}
