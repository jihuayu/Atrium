use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use async_trait::async_trait;
use moka::future::Cache;

use crate::{cache::CommentCacheStore, types::CommentResponse, Result};

type ListKey = (i64, i64, i64);

#[derive(Clone)]
pub struct CommentCache {
    list: Cache<ListKey, (Vec<CommentResponse>, i64)>,
    single: Cache<i64, CommentResponse>,
    issue_index: Arc<RwLock<HashMap<i64, HashSet<ListKey>>>>,
}

impl CommentCache {
    pub fn new(max_capacity: u64, ttl_secs: u64) -> Self {
        let ttl = Duration::from_secs(ttl_secs.max(1));
        Self {
            list: Cache::builder()
                .max_capacity(max_capacity.max(1))
                .time_to_live(ttl)
                .build(),
            single: Cache::builder()
                .max_capacity((max_capacity.max(1) * 10).max(10))
                .time_to_live(ttl)
                .build(),
            issue_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl CommentCacheStore for CommentCache {
    async fn get_list(
        &self,
        issue_id: i64,
        page: i64,
        per_page: i64,
    ) -> Result<Option<(Vec<CommentResponse>, i64)>> {
        Ok(self.list.get(&(issue_id, page, per_page)).await)
    }

    async fn set_list(
        &self,
        issue_id: i64,
        page: i64,
        per_page: i64,
        rows: Vec<CommentResponse>,
        total: i64,
    ) -> Result<()> {
        let key = (issue_id, page, per_page);
        self.list.insert(key, (rows, total)).await;

        if let Ok(mut index) = self.issue_index.write() {
            index
                .entry(issue_id)
                .or_insert_with(HashSet::new)
                .insert(key);
        }

        Ok(())
    }

    async fn get_single(&self, comment_id: i64) -> Result<Option<CommentResponse>> {
        Ok(self.single.get(&comment_id).await)
    }

    async fn set_single(&self, row: CommentResponse) -> Result<()> {
        self.single.insert(row.id, row).await;
        Ok(())
    }

    async fn invalidate_issue(&self, issue_id: i64) -> Result<()> {
        let keys = if let Ok(mut index) = self.issue_index.write() {
            index.remove(&issue_id).unwrap_or_default()
        } else {
            HashSet::new()
        };

        for key in keys {
            self.list.invalidate(&key).await;
        }
        Ok(())
    }

    async fn invalidate_comment(&self, comment_id: i64) -> Result<()> {
        self.single.invalidate(&comment_id).await;
        Ok(())
    }
}
