use async_trait::async_trait;

use crate::{types::CommentResponse, Result};

#[cfg_attr(feature = "worker", async_trait(?Send))]
#[cfg_attr(not(feature = "worker"), async_trait)]
pub trait CommentCacheStore: Send + Sync {
    async fn get_list(
        &self,
        issue_id: i64,
        page: i64,
        per_page: i64,
    ) -> Result<Option<(Vec<CommentResponse>, i64)>>;

    async fn set_list(
        &self,
        issue_id: i64,
        page: i64,
        per_page: i64,
        rows: Vec<CommentResponse>,
        total: i64,
    ) -> Result<()>;

    async fn get_single(&self, comment_id: i64) -> Result<Option<CommentResponse>>;

    async fn set_single(&self, row: CommentResponse) -> Result<()>;

    async fn invalidate_issue(&self, issue_id: i64) -> Result<()>;

    async fn invalidate_comment(&self, comment_id: i64) -> Result<()>;
}