use pulldown_cmark::{html, CowStr, Event, Options, Parser};

pub fn render_markdown(input: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(input, opts).map(|event| match event {
        // Treat raw HTML as literal text to avoid HTML/script injection in rendered output.
        Event::Html(value) | Event::InlineHtml(value) => Event::Text(CowStr::from(value)),
        _ => event,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

#[cfg(test)]
mod tests {
    use super::render_markdown;

    #[test]
    fn renders_basic_markdown() {
        let html = render_markdown("**hello**");
        assert!(html.contains("<strong>hello</strong>"));
    }

    #[test]
    fn escapes_raw_html() {
        let html = render_markdown("<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
