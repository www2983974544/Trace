use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_GEMINI_BASE: &str = "https://generativelanguage.googleapis.com";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeminiTestResult {
    pub model: String,
    pub latency_ms: u64,
    pub reply_preview: String,
}

pub fn normalize_base_and_model<'a>(base_url: &'a str, model: &'a str) -> (&'a str, &'a str) {
    let base = if base_url.trim().is_empty() {
        DEFAULT_GEMINI_BASE
    } else {
        base_url.trim().trim_end_matches('/')
    };
    let clean_model = model.trim().trim_start_matches("models/");
    (base, clean_model)
}

pub fn build_stream_url(base_url: &str, model: &str) -> String {
    let (base, clean_model) = normalize_base_and_model(base_url, model);
    format!("{base}/v1beta/models/{clean_model}:streamGenerateContent?alt=sse")
}

pub fn build_generate_url(base_url: &str, model: &str) -> String {
    let (base, clean_model) = normalize_base_and_model(base_url, model);
    format!("{base}/v1beta/models/{clean_model}:generateContent")
}

pub fn extract_parts_text(val: &Value) -> Option<String> {
    let parts = val
        .get("candidates")?
        .as_array()?
        .first()?
        .get("content")?
        .get("parts")?
        .as_array()?;

    let mut out = String::new();
    for part in parts {
        if part.get("thought").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            out.push_str(text);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn extract_error_message(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.trim().to_string();
        }
    }
    let trimmed = body.trim();
    if trimmed.chars().count() > 240 {
        let short: String = trimmed.chars().take(240).collect();
        format!("{short}…")
    } else {
        trimmed.to_string()
    }
}

pub fn format_http_error(status: reqwest::StatusCode, body: &str) -> String {
    let detail = extract_error_message(body);
    match status.as_u16() {
        401 | 403 => format!("API Key 无效或无该接口权限 ({status})：{detail}"),
        404 => format!("模型名称无效或已下线 ({status})：{detail}"),
        429 => format!(
            "当前 Key 配额不足或触发限流 (429)：免费额度可能不包含该模型（可尝试 gemini-3.6-flash / gemini-3.8-flash，或在 Google AI Studio 开启结算）。详情：{detail}"
        ),
        503 => format!(
            "Google 该模型当前高峰期拥堵 (503)：建议稍后重试，或先切换 gemini-3.6-flash 使用。详情：{detail}"
        ),
        _ => format!("请求失败 ({status})：{detail}"),
    }
}

pub fn parse_windows_proxy_server(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let selected = if trimmed.contains('=') {
        let mut https_val: Option<&str> = None;
        let mut http_val: Option<&str> = None;
        for part in trimmed.split(';') {
            let Some((scheme, addr)) = part.split_once('=') else {
                continue;
            };
            let s = scheme.trim();
            let a = addr.trim();
            if a.is_empty() {
                continue;
            }
            if s.eq_ignore_ascii_case("https") {
                https_val = Some(a);
            } else if s.eq_ignore_ascii_case("http") {
                http_val = Some(a);
            }
        }
        https_val.or(http_val)?
    } else {
        trimmed
    };

    if selected.is_empty() || selected.to_ascii_lowercase().starts_with("socks") {
        return None;
    }

    if selected.contains("://") {
        Some(selected.to_string())
    } else {
        Some(format!("http://{selected}"))
    }
}

#[cfg(windows)]
fn detect_windows_system_proxy() -> Option<String> {
    for var in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        if std::env::var_os(var).is_some_and(|v| !v.is_empty()) {
            return None;
        }
    }
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled != 1 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    parse_windows_proxy_server(&server)
}

fn build_http_client(timeout: Option<Duration>, target_url: &str) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }

    #[cfg(windows)]
    {
        let is_local = target_url.contains("://127.0.0.1") || target_url.contains("://localhost");
        if !is_local {
            if let Some(proxy_url) = detect_windows_system_proxy() {
                if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                    builder = builder.proxy(proxy);
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = target_url;
    }

    builder
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))
}

pub fn parse_sse_data_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let payload = trimmed.strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }

    let val: Value = serde_json::from_str(payload).ok()?;
    extract_parts_text(&val)
}

pub async fn test_connection(
    api_key: &str,
    model: &str,
    base_url: &str,
) -> Result<GeminiTestResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("请先填写 Gemini API Key".to_string());
    }
    let (_, clean_model) = normalize_base_and_model(base_url, model);
    if clean_model.is_empty() {
        return Err("请先填写模型名称".to_string());
    }

    let url = build_generate_url(base_url, clean_model);
    let body = json!({
        "contents": [
            {
                "role": "user",
                "parts": [{ "text": "Reply OK" }]
            }
        ]
    });

    let client = build_http_client(Some(Duration::from_secs(30)), &url)?;

    let started = Instant::now();
    let resp = client
        .post(&url)
        .header("x-goog-api-key", key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "请求超时（30秒）：请确认代理已开启，或该模型当前高峰响应较慢（可切换 gemini-3.6-flash 重试）".to_string()
            } else {
                format!("请求失败：{e}")
            }
        })?;

    let latency_ms = started.elapsed().as_millis() as u64;
    if !resp.status().is_success() {
        let status = resp.status();
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format_http_error(status, &err_text));
    }

    let val: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 Gemini 响应失败：{e}"))?;

    let has_candidates = val
        .get("candidates")
        .and_then(|c| c.as_array())
        .is_some_and(|arr| !arr.is_empty());
    if !has_candidates {
        return Err("接口已连通，但未返回候选回复内容".to_string());
    }

    let reply_preview = extract_parts_text(&val)
        .map(|s| {
            let trimmed = s.trim();
            if trimmed.chars().count() > 40 {
                let short: String = trimmed.chars().take(40).collect();
                format!("{short}…")
            } else {
                trimmed.to_string()
            }
        })
        .unwrap_or_default();

    Ok(GeminiTestResult {
        model: clean_model.to_string(),
        latency_ms,
        reply_preview,
    })
}

pub async fn stream_analyze<F>(
    api_key: &str,
    model: &str,
    base_url: &str,
    prompt_text: &str,
    mut on_chunk: F,
) -> Result<String, String>
where
    F: FnMut(&str),
{
    let key = api_key.trim();
    if key.is_empty() {
        return Err("请先在「设置」中配置 Gemini API Key".to_string());
    }

    let url = build_stream_url(base_url, model);
    let body = json!({
        "contents": [
            {
                "role": "user",
                "parts": [{ "text": prompt_text }]
            }
        ]
    });

    let client = build_http_client(None, &url)?;
    let resp = client
        .post(&url)
        .header("x-goog-api-key", key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 Gemini 失败：{e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format_http_error(status, &err_text));
    }

    let mut stream = resp.bytes_stream();
    let mut byte_buf: Vec<u8> = Vec::new();
    let mut full_text = String::new();

    while let Some(chunk_res) = stream.next().await {
        let bytes = chunk_res.map_err(|e| format!("读取流式响应中断：{e}"))?;
        byte_buf.extend_from_slice(&bytes);

        while let Some(pos) = byte_buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = byte_buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            if let Some(delta) = parse_sse_data_line(&line) {
                on_chunk(&delta);
                full_text.push_str(&delta);
            }
        }
    }

    if !byte_buf.is_empty() {
        let tail = String::from_utf8_lossy(&byte_buf);
        if let Some(delta) = parse_sse_data_line(&tail) {
            on_chunk(&delta);
            full_text.push_str(&delta);
        }
    }

    if full_text.is_empty() {
        return Err("Gemini 未返回有效文本内容".to_string());
    }

    Ok(full_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_default_and_custom_urls() {
        assert_eq!(
            build_stream_url("", "gemini-3.8-flash"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            build_generate_url("", "gemini-3.8-flash"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
        );
        assert_eq!(
            build_stream_url("https://proxy.example.com/", "models/gemini-4-hydrogen"),
            "https://proxy.example.com/v1beta/models/gemini-4-hydrogen:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            build_generate_url("https://proxy.example.com/", "models/gemini-4-hydrogen"),
            "https://proxy.example.com/v1beta/models/gemini-4-hydrogen:generateContent"
        );
    }

    #[test]
    fn parses_sse_chunks_skips_thought_parts_and_ignores_heartbeats() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"thought":true,"text":"内部思考"},{"text":"本周"},{"text":"总结"}]}}]}"#;
        assert_eq!(parse_sse_data_line(line), Some("本周总结".to_string()));
        assert_eq!(
            parse_sse_data_line(r#"data: {"candidates":[{"content":{"parts":[{"thought":true,"text":"仅思考"}]}}]}"#),
            None
        );
        assert_eq!(parse_sse_data_line("data: [DONE]"), None);
        assert_eq!(parse_sse_data_line(": keepalive"), None);
        assert_eq!(parse_sse_data_line("data: {broken"), None);
    }

    #[test]
    fn extracts_api_error_messages() {
        let json_err = r#"{"error":{"code":400,"message":"API key not valid."}}"#;
        assert_eq!(extract_error_message(json_err), "API key not valid.");
        assert_eq!(extract_error_message("  Gateway Timeout  "), "Gateway Timeout");
    }

    #[test]
    fn formats_common_http_errors_in_chinese() {
        let msg_401 = format_http_error(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"Invalid key"}}"#,
        );
        assert!(msg_401.contains("API Key 无效"));

        let msg_404 = format_http_error(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"error":{"message":"Model not found"}}"#,
        );
        assert!(msg_404.contains("模型名称无效"));

        let msg_429 = format_http_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"Quota exceeded"}}"#,
        );
        assert!(msg_429.contains("配额不足"));

        let msg_503 = format_http_error(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            r#"{"error":{"message":"High demand"}}"#,
        );
        assert!(msg_503.contains("高峰期拥堵"));
    }

    #[test]
    fn parses_windows_proxy_server_formats() {
        assert_eq!(
            parse_windows_proxy_server("127.0.0.1:7897"),
            Some("http://127.0.0.1:7897".to_string())
        );
        assert_eq!(
            parse_windows_proxy_server("http=127.0.0.1:7890;https=127.0.0.1:7897"),
            Some("http://127.0.0.1:7897".to_string())
        );
        assert_eq!(
            parse_windows_proxy_server("http://127.0.0.1:8080"),
            Some("http://127.0.0.1:8080".to_string())
        );
        assert_eq!(parse_windows_proxy_server("   "), None);
        assert_eq!(parse_windows_proxy_server("socks=127.0.0.1:7897"), None);
    }
}
