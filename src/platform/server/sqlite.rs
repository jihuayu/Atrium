use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use serde_json::{Map, Value};
use sqlx::{sqlite::SqliteArguments, sqlite::SqliteRow, Column, Row, SqlitePool};

use crate::{db::Database, db::DbValue, error::ApiError, Result};

#[derive(Clone)]
pub struct SqliteDatabase {
    pool: SqlitePool,
}

impl SqliteDatabase {
    pub async fn connect_and_migrate(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url)
            .await
            .map_err(|e| ApiError::internal(format!("db connect failed: {}", e)))?;

        sqlx::query(include_str!("../../../migrations/0001_initial_schema.sql"))
            .execute(&pool)
            .await
            .map_err(|e| ApiError::internal(format!("migration failed: {}", e)))?;

        let has_user_identities: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'user_identities' LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .map_err(|e| ApiError::internal(format!("check migration state failed: {}", e)))?;

        if has_user_identities.is_none() {
            sqlx::query(include_str!(
                "../../../migrations/0002_multi_provider_auth.sql"
            ))
            .execute(&pool)
            .await
            .map_err(|e| ApiError::internal(format!("migration 0002 failed: {}", e)))?;
        }

        Ok(Self { pool })
    }

    fn bind_all<'q>(
        mut query: sqlx::query::Query<'q, sqlx::Sqlite, SqliteArguments<'q>>,
        params: &'q [DbValue],
    ) -> sqlx::query::Query<'q, sqlx::Sqlite, SqliteArguments<'q>> {
        for param in params {
            query = match param {
                DbValue::Null => query.bind::<Option<String>>(None),
                DbValue::Integer(value) => query.bind(*value),
                DbValue::Text(value) => query.bind(value.clone()),
            };
        }
        query
    }

    fn row_to_json(row: &SqliteRow) -> Value {
        let mut out = Map::new();
        for (index, column) in row.columns().iter().enumerate() {
            let key = column.name();
            let value = if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<String>, _>(index) {
                value.map(Value::from).unwrap_or(Value::Null)
            } else if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
                match value {
                    Some(bytes) => Value::from(general_purpose::STANDARD.encode(bytes)),
                    None => Value::Null,
                }
            } else {
                Value::Null
            };
            out.insert(key.to_string(), value);
        }
        Value::Object(out)
    }
}

#[async_trait]
impl Database for SqliteDatabase {
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<u64> {
        let query = Self::bind_all(sqlx::query(sql), params);
        let result = query
            .execute(&self.pool)
            .await
            .map_err(|e| ApiError::internal(format!("execute failed: {}", e)))?;
        Ok(result.rows_affected())
    }

    async fn query_opt_value(&self, sql: &str, params: &[DbValue]) -> Result<Option<Value>> {
        let query = Self::bind_all(sqlx::query(sql), params);
        let row = query
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ApiError::internal(format!("query_opt failed: {}", e)))?;
        match row {
            None => Ok(None),
            Some(row) => Ok(Some(Self::row_to_json(&row))),
        }
    }

    async fn query_all_value(&self, sql: &str, params: &[DbValue]) -> Result<Vec<Value>> {
        let query = Self::bind_all(sqlx::query(sql), params);
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ApiError::internal(format!("query_all failed: {}", e)))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(Self::row_to_json(&row));
        }

        Ok(out)
    }

    async fn batch(&self, stmts: Vec<(&str, Vec<DbValue>)>) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| ApiError::internal(format!("begin tx failed: {}", e)))?;

        for (sql, params) in stmts {
            let query = Self::bind_all(sqlx::query(sql), &params);
            query
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::internal(format!("batch execute failed: {}", e)))?;
        }

        tx.commit()
            .await
            .map_err(|e| ApiError::internal(format!("commit tx failed: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteDatabase;
    use crate::db::{Database, DbValue};

    async fn make_db() -> (tempfile::TempPath, SqliteDatabase) {
        let db_file = tempfile::NamedTempFile::new()
            .expect("temp file")
            .into_temp_path();
        let db_url = format!("sqlite://{}", db_file.to_string_lossy().replace('\\', "/"));
        let db = SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("connect db");
        (db_file, db)
    }

    #[tokio::test]
    async fn query_and_batch_cover_value_kinds() {
        let (_db_file, db) = make_db().await;
        db.execute(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, i INTEGER, f REAL, s TEXT, b BLOB, n TEXT)",
            &[],
        )
        .await
        .expect("create table");

        db.execute(
            "INSERT INTO t (id, i, f, s, b, n) VALUES (1, 7, 1.5, 'txt', x'0102', NULL)",
            &[],
        )
        .await
        .expect("insert row");

        let one = db
            .query_opt_value(
                "SELECT i, f, s, b, n FROM t WHERE id = ?1",
                &[DbValue::Integer(1)],
            )
            .await
            .expect("query one")
            .expect("row exists");
        assert_eq!(one["i"].as_i64(), Some(7));
        assert_eq!(one["f"].as_f64(), Some(1.5));
        assert_eq!(one["s"].as_str(), Some("txt"));
        assert_eq!(one["b"].as_str(), Some("AQI="));
        assert!(one["n"].is_null());

        let null_value = db
            .query_opt_value("SELECT ?1 AS v", &[DbValue::Null])
            .await
            .expect("query null")
            .expect("row exists");
        assert!(null_value["v"].is_null());

        db.batch(vec![
            (
                "INSERT INTO t (id, i, f, s, b, n) VALUES (?1, ?2, ?3, ?4, x'03', NULL)",
                vec![
                    DbValue::Integer(2),
                    DbValue::Integer(9),
                    DbValue::Text("2.5".to_string()),
                    DbValue::Text("next".to_string()),
                ],
            ),
            (
                "UPDATE t SET s = ?1 WHERE id = ?2",
                vec![DbValue::Text("updated".to_string()), DbValue::Integer(1)],
            ),
        ])
        .await
        .expect("batch ok");

        let all = db
            .query_all_value("SELECT id, s FROM t ORDER BY id ASC", &[])
            .await
            .expect("query all");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["s"].as_str(), Some("updated"));
    }

    #[tokio::test]
    async fn db_errors_map_to_api_error() {
        let (_db_file, db) = make_db().await;

        let exec_err = db
            .execute("INSERT INTO missing_table(x) VALUES(1)", &[])
            .await
            .expect_err("execute should fail");
        assert_eq!(exec_err.status, 500);

        let opt_err = db
            .query_opt_value("SELECT * FROM missing_table", &[])
            .await
            .expect_err("query_opt should fail");
        assert_eq!(opt_err.status, 500);

        let all_err = db
            .query_all_value("SELECT * FROM missing_table", &[])
            .await
            .expect_err("query_all should fail");
        assert_eq!(all_err.status, 500);

        let batch_err = db
            .batch(vec![("INSERT INTO missing_table(x) VALUES(1)", Vec::new())])
            .await
            .expect_err("batch should fail");
        assert_eq!(batch_err.status, 500);
    }
}
