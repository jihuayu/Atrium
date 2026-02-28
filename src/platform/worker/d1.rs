use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_arch = "wasm32")]
use worker::{wasm_bindgen::JsValue, D1Database};

use crate::{db::Database, db::DbValue, error::ApiError, Result};

#[cfg(target_arch = "wasm32")]
pub struct D1Db {
    pub db: D1Database,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct D1Db;

#[cfg(target_arch = "wasm32")]
impl D1Db {
    fn bind_values(
        statement: worker::D1PreparedStatement,
        params: &[DbValue],
    ) -> Result<worker::D1PreparedStatement> {
        let values: Vec<JsValue> = params
            .iter()
            .map(|v| match v {
                DbValue::Null => JsValue::NULL,
                DbValue::Integer(i) => JsValue::from_f64(*i as f64),
                DbValue::Text(s) => JsValue::from_str(s),
            })
            .collect();
        statement
            .bind(&values)
            .map_err(|e| ApiError::internal(format!("d1 bind failed: {}", e)))
    }
}

#[cfg(target_arch = "wasm32")]
#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
impl Database for D1Db {
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<u64> {
        let statement = Self::bind_values(self.db.prepare(sql), params)?;
        let result = statement
            .run()
            .await
            .map_err(|e| ApiError::internal(format!("d1 execute failed: {}", e)))?;
        if !result.success() {
            return Err(ApiError::internal(
                result
                    .error()
                    .unwrap_or_else(|| "d1 execute failed".to_string()),
            ));
        }
        let changed = result
            .meta()
            .map_err(|e| ApiError::internal(format!("d1 meta failed: {}", e)))?
            .and_then(|m| m.changes)
            .unwrap_or(0);
        Ok(changed as u64)
    }

    async fn query_opt_value(&self, sql: &str, params: &[DbValue]) -> Result<Option<Value>> {
        let statement = Self::bind_values(self.db.prepare(sql), params)?;
        statement
            .first::<Value>(None)
            .await
            .map_err(|e| ApiError::internal(format!("d1 query_opt failed: {}", e)))
    }

    async fn query_all_value(&self, sql: &str, params: &[DbValue]) -> Result<Vec<Value>> {
        let statement = Self::bind_values(self.db.prepare(sql), params)?;
        let result = statement
            .all()
            .await
            .map_err(|e| ApiError::internal(format!("d1 query_all failed: {}", e)))?;
        if !result.success() {
            return Err(ApiError::internal(
                result
                    .error()
                    .unwrap_or_else(|| "d1 query_all failed".to_string()),
            ));
        }
        result
            .results::<Value>()
            .map_err(|e| ApiError::internal(format!("d1 decode failed: {}", e)))
    }

    async fn batch(&self, stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()> {
        let mut prepared = Vec::with_capacity(stmts.len());
        for (sql, params) in stmts {
            prepared.push(Self::bind_values(self.db.prepare(sql), &params)?);
        }

        let results = self
            .db
            .batch(prepared)
            .await
            .map_err(|e| ApiError::internal(format!("d1 batch failed: {}", e)))?;
        for result in results {
            if !result.success() {
                return Err(ApiError::internal(
                    result
                        .error()
                        .unwrap_or_else(|| "d1 batch statement failed".to_string()),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
impl Database for D1Db {
    async fn execute(&self, _sql: &str, _params: &[DbValue]) -> Result<u64> {
        Err(ApiError::internal(
            "D1 database only supports wasm32 target",
        ))
    }

    async fn query_opt_value(&self, _sql: &str, _params: &[DbValue]) -> Result<Option<Value>> {
        Err(ApiError::internal(
            "D1 database only supports wasm32 target",
        ))
    }

    async fn query_all_value(&self, _sql: &str, _params: &[DbValue]) -> Result<Vec<Value>> {
        Err(ApiError::internal(
            "D1 database only supports wasm32 target",
        ))
    }

    async fn batch(&self, _stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()> {
        Err(ApiError::internal(
            "D1 database only supports wasm32 target",
        ))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::db::Database;

    use super::D1Db;

    #[tokio::test]
    async fn non_wasm_stub_returns_internal_errors() {
        let db = D1Db;

        let exec_err = db.execute("SELECT 1", &[]).await.err().expect("must fail");
        assert_eq!(exec_err.status, 500);

        let opt_err = db
            .query_opt_value("SELECT 1", &[])
            .await
            .err()
            .expect("must fail");
        assert_eq!(opt_err.status, 500);

        let all_err = db
            .query_all_value("SELECT 1", &[])
            .await
            .err()
            .expect("must fail");
        assert_eq!(all_err.status, 500);

        let batch_err = db.batch(Vec::new()).await.err().expect("must fail");
        assert_eq!(batch_err.status, 500);
    }
}
