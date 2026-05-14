use axum::{Json, http::StatusCode, response::IntoResponse};
use chrono::Utc;
use scraper::{Html, Node, Selector};
use serde::Deserialize;
use serde_json::json;
use std::sync::LazyLock;

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build web_fetch http client")
});

const UA: &str = "brief-agent-buildOS26/0.1";

#[derive(Deserialize)]
pub struct Req {
    pub url: String,
}

fn truncate_chars(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_text_filtered(root: scraper::ElementRef) -> String {
    let mut buf = String::new();
    for node in root.descendants() {
        if let Node::Text(text) = node.value() {
            let mut ancestor = node.parent();
            let mut skip = false;
            while let Some(a) = ancestor {
                if let Node::Element(el) = a.value()
                    && matches!(el.name(), "script" | "style" | "noscript")
                {
                    skip = true;
                    break;
                }
                ancestor = a.parent();
            }
            if !skip {
                buf.push_str(text);
                buf.push(' ');
            }
        }
    }
    buf
}

pub async fn web_fetch(Json(req): Json<Req>) -> axum::response::Response {
    let url = req.url.trim().to_string();
    if !url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "url must start with https://"})),
        )
            .into_response();
    }

    let resp = match CLIENT.get(&url).header("User-Agent", UA).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": format!("fetch failed: {}", e)})),
            )
                .into_response();
        }
    };
    let status = resp.status();
    if !status.is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("upstream returned {}", status)})),
        )
            .into_response();
    }
    let html = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": format!("read body failed: {}", e)})),
            )
                .into_response();
        }
    };

    let doc = Html::parse_document(&html);

    let title_sel = Selector::parse("title").expect("static selector");
    let title = doc
        .select(&title_sel)
        .next()
        .map(|e| collapse_ws(&e.text().collect::<String>()))
        .unwrap_or_default();

    let main_sel = Selector::parse("main").expect("static selector");
    let article_sel = Selector::parse("article").expect("static selector");
    let body_sel = Selector::parse("body").expect("static selector");

    let raw_text = doc
        .select(&main_sel)
        .next()
        .or_else(|| doc.select(&article_sel).next())
        .or_else(|| doc.select(&body_sel).next())
        .map(extract_text_filtered)
        .unwrap_or_default();

    let collapsed = collapse_ws(&raw_text);
    let excerpt = truncate_chars(&collapsed, 2000);

    Json(json!({
        "url": url,
        "fetched_at": Utc::now().to_rfc3339(),
        "title": title,
        "excerpt": excerpt,
    }))
    .into_response()
}
