//! Profile store: one entry per instance, with the token. Plain JSON file
//! under `$XDG_CONFIG_HOME/vcsstar/profiles.json`, mode 0600.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_digest: Option<String>,
}

pub fn path() -> PathBuf {
    if let Ok(p) = std::env::var("VCSSTAR_PROFILES") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        });
    base.join("vcsstar").join("profiles.json")
}

pub fn load() -> Result<Store> {
    let p = path();
    match std::fs::read(&p) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", p.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
    }
}

pub fn save(store: &Store) -> Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = p.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(store)?;
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

impl Store {
    pub fn select(&self, name: Option<&str>) -> Result<(String, &Profile)> {
        let name = name
            .map(str::to_string)
            .or_else(|| std::env::var("VCSSTAR_PROFILE").ok())
            .or_else(|| self.default.clone())
            .ok_or_else(|| {
                anyhow!("no profile selected: run `vcsstar login <url>` or set VCSSTAR_PROFILE")
            })?;
        let p = self
            .profiles
            .get(&name)
            .ok_or_else(|| anyhow!("profile {name} does not exist"))?;
        Ok((name, p))
    }
}
