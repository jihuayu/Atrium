use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubApiUser {
    pub id: i64,
    pub login: String,
    pub email: Option<String>,
    pub avatar_url: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub site_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub id: i64,
    pub login: String,
    pub email: String,
    pub avatar_url: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub site_admin: bool,
}

impl From<GitHubApiUser> for GitHubUser {
    fn from(value: GitHubApiUser) -> Self {
        Self {
            id: value.id,
            login: value.login,
            email: value.email.unwrap_or_default(),
            avatar_url: value.avatar_url,
            r#type: value.r#type,
            site_admin: value.site_admin,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiUser {
    pub login: String,
    pub id: i64,
    pub avatar_url: String,
    pub html_url: String,
    #[serde(rename = "type")]
    pub r#type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Reactions {
    pub url: String,
    pub total_count: i64,
    #[serde(rename = "+1")]
    pub plus_one: i64,
    #[serde(rename = "-1")]
    pub minus_one: i64,
    pub laugh: i64,
    pub confused: i64,
    pub heart: i64,
    pub hooray: i64,
    pub rocket: i64,
    pub eyes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueResponse {
    pub id: i64,
    pub node_id: String,
    pub number: i64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    pub state: String,
    pub locked: bool,
    pub user: ApiUser,
    pub labels: Vec<Label>,
    pub comments: i64,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    pub author_association: String,
    pub reactions: Reactions,
    pub url: String,
    pub html_url: String,
    pub comments_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentResponse {
    pub id: i64,
    pub node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    pub user: ApiUser,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: String,
    pub issue_url: String,
    pub author_association: String,
    pub reactions: Reactions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionResponse {
    pub id: i64,
    pub content: String,
    pub user: ApiUser,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateIssueInput {
    pub title: String,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateIssueInput {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<String>,
    pub state_reason: Option<String>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCommentInput {
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCommentInput {
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateReactionInput {
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenderMarkdownInput {
    pub text: String,
    pub mode: Option<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateLabelInput {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListIssuesQuery {
    pub state: Option<String>,
    pub labels: Option<String>,
    pub sort: Option<String>,
    pub direction: Option<String>,
    pub since: Option<String>,
    pub per_page: Option<i64>,
    pub page: Option<i64>,
    pub creator: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListCommentsQuery {
    pub per_page: Option<i64>,
    pub page: Option<i64>,
    pub since: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchIssuesQuery {
    pub q: String,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub per_page: Option<i64>,
    pub page: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaginationQuery {
    pub per_page: Option<i64>,
    pub page: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIssuesResponse {
    pub total_count: i64,
    pub incomplete_results: bool,
    pub items: Vec<IssueResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoRow {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub admin_user_id: Option<i64>,
    pub issue_counter: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoExportResponse {
    pub schema_version: i64,
    pub exported_at: String,
    pub user: GitHubUser,
    pub repos: Vec<ExportRepoRow>,
    pub issues: Vec<ExportIssueRow>,
    pub comments: Vec<ExportCommentRow>,
    pub labels: Vec<ExportLabelRow>,
    pub issue_labels: Vec<ExportIssueLabelRow>,
    pub reactions: Vec<ExportReactionRow>,
    pub users: Vec<ExportUserRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRepoRow {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub admin_user_id: Option<i64>,
    pub issue_counter: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportIssueRow {
    pub id: i64,
    pub repo_id: i64,
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub state_reason: Option<String>,
    pub locked: i64,
    pub user_id: i64,
    pub comment_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportCommentRow {
    pub id: i64,
    pub repo_id: i64,
    pub issue_id: i64,
    pub body: String,
    pub user_id: i64,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub reactions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportLabelRow {
    pub id: i64,
    pub repo_id: i64,
    pub name: String,
    pub description: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportIssueLabelRow {
    pub issue_id: i64,
    pub label_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReactionRow {
    pub id: i64,
    pub comment_id: i64,
    pub user_id: i64,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportUserRow {
    pub id: i64,
    pub login: String,
    pub email: String,
    pub avatar_url: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub site_admin: i64,
    pub cached_at: String,
}
