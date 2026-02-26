use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    error::ApiError,
    services::repo,
    types::{CreateLabelInput, Label},
    AppContext, Result,
};

pub async fn list_labels(ctx: &AppContext<'_>, owner: &str, repo_name: &str) -> Result<Vec<Label>> {
    let repo = repo::ensure_repo(ctx, owner, repo_name, ctx.user).await?;

    #[derive(Debug, Deserialize)]
    struct Row {
        id: i64,
        name: String,
        color: String,
        description: String,
    }

    let rows = db::query_all::<Row>(
        ctx.db,
            "SELECT id, name, color, description FROM labels WHERE repo_id = ?1 ORDER BY name ASC",
            &[DbValue::Integer(repo.id)],
        )
    .await?;

    Ok(rows
        .into_iter()
        .map(|v| Label {
            id: v.id,
            name: v.name,
            color: v.color,
            description: v.description,
        })
        .collect())
}

pub async fn create_label(
    ctx: &AppContext<'_>,
    owner: &str,
    repo_name: &str,
    input: &CreateLabelInput,
) -> Result<Label> {
    let user = ctx.user.ok_or_else(ApiError::unauthorized)?;
    let repo = repo::ensure_repo(ctx, owner, repo_name, Some(user)).await?;

    if input.name.trim().is_empty() {
        return Err(ApiError::validation("Label", "name", "missing_field"));
    }

    let color = input
        .color
        .clone()
        .unwrap_or_else(|| "ededed".to_string())
        .to_lowercase();

    ctx.db
        .execute(
            "INSERT INTO labels (repo_id, name, description, color) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(repo_id, name) DO UPDATE SET description = excluded.description, color = excluded.color",
            &[
                DbValue::Integer(repo.id),
                DbValue::Text(input.name.trim().to_string()),
                DbValue::Text(input.description.clone().unwrap_or_default()),
                DbValue::Text(color.clone()),
            ],
        )
        .await?;

    #[derive(Debug, Deserialize)]
    struct Row {
        id: i64,
        name: String,
        color: String,
        description: String,
    }

    let row = db::query_opt::<Row>(
        ctx.db,
            "SELECT id, name, color, description FROM labels WHERE repo_id = ?1 AND name = ?2",
            &[DbValue::Integer(repo.id), DbValue::Text(input.name.trim().to_string())],
        )
    .await?
        .ok_or_else(|| ApiError::internal("failed to create label"))?;

    Ok(Label {
        id: row.id,
        name: row.name,
        color: row.color,
        description: row.description,
    })
}

pub async fn ensure_label_ids(ctx: &AppContext<'_>, repo_id: i64, names: &[String]) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        ctx.db
            .execute(
                "INSERT INTO labels (repo_id, name, description, color) VALUES (?1, ?2, '', 'ededed') \
                 ON CONFLICT(repo_id, name) DO NOTHING",
                &[DbValue::Integer(repo_id), DbValue::Text(name.to_string())],
            )
            .await?;

        #[derive(Debug, Deserialize)]
        struct Row {
            id: i64,
        }

        let row = db::query_opt::<Row>(
            ctx.db,
                "SELECT id FROM labels WHERE repo_id = ?1 AND name = ?2",
                &[DbValue::Integer(repo_id), DbValue::Text(name.to_string())],
            )
        .await?;
        if let Some(v) = row {
            ids.push(v.id);
        }
    }
    Ok(ids)
}
