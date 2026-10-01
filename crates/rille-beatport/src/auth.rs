//! Signing in the way beatport.com does: a session cookie from the login
//! form, an authorization code for Beatport's own client id, then OAuth
//! tokens. Only the tokens are kept; the password never touches the disk.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ureq::Agent;

use crate::{API, Error, Result, api_error, net};

/// Beatport's public web client (as used by beatportdl).
pub(crate) const CLIENT_ID: &str = "ryZ8LuyQVPqbK2mBX2Hwt4qSMtnWuTYSqBPO92yQ";

/// Refresh this long before the access token runs out.
const REFRESH_MARGIN_SECS: i64 = 300;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Lifetime of the access token in seconds, from `issued_at`.
    pub expires_in: i64,
    #[serde(default)]
    pub issued_at: i64,
    /// Who signed in, for display.
    #[serde(default)]
    pub username: String,
}

impl Token {
    pub(crate) fn expiring(&self, now: i64) -> bool {
        now + REFRESH_MARGIN_SECS >= self.issued_at + self.expires_in
    }

    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// Writes the token readable only by the user (write, then rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(self).map_err(std::io::Error::other)?.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(tmp, path)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// `agent` must not follow redirects: the code is in the redirect itself.
pub(crate) fn login(agent: &Agent, username: &str, password: &str) -> Result<Token> {
    // 1. The login form sets a session cookie.
    let res = crate::with_headers(agent.post(format!("{API}/auth/login/")))
        .send_json(serde_json::json!({ "username": username, "password": password }))
        .map_err(net)?;
    // Success may also come as a redirect (as beatportdl accepts it).
    if !(res.status().is_success() || res.status().is_redirection()) {
        let status = res.status().as_u16();
        return Err(match api_error(res) {
            Error::Http { detail, .. } if status == 400 || status == 401 || status == 403 => Error::Login(detail),
            e => e,
        });
    }
    let session = res
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| cookie_value(c, "sessionid"))
        .ok_or_else(|| Error::Login("no session cookie in the login response".into()))?;

    // 2. Authorizing the web client with that session redirects to a code.
    let res = crate::with_headers(agent.get(format!("{API}/auth/o/authorize/")))
        .query("client_id", CLIENT_ID)
        .query("response_type", "code")
        .header("cookie", format!("sessionid={session}"))
        .call()
        .map_err(net)?;
    let code = res
        .headers()
        .get("location")
        .and_then(|l| l.to_str().ok())
        .and_then(|l| query_param(l, "code"))
        .ok_or_else(|| Error::Login(format!("no authorization code (HTTP {})", res.status().as_u16())))?;

    // 3. The code buys the tokens.
    let mut token =
        token_request(agent, &[("client_id", CLIENT_ID), ("grant_type", "authorization_code"), ("code", &code)])?;
    token.username = username.to_owned();
    Ok(token)
}

pub(crate) fn refresh(agent: &Agent, old: &Token) -> Result<Token> {
    let mut token = token_request(
        agent,
        &[("client_id", CLIENT_ID), ("grant_type", "refresh_token"), ("refresh_token", &old.refresh_token)],
    )?;
    token.username.clone_from(&old.username);
    Ok(token)
}

fn token_request(agent: &Agent, form: &[(&str, &str)]) -> Result<Token> {
    let mut res =
        crate::with_headers(agent.post(format!("{API}/auth/o/token/"))).send_form(form.iter().copied()).map_err(net)?;
    if !res.status().is_success() {
        return Err(api_error(res));
    }
    let t: TokenResponse = res.body_mut().read_json().map_err(|e| Error::Decode(e.to_string()))?;
    Ok(Token {
        access_token: t.access_token,
        refresh_token: t.refresh_token,
        expires_in: t.expires_in,
        issued_at: now(),
        username: String::new(),
    })
}

/// `name`'s value in a `Set-Cookie` header.
fn cookie_value(set_cookie: &str, name: &str) -> Option<String> {
    let (k, v) = set_cookie.split(';').next()?.split_once('=')?;
    (k.trim() == name && !v.trim().is_empty()).then(|| v.trim().to_owned())
}

/// A query parameter of a URL, percent-decoded.
fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next()?;
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name && !v.is_empty()).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let hex = |c: u8| char::from(c).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let escaped = (b[i] == b'%' && i + 2 < b.len()).then(|| Some(hex(b[i + 1])? * 16 + hex(b[i + 2])?)).flatten();
        match (escaped, b[i]) {
            (Some(v), _) => {
                out.push(v);
                i += 3;
                continue;
            }
            (None, b'+') => out.push(b' '),
            (None, c) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_from_redirect() {
        let loc = "https://www.beatport.com/oauth/callback?code=AbC%2Fd123&state=x";
        assert_eq!(query_param(loc, "code").as_deref(), Some("AbC/d123"));
        assert_eq!(query_param("/?state=1", "code"), None);
        assert_eq!(query_param("/cb?code=", "code"), None);
        assert_eq!(query_param("/cb?code=ab%2", "code").as_deref(), Some("ab%2"));
    }

    #[test]
    fn session_cookie() {
        let c = "sessionid=abc123; expires=Thu, 01 Jan 2099 00:00:00 GMT; HttpOnly; Path=/";
        assert_eq!(cookie_value(c, "sessionid").as_deref(), Some("abc123"));
        assert_eq!(cookie_value("csrftoken=x; Path=/", "sessionid"), None);
    }

    #[test]
    fn token_file_and_expiry() {
        let dir = std::env::temp_dir().join(format!("rille-bp-test-{}", std::process::id()));
        let path = dir.join("token.json");
        let t = Token {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_in: 36000,
            issued_at: 1000,
            username: "dj".into(),
        };
        t.save(&path).unwrap();
        assert_eq!(Token::load(&path), Some(t.clone()));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!t.expiring(1000 + 36000 - 301));
        assert!(t.expiring(1000 + 36000 - 300));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
