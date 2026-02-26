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
