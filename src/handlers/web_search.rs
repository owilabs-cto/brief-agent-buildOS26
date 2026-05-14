use axum::{Json, http::StatusCode, response::IntoResponse};
use chrono::Utc;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::LazyLock;

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("build web_search http client")
});

const DDG_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Deserialize)]
pub struct Req {
    pub query: String,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Serialize, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub fetched_at: String,
}

fn truncate_chars(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

pub async fn web_search(Json(req): Json<Req>) -> axum::response::Response {
    let max = req.max_results.unwrap_or(8).clamp(1, 8);
    let query = req.query.trim().to_string();
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "query required"})),
        )
            .into_response();
    }

    if let Ok(key) = std::env::var("APP__TAVILY_API_KEY")
        && !key.trim().is_empty()
    {
        match tavily(&key, &query, max).await {
            Ok(results) => {
                let count = results.len();
                return Json(json!({
                    "query": query,
                    "results": results,
                    "source": "tavily",
                    "result_count": count,
                }))
                .into_response();
            }
            Err(e) => {
                tracing::warn!(error = %e, "tavily failed, falling back to duckduckgo");
            }
        }
    }

    match duckduckgo(&query, max).await {
        Ok((results, warning)) => {
            let count = results.len();
            let mut body = json!({
                "query": query,
                "results": results,
                "source": "duckduckgo_html",
                "result_count": count,
            });
            if let Some(w) = warning {
                body["warning"] = json!(w);
            }
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": format!("search failed: {}", e),
                "source": "duckduckgo_html",
            })),
        )
            .into_response(),
    }
}

async fn tavily(key: &str, query: &str, max: usize) -> anyhow::Result<Vec<SearchResult>> {
    let resp = CLIENT
        .post("https://api.tavily.com/search")
        .json(&json!({
            "api_key": key,
            "query": query,
            "max_results": max,
            "search_depth": "basic",
            "include_answer": false,
        }))
        .send()
        .await?
        .error_for_status()?;
    let body: Value = resp.json().await?;
    let now = Utc::now().to_rfc3339();
    let results = body["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .take(max)
                .map(|r| SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    url: r["url"].as_str().unwrap_or("").to_string(),
                    snippet: truncate_chars(r["content"].as_str().unwrap_or(""), 400),
                    fetched_at: now.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(results)
}

async fn duckduckgo(
    query: &str,
    max: usize,
) -> anyhow::Result<(Vec<SearchResult>, Option<String>)> {
    let url = format!(
        "https://html.duckduckgo.com/html?q={}",
        urlencoding::encode(query)
    );
    let html = CLIENT
        .get(&url)
        .header("User-Agent", DDG_UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let doc = Html::parse_document(&html);
    let result_sel = Selector::parse("div.result")
        .map_err(|e| anyhow::anyhow!("bad result selector: {:?}", e))?;
    let title_sel = Selector::parse(".result__title a")
        .map_err(|e| anyhow::anyhow!("bad title selector: {:?}", e))?;
    let snippet_sel = Selector::parse(".result__snippet")
        .map_err(|e| anyhow::anyhow!("bad snippet selector: {:?}", e))?;

    let now = Utc::now().to_rfc3339();
    let mut out: Vec<SearchResult> = Vec::new();
    for div in doc.select(&result_sel) {
        if out.len() >= max {
            break;
        }
        let title_el = div.select(&title_sel).next();
        let title = title_el
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let href = title_el
            .and_then(|e| e.value().attr("href"))
            .unwrap_or("")
            .to_string();
        let resolved = resolve_ddg_url(&href);
        if resolved.is_empty() {
            continue;
        }
        let snippet = div
            .select(&snippet_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url: resolved,
            snippet: truncate_chars(&snippet, 400),
            fetched_at: now.clone(),
        });
    }

    let warning = if out.is_empty() {
        Some("DDG returned 0 results or was rate-limited".to_string())
    } else {
        None
    };
    Ok((out, warning))
}

fn resolve_ddg_url(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if let Some(idx) = href.find("uddg=") {
        let after = &href[idx + "uddg=".len()..];
        let encoded = after.split('&').next().unwrap_or("");
        if let Ok(decoded) = urlencoding::decode(encoded) {
            return decoded.into_owned();
        }
    }
    String::new()
}
