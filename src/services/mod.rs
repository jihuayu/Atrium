pub mod auth;
pub mod comment;
pub mod cursor;
pub mod exports;
pub mod issue;
pub mod label;
pub mod reaction;
pub mod repo;
pub mod search;
pub mod session;

pub fn normalize_pagination(page: Option<i64>, per_page: Option<i64>) -> (i64, i64, i64) {
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page.unwrap_or(30).clamp(1, 100);
    let offset = (page - 1) * per_page;
    (page, per_page, offset)
}
