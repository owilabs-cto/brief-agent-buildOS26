use anyhow::{Context, Result, bail};
use axum::http::HeaderMap;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

const TOLERANCE_SECS: i64 = 5 * 60;

pub fn verify_webhook_signature(
    webhook_secret: &str,
    headers: &HeaderMap,
    body: &str,
) -> Result<()> {
    let webhook_id = required_header(headers, "webhook-id")?;
    let webhook_timestamp = required_header(headers, "webhook-timestamp")?;
    let signature_header = required_header(headers, "webhook-signature")?;

    let timestamp: i64 = webhook_timestamp
        .parse()
        .context("invalid webhook timestamp")?;
    let now = chrono::Utc::now().timestamp();
    if (now - timestamp).abs() > TOLERANCE_SECS {
        bail!("timestamp outside tolerance zone");
    }

    let signed_payload = format!("{webhook_id}.{webhook_timestamp}.{body}");

    let mut digests: Vec<[u8; 32]> = Vec::new();
    for key in derive_key_candidates(webhook_secret)? {
        type HmacSha256 = Hmac<Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(&key).context("HMAC key derivation invalid length")?;
        mac.update(signed_payload.as_bytes());
        let bytes = mac.finalize().into_bytes();
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&bytes);
        digests.push(buf);
    }

    for raw_sig in signature_header.split_whitespace() {
        let Some(b64) = raw_sig.strip_prefix("v1,") else {
            continue;
        };
        let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(b64) else {
            continue;
        };
        if decoded.len() != 32 {
            continue;
        }
        for digest in &digests {
            if digest.ct_eq(&decoded[..]).unwrap_u8() == 1 {
                return Ok(());
            }
        }
    }

    bail!("signature mismatch");
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .with_context(|| format!("missing {name} header"))
}

fn derive_key_candidates(secret: &str) -> Result<Vec<Vec<u8>>> {
    if secret.is_empty() {
        bail!("webhook secret is empty");
    }
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(2);
    if let Some(suffix) = secret.strip_prefix("whsec_")
        && let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(suffix)
    {
        out.push(decoded);
    }
    out.push(secret.as_bytes().to_vec());
    Ok(out)
}
