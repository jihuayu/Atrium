use std::{path::Path, str::FromStr};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use serde_json::{Map, Value};
use sqlx::{
    query, query_scalar, sqlite::SqliteArguments, sqlite::SqliteConnectOptions, sqlite::SqliteRow,
    Column, Row, SqlitePool,
};

use crate::{db::Database, db::DbValue, error::ApiError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

#[derive(Clone)]
pub struct SqliteDatabase {
    pool: SqlitePool,
}

impl SqliteDatabase {
    pub async fn connect_and_migrate(database_url: &str) -> Result<Self> {
        ensure_sqlite_parent_dir(database_url)?;

        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| ApiError::internal(format!("parse db url failed: {}", e)))?
            .create_if_missing(true);

        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|e| ApiError::internal(format!("db connect failed: {}", e)))?;

        bootstrap_legacy_migration_state(&pool).await?;
        MIGRATOR
            .run(&pool)
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

fn ensure_sqlite_parent_dir(database_url: &str) -> Result<()> {
    if !database_url.starts_with("sqlite://") {
        return Ok(());
    }

    let raw_path = database_url
        .trim_start_matches("sqlite://")
        .split('?')
        .next()
        .unwrap_or_default();

    if raw_path.is_empty() || raw_path == ":memory:" {
        return Ok(());
    }

    let parent = Path::new(raw_path).parent();
    if let Some(parent) = parent {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ApiError::internal(format!("create db directory failed: {}", e)))?;
        }
    }

    Ok(())
}

async fn bootstrap_legacy_migration_state(pool: &SqlitePool) -> Result<()> {
    let has_migration_table: Option<i64> = query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("check sqlx migration table failed: {}", e)))?;
    if has_migration_table.is_some() {
        return Ok(());
    }

    let has_user_identities: bool = query_scalar::<_, i64>(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'user_identities' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("check user_identities table failed: {}", e)))?
    .is_some();
    let has_token_provider: bool = query_scalar::<_, i64>(
        "SELECT 1 FROM pragma_table_info('token_cache') WHERE name = 'provider' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("check token_cache.provider failed: {}", e)))?
    .is_some();
    let has_repo_owner_user_id: bool = query_scalar::<_, i64>(
        "SELECT 1 FROM pragma_table_info('repos') WHERE name = 'owner_user_id' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("check repos.owner_user_id failed: {}", e)))?
    .is_some();

    // Legacy databases that were migrated manually may already be at schema v2
    // but still miss `_sqlx_migrations`. Seed v1/v2 records so sqlx can continue
    // from the current state.
    if !(has_user_identities && has_token_provider) {
        return Ok(());
    }

    query(
        r#"
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(pool)
    .await
    .map_err(|e| ApiError::internal(format!("create sqlx migration table failed: {}", e)))?;

    let mut seeded_versions = vec![1_i64, 2_i64];
    if has_repo_owner_user_id {
        seeded_versions.push(3);
    }

    for version in seeded_versions {
        let migration = MIGRATOR
            .iter()
            .find(|m| m.version == version)
            .ok_or_else(|| ApiError::internal(format!("missing migration {}", version)))?;
        query(
            "INSERT OR IGNORE INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (?1, ?2, TRUE, ?3, 0)",
        )
        .bind(migration.version)
        .bind(migration.description.as_ref())
        .bind(migration.checksum.as_ref())
        .execute(pool)
        .await
        .map_err(|e| ApiError::internal(format!("seed sqlx migration {} failed: {}", version, e)))?;
    }

    Ok(())
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
    use sqlx::{query, query_scalar, sqlite::SqliteConnectOptions, SqlitePool};
    use std::str::FromStr;

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

    #[tokio::test]
    async fn connect_and_migrate_creates_missing_parent_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("data").join("atrium.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));

        assert!(!db_path.parent().expect("parent").exists());
        let _db = SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("connect db");

        assert!(db_path.parent().expect("parent").exists());
        assert!(db_path.exists());
    }

    #[tokio::test]
    async fn connect_and_migrate_bootstraps_legacy_schema_to_sqlx_tracking() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("legacy.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));

        let options = SqliteConnectOptions::from_str(&db_url)
            .expect("parse sqlite url")
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .expect("connect legacy db");
        query(include_str!("../../../migrations/0001_initial_schema.sql"))
            .execute(&pool)
            .await
            .expect("apply migration 0001 manually");
        query(include_str!("../../../migrations/0002_multi_provider_auth.sql"))
        .execute(&pool)
        .await
        .expect("apply migration 0002 manually");
        pool.close().await;

        let _db = SqliteDatabase::connect_and_migrate(&db_url)
            .await
            .expect("connect and migrate through sqlx");

        let verify_pool = SqlitePool::connect(&db_url).await.expect("verify connect");
        let count: i64 = query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&verify_pool)
            .await
            .expect("count sqlx migrations");
        assert_eq!(count, 3);
        let v1: Option<i64> =
            query_scalar("SELECT 1 FROM _sqlx_migrations WHERE version = 1 LIMIT 1")
                .fetch_optional(&verify_pool)
                .await
                .expect("find v1");
        let v2: Option<i64> =
            query_scalar("SELECT 1 FROM _sqlx_migrations WHERE version = 2 LIMIT 1")
                .fetch_optional(&verify_pool)
                .await
                .expect("find v2");
        let v3: Option<i64> =
            query_scalar("SELECT 1 FROM _sqlx_migrations WHERE version = 3 LIMIT 1")
                .fetch_optional(&verify_pool)
                .await
                .expect("find v3");
        assert!(v1.is_some());
        assert!(v2.is_some());
        assert!(v3.is_some());
    }
}
