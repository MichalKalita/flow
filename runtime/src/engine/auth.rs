//! Resolves a transport credential to an actor the program already stores.
//! Verifies JWT signatures. Does not create users or store passwords.

use crate::{Result, program::Program, value::Value};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::Config;
use super::store::{err, lookup};

pub(in crate::engine) fn authenticate(
    p: &Program,
    db: &Connection,
    config: &Config,
    credential: Option<&str>,
    transport: &str,
) -> Result<(String, Value)> {
    authenticate_candidates(
        p,
        db,
        config,
        credential,
        p.transports
            .get(transport)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    )
}
pub(in crate::engine) fn authenticate_candidates(
    p: &Program,
    db: &Connection,
    config: &Config,
    credential: Option<&str>,
    aliases: &[String],
) -> Result<(String, Value)> {
    if let Some(credential) = credential {
        if credential.len() > 65536 {
            return Err(err("unauthenticated"));
        };
        let (scheme, secret) = credential
            .split_once(' ')
            .ok_or_else(|| err("unauthenticated"))?;
        let mode = match scheme {
            "Bearer" => "jwt",
            "ApiKey" => "apiKey",
            _ => return Err(err("unauthenticated")),
        };
        for auth in p
            .auth
            .iter()
            .filter(|a| a.mode == mode && aliases.contains(&a.alias))
        {
            let subject = if mode == "apiKey" {
                if secret.len() < 32 || secret.len() > 4096 {
                    return Err(err("unauthenticated"));
                };
                format!("{:x}", Sha256::digest(secret))
            } else {
                let Some(key) = config.jwt_keys.get(&auth.alias) else {
                    continue;
                };
                match jwt(secret, key, &auth.issuer, &auth.audience) {
                    Ok(s) => s,
                    Err(_) => continue,
                }
            };
            if let Some(actor) = lookup(db, auth, &subject)? {
                return Ok((auth.entity.clone(), actor.pin()));
            }
        }
        return Err(err("unauthenticated"));
    }
    if p.auth
        .iter()
        .any(|a| a.mode == "anonymous" && aliases.contains(&a.alias))
    {
        Ok(("Anonymous".into(), Value::Null))
    } else {
        Err(err("unauthenticated"))
    }
}
pub(in crate::engine) fn jwt(
    token: &str,
    key: &[u8],
    issuer: &str,
    audience: &str,
) -> Result<String> {
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(err("unauthenticated"));
    };
    let header: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| err("unauthenticated"))?,
    )?;
    if header["alg"] != "HS256" || header.get("crit").is_some() {
        return Err(err("unauthenticated"));
    };
    let signature = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| err("unauthenticated"))?;
    let hmac = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::verify(
        &hmac,
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &signature,
    )
    .map_err(|_| err("unauthenticated"))?;
    let claims: serde_json::Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| err("unauthenticated"))?,
    )?;
    let now = Utc::now().timestamp();
    if claims["iss"] != issuer
        || !(claims["aud"] == audience
            || claims["aud"]
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v == audience)))
        || !claims["exp"].as_i64().is_some_and(|exp| exp > now)
        || claims
            .get("nbf")
            .is_some_and(|n| !n.as_i64().is_some_and(|n| n <= now))
    {
        return Err(err("unauthenticated"));
    };
    let subject = claims["sub"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(|| err("unauthenticated"))?;
    Ok(subject.into())
}
