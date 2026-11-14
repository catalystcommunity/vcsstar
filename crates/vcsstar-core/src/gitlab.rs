//! Read project metadata for the GitLab Git mirror prototype.

use serde::Deserialize;

use crate::github::Failure;

#[derive(Deserialize)]
pub struct Project {
    pub default_branch: Option<String>,
    pub description: Option<String>,
    pub visibility: String,
}

pub fn project(host: &str, path: &str, token: Option<&str>) -> Result<Project, Failure> {
    // Encode the full path as one API parameter, including subgroup separators.
    let id: String = path
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .redirects(0)
        .build();
    let url = format!("https://{host}/api/v4/projects/{id}");
    let mut request = agent.get(&url).set("Accept", "application/json");
    if let Some(token) = token {
        request = request.set("PRIVATE-TOKEN", token);
    }
    match request.call() {
        Ok(response) => {
            let body = response
                .into_string()
                .map_err(|_| Failure::Transient("could not read GitLab project response".into()))?;
            serde_json::from_str(&body)
                .map_err(|_| Failure::Terminal("invalid GitLab project response".into()))
        }
        Err(ureq::Error::Status(code, _)) => {
            let message = format!("GitLab project {path} on {host}: HTTP {code}");
            if code == 429 || code >= 500 {
                Err(Failure::Transient(message))
            } else {
                Err(Failure::Terminal(message))
            }
        }
        Err(_) => Err(Failure::Transient(format!(
            "could not connect to GitLab on {host}"
        ))),
    }
}
