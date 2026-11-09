//! Service implementations. Each function takes the app, the request
//! context, and the decoded request, and returns the response record.

pub mod auth;
pub mod login;
pub mod meta;
pub mod orgs;
pub mod repos;

use crate::Error;

/// Validate a name used as a URL segment: org names, repo names, team names.
pub fn check_name(kind: &str, s: &str) -> Result<(), Error> {
    let ok = !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !s.starts_with('.')
        && !s.starts_with('-')
        && s != ".."
        && !s.ends_with(".git");
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!("{kind} name must be 1-100 characters of letters, digits, '-', '_', or '.', not start with '.' or '-', and not end with .git")))
    }
}

/// Parse `user_id@domain` into (uuid, domain).
pub fn parse_identity_key(s: &str) -> Result<(uuid::Uuid, String), Error> {
    let (user, domain) = s
        .rsplit_once('@')
        .ok_or_else(|| Error::invalid("identity must be user_id@domain"))?;
    let id = uuid::Uuid::parse_str(user)
        .map_err(|_| Error::invalid("identity user_id must be a UUID"))?;
    if domain.is_empty() {
        return Err(Error::invalid("identity domain is empty"));
    }
    Ok((id, domain.to_ascii_lowercase()))
}
