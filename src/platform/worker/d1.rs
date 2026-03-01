use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_arch = "wasm32")]
use worker::{
    D1Database,
    js_sys::{Array, Error as JsError, Function, Promise, Reflect},
    wasm_bindgen::{JsCast, JsValue},
    wasm_bindgen_futures::JsFuture,
};

use crate::{Result, db::Database, db::DbValue, error::ApiError};

#[cfg(target_arch = "wasm32")]
pub struct D1Db {
    session: JsValue,
}

#[cfg(target_arch = "wasm32")]
unsafe impl Send for D1Db {}
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for D1Db {}

#[cfg(not(target_arch = "wasm32"))]
pub struct D1Db;

#[cfg(target_arch = "wasm32")]
impl D1Db {
    pub fn from_database(db: D1Database) -> Result<Self> {
        let db_js = db.as_ref().clone();
        let session = call_method1(&db_js, "withSession", &JsValue::from_str("first-primary"))
            .map_err(|e| ApiError::internal(format!("d1 withSession failed: {}", js_error(e))))?;
        Ok(Self { session })
    }

    fn prepare_in_session(&self, sql: &str) -> Result<worker::D1PreparedStatement> {
        let stmt = call_method1(&self.session, "prepare", &JsValue::from_str(sql))
            .map_err(|e| ApiError::internal(format!("d1 prepare failed: {}", js_error(e))))?;
        let stmt: worker::worker_sys::D1PreparedStatement = stmt.unchecked_into();
        Ok(stmt.into())
    }

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

    async fn batch_in_session(
        &self,
        statements: Vec<worker::D1PreparedStatement>,
    ) -> Result<Vec<worker::worker_sys::D1Result>> {
        let array = Array::new();
        for stmt in statements {
            array.push(stmt.inner().as_ref());
        }

        let promise = call_method1(&self.session, "batch", &JsValue::from(array))
            .map_err(|e| ApiError::internal(format!("d1 batch failed: {}", js_error(e))))?;
        let promise: Promise = promise
            .dyn_into()
            .map_err(|e| ApiError::internal(format!("d1 batch cast failed: {}", js_error(e))))?;
        let raw = JsFuture::from(promise)
            .await
            .map_err(|e| ApiError::internal(format!("d1 batch failed: {}", js_error(e))))?;
        let items: Array = raw
            .dyn_into()
            .map_err(|e| ApiError::internal(format!("d1 batch decode failed: {}", js_error(e))))?;

        let mut out = Vec::with_capacity(items.length() as usize);
        for item in items.iter() {
            out.push(item.unchecked_into::<worker::worker_sys::D1Result>());
        }
        Ok(out)
    }
}

#[cfg(target_arch = "wasm32")]
fn call_method1(
    this: &JsValue,
    method_name: &str,
    arg: &JsValue,
) -> std::result::Result<JsValue, JsValue> {
    let method = Reflect::get(this, &JsValue::from_str(method_name))?;
    let function: Function = method.dyn_into()?;
    function.call1(this, arg)
}

#[cfg(target_arch = "wasm32")]
fn js_error(value: JsValue) -> String {
    if let Ok(err) = value.clone().dyn_into::<JsError>() {
        return err.to_string().into();
    }
    value
        .as_string()
        .unwrap_or_else(|| "unknown js error".to_string())
}

#[cfg(target_arch = "wasm32")]
#[cfg_attr(feature = "server", async_trait)]
#[cfg_attr(not(feature = "server"), async_trait(?Send))]
impl Database for D1Db {
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<u64> {
        let statement = Self::bind_values(self.prepare_in_session(sql)?, params)?;
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
        let statement = Self::bind_values(self.prepare_in_session(sql)?, params)?;
        statement
            .first::<Value>(None)
            .await
            .map_err(|e| ApiError::internal(format!("d1 query_opt failed: {}", e)))
    }

    async fn query_all_value(&self, sql: &str, params: &[DbValue]) -> Result<Vec<Value>> {
        let statement = Self::bind_values(self.prepare_in_session(sql)?, params)?;
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
            prepared.push(Self::bind_values(self.prepare_in_session(sql)?, &params)?);
        }

        let results = self.batch_in_session(prepared).await?;
        for result in results {
            let success = result.success().map_err(|e| {
                ApiError::internal(format!("d1 batch status failed: {}", js_error(e)))
            })?;
            if !success {
                let msg = result
                    .error()
                    .map_err(|e| {
                        ApiError::internal(format!("d1 batch error decode failed: {}", js_error(e)))
                    })?
                    .unwrap_or_else(|| "d1 batch statement failed".to_string());
                return Err(ApiError::internal(msg));
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
