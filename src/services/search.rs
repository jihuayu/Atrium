use serde::Deserialize;

use crate::{
    db::{self, DbValue},
    services::{issue, normalize_pagination},
    types::{IssueResponse, SearchIssuesQuery},
    AppContext, Result,
};

#[derive(Debug, Default)]
struct ParsedQuery {
    repo_owner: Option<String>,
    repo_name: Option<String>,
    label: Option<String>,
    state: Option<String>,
    text: String,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    total: i64,
}

#[derive(Debug, Deserialize)]
struct IssuePointer {
    owner: String,
    repo: String,
    number: i64,
}

pub async fn search_issues(
    ctx: &AppContext<'_>,
    query: &SearchIssuesQuery,
) -> Result<(Vec<IssueResponse>, i64, i64, i64)> {
    let parsed = parse_query(&query.q);
    let (page, per_page, offset) = normalize_pagination(query.page, query.per_page);

    let mut filters = vec!["i.deleted_at IS NULL".to_string()];
    let mut params: Vec<DbValue> = Vec::new();
    let mut idx = 1;

    if let (Some(owner), Some(repo)) = (&parsed.repo_owner, &parsed.repo_name) {
        filters.push(format!("r.owner = ?{}", idx));
        params.push(DbValue::Text(owner.clone()));
        idx += 1;

        filters.push(format!("r.name = ?{}", idx));
        params.push(DbValue::Text(repo.clone()));
        idx += 1;
    }

    if let Some(state) = &parsed.state {
        filters.push(format!("i.state = ?{}", idx));
        params.push(DbValue::Text(state.clone()));
        idx += 1;
    }

    if let Some(label) = &parsed.label {
        filters.push(format!(
            "EXISTS (SELECT 1 FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE il.issue_id = i.id AND l.name = ?{})",
            idx
        ));
        params.push(DbValue::Text(label.clone()));
        idx += 1;
    }

    if !parsed.text.is_empty() {
        filters.push(format!(
            "(INSTR(LOWER(i.title), LOWER(?{})) > 0 OR INSTR(LOWER(COALESCE(i.body, '')), LOWER(?{})) > 0)",
            idx,
            idx + 1
        ));
        params.push(DbValue::Text(parsed.text.clone()));
        params.push(DbValue::Text(parsed.text.clone()));
        idx += 2;
    }

    let where_sql = filters.join(" AND ");
    let count_sql = format!(
        "SELECT COUNT(*) AS total FROM issues i JOIN repos r ON r.id = i.repo_id WHERE {}",
        where_sql
    );

    let total = db::query_opt::<CountRow>(ctx.db, &count_sql, &params)
        .await?
        .map(|v| v.total)
        .unwrap_or(0);

    let sort_col = match query.sort.as_deref().unwrap_or("created") {
        "updated" => "i.updated_at",
        "comments" => "i.comment_count",
        _ => "i.created_at",
    };

    let order = match query.order.as_deref().unwrap_or("desc") {
        "asc" => "ASC",
        _ => "DESC",
    };

    let mut list_params = params.clone();
    list_params.push(DbValue::Integer(per_page));
    list_params.push(DbValue::Integer(offset));

    let list_sql = format!(
        "SELECT r.owner AS owner, r.name AS repo, i.number AS number \
         FROM issues i \
         JOIN repos r ON r.id = i.repo_id \
         WHERE {} \
         ORDER BY {} {} \
         LIMIT ?{} OFFSET ?{}",
        where_sql,
        sort_col,
        order,
        idx,
        idx + 1
    );

    let pointers = db::query_all::<IssuePointer>(ctx.db, &list_sql, &list_params).await?;
    let mut items = Vec::with_capacity(pointers.len());
    for p in pointers {
        items.push(issue::get_issue(ctx, &p.owner, &p.repo, p.number).await?);
    }

    Ok((items, total, page, per_page))
}

fn parse_query(q: &str) -> ParsedQuery {
    let mut parsed = ParsedQuery::default();
    let mut text = Vec::new();

    for token in q.split_whitespace() {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        if let Some(value) = token.strip_prefix("repo:") {
            if let Some((owner, repo)) = value.split_once('/') {
                parsed.repo_owner = Some(owner.to_string());
                parsed.repo_name = Some(repo.to_string());
                continue;
            }
        }
        if let Some(value) = token.strip_prefix("label:") {
            parsed.label = Some(value.to_string());
            continue;
        }
        if let Some(value) = token.strip_prefix("is:") {
            if value == "open" || value == "closed" {
                parsed.state = Some(value.to_string());
            }
            continue;
        }

        if token.starts_with("type:") || token.starts_with("in:") {
            continue;
        }

        if looks_like_qualifier(token) {
            continue;
        }

        let cleaned = token.trim_matches('"').trim_matches('\'');
        if !cleaned.is_empty() {
            text.push(cleaned.to_string());
        }
    }

    parsed.text = text.join(" ");
    parsed
}

fn looks_like_qualifier(token: &str) -> bool {
    let Some((name, value)) = token.split_once(':') else {
        return false;
    };
    if value.is_empty() || value.starts_with("//") {
        return false;
    }
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::{looks_like_qualifier, parse_query};

    #[test]
    fn parse_qualifiers_and_text() {
        let parsed = parse_query("repo:user/blog label:bug is:open hello world");
        assert_eq!(parsed.repo_owner.as_deref(), Some("user"));
        assert_eq!(parsed.repo_name.as_deref(), Some("blog"));
        assert_eq!(parsed.label.as_deref(), Some("bug"));
        assert_eq!(parsed.state.as_deref(), Some("open"));
        assert_eq!(parsed.text, "hello world");
    }

    #[test]
    fn ignores_unsupported_github_qualifiers() {
        let parsed = parse_query(
            r#"repo:user/blog type:issue in:title is:issue sort:updated-desc "hello world""#,
        );
        assert_eq!(parsed.repo_owner.as_deref(), Some("user"));
        assert_eq!(parsed.repo_name.as_deref(), Some("blog"));
        assert_eq!(parsed.state, None);
        assert_eq!(parsed.text, "hello world");
    }

    #[test]
    fn detects_qualifier_tokens() {
        assert!(looks_like_qualifier("sort:updated-desc"));
        assert!(looks_like_qualifier("author:foo"));
        assert!(!looks_like_qualifier("https://example.com"));
        assert!(!looks_like_qualifier("no_colon"));
    }
}
