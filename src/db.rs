use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{error::ApiError, Result};

#[derive(Debug, Clone)]
pub enum DbValue {
    Null,
    Integer(i64),
    Text(String),
}

#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
pub trait Database: Send + Sync {
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<u64>;

    async fn query_opt_value(&self, sql: &str, params: &[DbValue]) -> Result<Option<Value>>;

    async fn query_all_value(&self, sql: &str, params: &[DbValue]) -> Result<Vec<Value>>;

    async fn batch(&self, stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()>;
}

pub async fn query_opt<T: DeserializeOwned>(
    db: &dyn Database,
    sql: &str,
    params: &[DbValue],
) -> Result<Option<T>> {
    let value = db.query_opt_value(sql, params).await?;
    match value {
        None => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| ApiError::internal(format!("decode failed: {}", e))),
    }
}

pub async fn query_all<T: DeserializeOwned>(
    db: &dyn Database,
    sql: &str,
    params: &[DbValue],
) -> Result<Vec<T>> {
    let values = db.query_all_value(sql, params).await?;
    let mut rows = Vec::with_capacity(values.len());
    for value in values {
        rows.push(
            serde_json::from_value(value)
                .map_err(|e| ApiError::internal(format!("decode failed: {}", e)))?,
        );
    }
    Ok(rows)
}
