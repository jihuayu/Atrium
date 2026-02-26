pub fn build_link_header(
    base_url: &str,
    path: &str,
    page: i64,
    per_page: i64,
    total_count: i64,
) -> Option<String> {
    if per_page <= 0 {
        return None;
    }
    let last_page = ((total_count + per_page - 1) / per_page).max(1);
    if last_page <= 1 {
        return None;
    }

    let mut links = Vec::new();
    if page < last_page {
        links.push(format!(
            "<{base}{path}?page={}&per_page={}>; rel=\"next\"",
            page + 1,
            per_page,
            base = base_url,
            path = path
        ));
    }
    if page > 1 {
        links.push(format!(
            "<{base}{path}?page={}&per_page={}>; rel=\"prev\"",
            page - 1,
            per_page,
            base = base_url,
            path = path
        ));
    }
    links.push(format!(
        "<{base}{path}?page=1&per_page={per_page}>; rel=\"first\"",
        base = base_url,
        path = path,
        per_page = per_page
    ));
    links.push(format!(
        "<{base}{path}?page={last}&per_page={per_page}>; rel=\"last\"",
        base = base_url,
        path = path,
        last = last_page,
        per_page = per_page
    ));

    Some(links.join(", "))
}
