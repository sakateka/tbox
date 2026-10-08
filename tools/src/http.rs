use crate::config::{Config, HttpProfile};
use anyhow::{Context, Result, bail};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::{
    Url,
    blocking::Client,
    header::{AUTHORIZATION, HeaderName, HeaderValue},
};
use std::{collections::BTreeMap, fs, time::Duration};

const COMPONENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Placeholders encode one URL component. `{key:path}` preserves path separators.
/// Scheme and authority are always supplied by the configured template.
pub fn render(template: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let authority_end = template
        .find("://")
        .and_then(|i| template[i + 3..].find(['/', '?', '#']).map(|j| i + 3 + j))
        .unwrap_or(template.len());
    let mut output = String::new();
    let mut offset = 0;
    while let Some(relative_start) = template[offset..].find('{') {
        let start = offset + relative_start;
        if start < authority_end {
            bail!("URL placeholders must occur in the path, query or fragment");
        }
        output.push_str(&template[offset..start]);
        let end = template[start + 1..]
            .find('}')
            .map(|i| start + 1 + i)
            .context("Unclosed URL placeholder")?;
        let key = &template[start + 1..end];
        let (key, path) = key
            .strip_suffix(":path")
            .map_or((key, false), |key| (key, true));
        let value = vars
            .get(key)
            .with_context(|| format!("Missing URL variable '{key}'"))?;
        let in_fragment = template[..start].contains('#');
        let in_query = template[..start].contains('?');
        if path && (in_fragment || in_query) {
            bail!(":path is only valid in the URL path");
        }
        if !in_fragment
            && !in_query
            && if path {
                value
                    .split('/')
                    .any(|segment| matches!(segment, "." | ".."))
            } else {
                matches!(value.as_str(), "." | "..")
            }
        {
            bail!("URL path variables must not contain dot-only path segments");
        }
        if path {
            output.push_str(
                &value
                    .split('/')
                    .map(|segment| utf8_percent_encode(segment, COMPONENT).to_string())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        } else {
            output.push_str(&utf8_percent_encode(value, COMPONENT).to_string());
        }
        offset = end + 1;
    }
    if template[offset..].contains('}') {
        bail!("Unmatched URL placeholder delimiter");
    }
    output.push_str(&template[offset..]);
    let url = Url::parse(&output).map_err(|_| anyhow::anyhow!("Configured URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("Configured URL must be absolute HTTP(S) without embedded credentials");
    }
    Ok(url.into())
}

fn authorization(profile: &HttpProfile) -> Result<Option<HeaderValue>> {
    if profile.token_env.is_some() && profile.token_file.is_some() {
        bail!("Configure only one token reference per HTTP profile");
    }
    let token =
        if let Some(name) = &profile.token_env {
            Some(std::env::var(name).map_err(|_| {
                anyhow::anyhow!("Configured token environment variable is unavailable")
            })?)
        } else if let Some(path) = &profile.token_file {
            Some(
                fs::read_to_string(path)
                    .map_err(|_| anyhow::anyhow!("Cannot read configured token file"))?,
            )
        } else {
            None
        };
    let Some(token) = token else {
        return Ok(None);
    };
    // Token files may have one terminal newline, but never embedded control characters.
    let token = token.trim_end_matches(['\r', '\n']);
    if token.is_empty() || !token.bytes().all(|c| c.is_ascii_graphic()) {
        bail!("Configured token is invalid");
    }
    if profile.auth_scheme.eq_ignore_ascii_case("basic")
        || profile.auth_scheme.is_empty()
        || !profile
            .auth_scheme
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        bail!("Configured authorization scheme is invalid");
    }
    let mut value = HeaderValue::from_str(&format!("{} {token}", profile.auth_scheme))
        .map_err(|_| anyhow::anyhow!("Configured token is invalid"))?;
    value.set_sensitive(true);
    Ok(Some(value))
}

pub fn request(config: &Config, name: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let profile = config
        .http
        .get(name)
        .with_context(|| format!("Missing HTTP profile '{name}'"))?;
    let url = render(&profile.url, vars)?;
    if profile.timeout_secs == 0 || profile.timeout_secs > 3600 {
        bail!("HTTP timeout_secs must be between 1 and 3600");
    }
    let auth = authorization(profile)?;
    let mut client = Client::builder()
        .timeout(Duration::from_secs(profile.timeout_secs))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(path) = &profile.ca_file {
        let pem =
            fs::read(path).map_err(|_| anyhow::anyhow!("Cannot read configured CA bundle"))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|_| anyhow::anyhow!("Configured CA bundle is invalid"))?;
        if certs.is_empty() {
            bail!("Configured CA bundle is empty");
        }
        for cert in certs {
            client = client.add_root_certificate(cert);
        }
    }
    let client = client
        .build()
        .map_err(|_| anyhow::anyhow!("Cannot initialize HTTP client"))?;
    let method = reqwest::Method::from_bytes(profile.method.as_bytes())
        .map_err(|_| anyhow::anyhow!("Configured HTTP method is invalid"))?;
    let mut request = client.request(method, url);
    for (key, value) in &profile.headers {
        let key = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| anyhow::anyhow!("Configured HTTP header name is invalid"))?;
        if key == AUTHORIZATION {
            bail!("Use token_env or token_file for Authorization");
        }
        let value = HeaderValue::from_str(value)
            .map_err(|_| anyhow::anyhow!("Configured HTTP header value is invalid"))?;
        request = request.header(key, value);
    }
    if let Some(auth) = auth {
        request = request.header(AUTHORIZATION, auth);
    }
    if let Some(body) = &profile.body {
        request = request.body(body.clone());
    }
    let response = request.send().map_err(|e| {
        if e.is_timeout() {
            anyhow::anyhow!("HTTP request timed out")
        } else {
            anyhow::anyhow!("HTTP request failed (connection or TLS verification)")
        }
    })?;
    if !response.status().is_success() {
        bail!(
            "HTTP request returned status {}",
            response.status().as_u16()
        );
    }
    let bytes = response
        .bytes()
        .map_err(|_| anyhow::anyhow!("Cannot read HTTP response"))?;
    String::from_utf8(bytes.to_vec()).context("HTTP response is not UTF-8")
}
