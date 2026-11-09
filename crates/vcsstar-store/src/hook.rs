//! The pre-receive hook protocol. git runs the hook; the hook is a thin
//! client that forwards the ref updates to the serving process over a Unix
//! socket and relays the verdict. All policy lives in the server.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookRefUpdate {
    pub old: String,
    pub new: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRequest {
    pub push_id: String,
    pub hook: String,
    pub updates: Vec<HookRefUpdate>,
    /// Environment git set for the hook: the quarantine object directories
    /// the server needs to inspect the incoming objects.
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResponse {
    pub ok: bool,
    /// Lines for the pusher (git shows them prefixed with `remote:`).
    pub messages: Vec<String>,
}

pub const ZERO_ID: &str = "0000000000000000000000000000000000000000";
pub const ENV_PUSH_ID: &str = "VCSSTAR_PUSH_ID";
pub const ENV_SOCKET: &str = "VCSSTAR_HOOK_SOCKET";

const FORWARDED_ENV: &[&str] = &[
    "GIT_QUARANTINE_PATH",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PUSH_OPTION_COUNT",
];

/// Parse the hook's stdin ("old new ref" per line).
pub fn parse_updates(input: &str) -> Vec<HookRefUpdate> {
    input
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(HookRefUpdate {
                old: it.next()?.to_string(),
                new: it.next()?.to_string(),
                name: it.next()?.to_string(),
            })
        })
        .collect()
}

/// Run as the hook process: read stdin, call the server, print messages,
/// return the exit code.
pub fn client_main(hook: &str) -> i32 {
    let push_id = match std::env::var(ENV_PUSH_ID) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("vcsstar: hook invoked outside a vcsstar push");
            return 1;
        }
    };
    let socket = match std::env::var(ENV_SOCKET) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("vcsstar: hook socket not configured");
            return 1;
        }
    };
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("vcsstar: cannot read hook input");
        return 1;
    }
    let mut env = Vec::new();
    for key in FORWARDED_ENV {
        if let Ok(v) = std::env::var(key) {
            env.push((key.to_string(), v));
        }
    }
    if let Ok(n) = std::env::var("GIT_PUSH_OPTION_COUNT") {
        if let Ok(n) = n.parse::<usize>() {
            for i in 0..n {
                let k = format!("GIT_PUSH_OPTION_{i}");
                if let Ok(v) = std::env::var(&k) {
                    env.push((k, v));
                }
            }
        }
    }
    let req = HookRequest {
        push_id,
        hook: hook.to_string(),
        updates: parse_updates(&input),
        env,
    };
    match call(&socket, &req) {
        Ok(resp) => {
            for m in &resp.messages {
                eprintln!("{m}");
            }
            if resp.ok {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("vcsstar: hook call failed: {e}");
            1
        }
    }
}

pub fn call(socket: &str, req: &HookRequest) -> std::io::Result<HookResponse> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(600)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut line = serde_json::to_vec(req).map_err(std::io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp)?;
    serde_json::from_str(&resp).map_err(std::io::Error::other)
}

/// Server side: read one request line from an accepted connection.
pub fn read_request(stream: &mut UnixStream) -> std::io::Result<HookRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

pub fn write_response(stream: &mut UnixStream, resp: &HookResponse) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(resp).map_err(std::io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stdin() {
        let u = parse_updates(&format!(
            "{ZERO_ID} abc refs/heads/main\nabc def refs/tags/v1\n"
        ));
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].old, ZERO_ID);
        assert_eq!(u[1].name, "refs/tags/v1");
    }
}
