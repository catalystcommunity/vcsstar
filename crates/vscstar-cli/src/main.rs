//! `vscstar`: the command line for every contract operation.
//!
//! Exit codes: 0 ok, 1 server ServiceError, 2 usage, 3 authentication,
//! 4 transport or network.

mod convert;
mod descriptor;
mod profiles;

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::Value as Json;
use vscstar_transport::client::HttpTransport;
use vscstar_transport::Status;

use descriptor::{Direction, Schema};

const EXIT_SERVICE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_AUTH: u8 = 3;
const EXIT_TRANSPORT: u8 = 4;

#[derive(Debug)]
enum CliError {
    Usage(String),
    Auth(String),
    Transport(String),
    Service(Json),
    Other(anyhow::Error),
}

impl From<anyhow::Error> for CliError {
    fn from(e: anyhow::Error) -> Self {
        CliError::Other(e)
    }
}

fn main() -> ExitCode {
    let schema = match descriptor::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vscstar: embedded schema descriptor is unusable: {e:#}");
            return ExitCode::from(EXIT_TRANSPORT);
        }
    };
    let cmd = build_command(&schema);
    let matches = match cmd.try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { EXIT_USAGE } else { 0 });
        }
    };
    match run(&schema, &matches) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(m)) => {
            eprintln!("vscstar: {m}");
            ExitCode::from(EXIT_USAGE)
        }
        Err(CliError::Auth(m)) => {
            eprintln!("vscstar: {m}");
            ExitCode::from(EXIT_AUTH)
        }
        Err(CliError::Transport(m)) => {
            eprintln!("vscstar: {m}");
            ExitCode::from(EXIT_TRANSPORT)
        }
        Err(CliError::Service(err)) => {
            eprintln!("{}", serde_json::to_string(&err).unwrap_or_default());
            ExitCode::from(EXIT_SERVICE)
        }
        Err(CliError::Other(e)) => {
            eprintln!("vscstar: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn build_command(schema: &Schema) -> Command {
    let mut api = Command::new("api")
        .about("Call any contract operation: vscstar api <Service> <op> [--input FILE|-] [--set k=v]...")
        .subcommand_required(true)
        .arg_required_else_help(true);
    for svc in &schema.services {
        let mut sc = Command::new(svc.name.clone())
            .subcommand_required(true)
            .arg_required_else_help(true);
        for op in &svc.operations {
            let about = match op.direction {
                Direction::Unidirectional => {
                    format!("{} -> {}", ty_name(&op.input), ty_name(&op.output))
                }
                Direction::Bidirectional => {
                    format!("stream: {} <-> {}", ty_name(&op.input), ty_name(&op.output))
                }
                Direction::Reverse => {
                    format!("push: {} <- {}", ty_name(&op.input), ty_name(&op.output))
                }
            };
            sc = sc.subcommand(
                Command::new(op.name.clone())
                    .about(about)
                    .arg(
                        Arg::new("input")
                            .long("input")
                            .short('i')
                            .help("JSON request file, or - for stdin"),
                    )
                    .arg(
                        Arg::new("set")
                            .long("set")
                            .short('s')
                            .action(ArgAction::Append)
                            .help("field=value (dotted paths; value parsed as JSON when valid)"),
                    )
                    .arg(
                        Arg::new("describe")
                            .long("describe")
                            .action(ArgAction::SetTrue)
                            .help("Print the request and response shapes and exit"),
                    ),
            );
        }
        api = api.subcommand(sc);
    }
    Command::new("vscstar")
        .version(env!("CARGO_PKG_VERSION"))
        .about("vscstar command line. Every server operation is available under `api`.")
        .arg(
            Arg::new("profile")
                .long("profile")
                .global(true)
                .env("VSCSTAR_PROFILE")
                .help("Profile name from the profile store"),
        )
        .arg(
            Arg::new("instance")
                .long("instance")
                .global(true)
                .env("VSCSTAR_INSTANCE")
                .help("Instance origin URL; overrides the profile"),
        )
        .arg(
            Arg::new("token-file")
                .long("token-file")
                .global(true)
                .help("File holding the bearer token; overrides the profile"),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .global(true)
                .value_parser(["json", "table", "cbor", "raw"])
                .help("Output format (default: table on a terminal, json otherwise)"),
        )
        .subcommand(api)
        .subcommand(
            Command::new("schema")
                .about("Print the embedded contract: all services and operations, or one")
                .arg(Arg::new("service"))
                .arg(Arg::new("op"))
                .arg(
                    Arg::new("digest")
                        .long("digest")
                        .action(ArgAction::SetTrue)
                        .help("Print only the contract digest"),
                ),
        )
        .subcommand(
            Command::new("login")
                .about("Store a profile for an instance and verify the token")
                .arg(
                    Arg::new("url")
                        .required(true)
                        .help("Instance origin, e.g. https://vscstar.catalystsquad.com"),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .help("Profile name (default: the host)"),
                )
                .arg(
                    Arg::new("token-stdin")
                        .long("token-stdin")
                        .action(ArgAction::SetTrue)
                        .help("Read the bearer token from stdin"),
                )
                .arg(
                    Arg::new("token-file")
                        .long("token-file")
                        .help("Read the bearer token from a file"),
                )
                .arg(Arg::new("device").long("device").action(ArgAction::SetTrue).help("Device flow: approve a short code in your browser (default when no token option is given)"))
                .arg(Arg::new("scopes").long("scopes").default_value("repo:write,pr:write,pr:review,statuses:write,org:read,tokens:manage").help("Comma-separated scopes for the device-flow token"))
                .arg(Arg::new("label").long("label").help("Client label for the device-flow token (default: vscstar-cli@<hostname>)"))
                .arg(
                    Arg::new("no-default")
                        .long("no-default")
                        .action(ArgAction::SetTrue)
                        .help("Do not make this the default profile"),
                ),
        )
        .subcommand(
            Command::new("logout")
                .about("Revoke the profile's token on the server and remove it locally"),
        )
        .subcommand(
            Command::new("whoami").about("Show the identity and scopes of the current credential"),
        )
        .subcommand(
            Command::new("profiles")
                .about("Manage stored profiles")
                .subcommand(Command::new("list"))
                .subcommand(Command::new("use").arg(Arg::new("name").required(true)))
                .subcommand(Command::new("remove").arg(Arg::new("name").required(true))),
        )
        .subcommand(
            Command::new("version").about("Print CLI, contract version, and descriptor digest"),
        )
}

fn ty_name(t: &descriptor::Ty) -> String {
    match t {
        descriptor::Ty::Reference(n) => n.clone(),
        descriptor::Ty::Builtin(b) => b.clone(),
        descriptor::Ty::Choice(arms) => arms.iter().map(ty_name).collect::<Vec<_>>().join(" / "),
        other => format!("{other:?}"),
    }
}

struct Conn {
    url: String,
    transport: HttpTransport,
}

fn connect(m: &ArgMatches, need_auth: bool) -> Result<Conn, CliError> {
    let store = profiles::load()?;
    let profile = store
        .select(m.get_one::<String>("profile").map(String::as_str))
        .ok();
    let url = m
        .get_one::<String>("instance")
        .cloned()
        .or_else(|| profile.as_ref().map(|(_, p)| p.url.clone()))
        .ok_or_else(|| {
            CliError::Usage("no instance: pass --instance URL or run `vscstar login <url>`".into())
        })?;
    let token = if let Some(t) = std::env::var("VSCSTAR_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
    {
        Some(t.trim().to_string())
    } else if let Some(f) = m.get_one::<String>("token-file") {
        Some(
            std::fs::read_to_string(f)
                .with_context(|| format!("read {f}"))?
                .trim()
                .to_string(),
        )
    } else {
        profile.as_ref().and_then(|(_, p)| p.token.clone())
    };
    if need_auth && token.is_none() {
        return Err(CliError::Auth(
            "no credential: set VSCSTAR_TOKEN, pass --token-file, or run `vscstar login`".into(),
        ));
    }
    Ok(Conn {
        transport: HttpTransport::new(&url, token),
        url,
    })
}

/// Call one operation with a JSON request and return the JSON response.
fn call(
    schema: &Schema,
    conn: &Conn,
    service: &str,
    op_name: &str,
    mut request: Json,
) -> Result<Json, CliError> {
    let op = schema
        .operation(service, op_name)
        .ok_or_else(|| CliError::Usage(format!("unknown operation {service}/{op_name}")))?;
    if op.direction != Direction::Unidirectional {
        return Err(CliError::Usage(format!("{service}/{op_name} is a streaming operation; use the events carrier (not available yet)")));
    }
    inject_ctx(schema, &op.input, &mut request);
    let cbor = convert::json_to_cbor(schema, &op.input, &request, "$")
        .map_err(|e| CliError::Usage(format!("request: {e:#}")))?;
    let payload = convert::encode(&cbor)?;
    let response = conn
        .transport
        .call_envelope(service, op_name, payload)
        .map_err(map_client_error)?;
    if response.status != Status::Ok {
        let message = response.error.clone().unwrap_or_default();
        return Err(match response.status {
            Status::Unauthenticated => {
                CliError::Auth(format!("{}: unauthenticated: {message}", conn.url))
            }
            Status::Forbidden => CliError::Auth(format!("{}: forbidden: {message}", conn.url)),
            other => CliError::Transport(format!(
                "{}: transport status {other:?}: {message}",
                conn.url
            )),
        });
    }
    let value = convert::decode(&response.payload)
        .map_err(|e| CliError::Transport(format!("response payload: {e:#}")))?;
    if response.variant.as_deref() == Some("ServiceError") {
        let err_ty = descriptor::Ty::Reference("ServiceError".into());
        let err = convert::cbor_to_json(schema, &err_ty, &value, "$")
            .unwrap_or_else(|_| convert::cbor_any_to_json(&value));
        let code = err.get("code").and_then(Json::as_u64).unwrap_or(0);
        if (10..=12).contains(&code) {
            return Err(CliError::Auth(
                err.get("message")
                    .and_then(Json::as_str)
                    .unwrap_or("authentication failed")
                    .to_string(),
            ));
        }
        return Err(CliError::Service(err));
    }
    // The success arm: the output type minus the ServiceError arm.
    let out_ty = success_arm(schema, &op.output);
    convert::cbor_to_json(schema, &out_ty, &value, "$")
        .map_err(|e| CliError::Transport(format!("response: {e:#}")))
}

fn success_arm(schema: &Schema, output: &descriptor::Ty) -> descriptor::Ty {
    if let descriptor::Ty::Choice(arms) = output {
        let non_error: Vec<&descriptor::Ty> = arms
            .iter()
            .filter(|a| schema.arm_name(a).as_deref() != Some("ServiceError"))
            .collect();
        if non_error.len() == 1 {
            return non_error[0].clone();
        }
    }
    output.clone()
}

/// Fill `ctx` when the request type has one and the caller did not set it.
fn inject_ctx(schema: &Schema, input: &descriptor::Ty, request: &mut Json) {
    let has_ctx = match schema.resolve(input) {
        descriptor::Resolved::Group(g, _) => {
            g.entries.iter().any(|e| e.key.as_deref() == Some("ctx"))
        }
        _ => false,
    };
    if !has_ctx {
        return;
    }
    if !request.is_object() {
        *request = Json::Object(Default::default());
    }
    let obj = request.as_object_mut().unwrap();
    let ctx = obj
        .entry("ctx")
        .or_insert_with(|| Json::Object(Default::default()));
    if let Some(c) = ctx.as_object_mut() {
        c.entry("request_id")
            .or_insert_with(|| Json::String(uuid::Uuid::now_v7().to_string()));
        c.entry("contract_version")
            .or_insert_with(|| Json::String(schema.version.clone()));
        c.entry("contract_digest").or_insert_with(|| {
            Json::String(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &schema.digest,
            ))
        });
    }
}

fn map_client_error(e: vscstar_types::ClientError) -> CliError {
    match e {
        vscstar_types::ClientError::Service { code, message } => {
            CliError::Service(serde_json::json!({"code": code, "message": message}))
        }
        vscstar_types::ClientError::Transport(m) => CliError::Transport(m),
    }
}

fn read_request(m: &ArgMatches) -> Result<Json, CliError> {
    let mut request = match m.get_one::<String>("input") {
        Some(path) if path == "-" => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("read stdin")?;
            if s.trim().is_empty() {
                Json::Object(Default::default())
            } else {
                serde_json::from_str(&s).context("stdin is not JSON")?
            }
        }
        Some(path) => {
            let s = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
            serde_json::from_str(&s).with_context(|| format!("{path} is not JSON"))?
        }
        None => Json::Object(Default::default()),
    };
    if let Some(sets) = m.get_many::<String>("set") {
        for kv in sets {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| CliError::Usage(format!("--set expects field=value, got {kv}")))?;
            let value =
                serde_json::from_str::<Json>(v).unwrap_or_else(|_| Json::String(v.to_string()));
            set_path(&mut request, k, value);
        }
    }
    Ok(request)
}

fn set_path(root: &mut Json, path: &str, value: Json) {
    let mut cur = root;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        if !cur.is_object() {
            *cur = Json::Object(Default::default());
        }
        let obj = cur.as_object_mut().unwrap();
        if i == parts.len() - 1 {
            obj.insert((*part).to_string(), value);
            return;
        }
        cur = obj
            .entry(*part)
            .or_insert_with(|| Json::Object(Default::default()));
    }
}

fn output_mode(m: &ArgMatches) -> &str {
    match m.get_one::<String>("output").map(String::as_str) {
        Some(o) => o,
        None if std::io::stdout().is_terminal() => "table",
        None => "json",
    }
}

fn print_json(m: &ArgMatches, value: &Json) -> Result<()> {
    let mut out = std::io::stdout().lock();
    match output_mode(m) {
        "table" => {
            if let Some(text) = render_table(value) {
                out.write_all(text.as_bytes())?;
            } else {
                writeln!(out, "{}", serde_json::to_string_pretty(value)?)?;
            }
        }
        "raw" => match value {
            Json::String(s) => writeln!(out, "{s}")?,
            other => writeln!(out, "{other}")?,
        },
        _ => writeln!(out, "{}", serde_json::to_string_pretty(value)?)?,
    }
    Ok(())
}

/// Render arrays of flat records, or one flat record, as aligned columns.
fn render_table(value: &Json) -> Option<String> {
    let rows: Vec<&serde_json::Map<String, Json>> = match value {
        Json::Array(items) => items
            .iter()
            .map(|i| i.as_object())
            .collect::<Option<Vec<_>>>()?,
        Json::Object(obj) => {
            // A one-key wrapper around an array (e.g. {"repos": [...]}) renders the array.
            if obj.len() <= 2 {
                if let Some((_, Json::Array(items))) = obj.iter().find(|(_, v)| v.is_array()) {
                    return render_table(&Json::Array(items.clone()));
                }
            }
            let mut s = String::new();
            let width = obj.keys().map(String::len).max().unwrap_or(0);
            for (k, v) in obj {
                s.push_str(&format!("{k:width$}  {}\n", scalar(v)));
            }
            return Some(s);
        }
        _ => return None,
    };
    if rows.is_empty() {
        return Some("(empty)\n".into());
    }
    let mut columns: Vec<String> = Vec::new();
    for r in &rows {
        for k in r.keys() {
            if !columns.contains(k) {
                columns.push(k.clone());
            }
        }
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            columns
                .iter()
                .map(|c| scalar(r.get(c).unwrap_or(&Json::Null)))
                .collect()
        })
        .collect();
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            cells
                .iter()
                .map(|r| r[i].len())
                .max()
                .unwrap_or(0)
                .max(c.len())
        })
        .collect();
    let mut s = String::new();
    for (i, c) in columns.iter().enumerate() {
        s.push_str(&format!("{:width$}  ", c.to_uppercase(), width = widths[i]));
    }
    s.push('\n');
    for r in cells {
        for (i, v) in r.iter().enumerate() {
            s.push_str(&format!("{:width$}  ", v, width = widths[i]));
        }
        s.push('\n');
    }
    Some(s)
}

fn scalar(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        Json::Array(a) if a.iter().all(|x| x.is_string()) => a
            .iter()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>()
            .join(","),
        other => other.to_string(),
    }
}

fn run(schema: &Schema, m: &ArgMatches) -> Result<(), CliError> {
    match m.subcommand() {
        Some(("api", am)) => {
            let (service, sm) = am
                .subcommand()
                .ok_or_else(|| CliError::Usage("service required".into()))?;
            let (op_name, om) = sm
                .subcommand()
                .ok_or_else(|| CliError::Usage("operation required".into()))?;
            if om.get_flag("describe") {
                let op = schema.operation(service, op_name).unwrap();
                let desc = serde_json::json!({
                    "service": service, "op": op_name, "wire_id": op.wire_id,
                    "direction": format!("{:?}", op.direction),
                    "input": describe_ty(schema, &op.input, 0),
                    "output": describe_ty(schema, &op.output, 0),
                });
                print_json(m, &desc)?;
                return Ok(());
            }
            let request = read_request(om)?;
            let conn = connect(m, false)?;
            let result = call(schema, &conn, service, op_name, request)?;
            print_json(m, &result)?;
            Ok(())
        }
        Some(("schema", sm)) => {
            if sm.get_flag("digest") {
                println!("{}", hex::encode(&schema.digest));
                return Ok(());
            }
            let value = match (sm.get_one::<String>("service"), sm.get_one::<String>("op")) {
                (Some(s), Some(o)) => {
                    let op = schema
                        .operation(s, o)
                        .ok_or_else(|| CliError::Usage(format!("unknown operation {s}/{o}")))?;
                    serde_json::json!({"service": s, "op": o, "wire_id": op.wire_id, "direction": format!("{:?}", op.direction),
                        "input": describe_ty(schema, &op.input, 0), "output": describe_ty(schema, &op.output, 0)})
                }
                (Some(s), None) => {
                    let svc = schema
                        .service(s)
                        .ok_or_else(|| CliError::Usage(format!("unknown service {s}")))?;
                    serde_json::json!({"service": svc.name, "wire_id": svc.wire_id,
                        "operations": svc.operations.iter().map(|o| serde_json::json!({"op": o.name, "wire_id": o.wire_id,
                            "input": ty_name(&o.input), "output": ty_name(&o.output), "direction": format!("{:?}", o.direction)})).collect::<Vec<_>>()})
                }
                _ => serde_json::json!({
                    "version": schema.version, "digest": hex::encode(&schema.digest),
                    "services": schema.services.iter().map(|svc| serde_json::json!({"service": svc.name, "wire_id": svc.wire_id,
                        "operations": svc.operations.iter().map(|o| serde_json::json!({"op": o.name, "wire_id": o.wire_id,
                            "input": ty_name(&o.input), "output": ty_name(&o.output), "direction": format!("{:?}", o.direction)})).collect::<Vec<_>>()})).collect::<Vec<_>>()
                }),
            };
            let mut forced = m.clone();
            let _ = &mut forced;
            println!(
                "{}",
                serde_json::to_string_pretty(&value).map_err(anyhow::Error::from)?
            );
            Ok(())
        }
        Some(("login", lm)) => login(schema, m, lm),
        Some(("logout", _)) => {
            let conn = connect(m, true)?;
            let _ = call(
                schema,
                &conn,
                "Auth",
                "logout",
                Json::Object(Default::default()),
            )?;
            let mut store = profiles::load()?;
            let name = store
                .select(m.get_one::<String>("profile").map(String::as_str))
                .map(|(n, _)| n)
                .ok();
            if let Some(name) = name {
                if let Some(p) = store.profiles.get_mut(&name) {
                    p.token = None;
                    p.identity = None;
                }
                profiles::save(&store)?;
                eprintln!("logged out of profile {name}");
            }
            Ok(())
        }
        Some(("whoami", _)) => {
            let conn = connect(m, true)?;
            let result = call(
                schema,
                &conn,
                "Auth",
                "whoami",
                Json::Object(Default::default()),
            )?;
            print_json(m, &result)?;
            Ok(())
        }
        Some(("profiles", pm)) => {
            let mut store = profiles::load()?;
            match pm.subcommand() {
                Some(("use", um)) => {
                    let name = um.get_one::<String>("name").unwrap();
                    if !store.profiles.contains_key(name) {
                        return Err(CliError::Usage(format!("profile {name} does not exist")));
                    }
                    store.default = Some(name.clone());
                    profiles::save(&store)?;
                }
                Some(("remove", rm)) => {
                    let name = rm.get_one::<String>("name").unwrap();
                    store.profiles.remove(name);
                    if store.default.as_deref() == Some(name) {
                        store.default = None;
                    }
                    profiles::save(&store)?;
                }
                _ => {
                    let rows: Vec<Json> = store
                        .profiles
                        .iter()
                        .map(|(n, p)| {
                            serde_json::json!({"name": n, "url": p.url, "identity": p.identity, "default": store.default.as_deref() == Some(n.as_str()), "token": p.token.is_some()})
                        })
                        .collect();
                    print_json(m, &Json::Array(rows))?;
                }
            }
            Ok(())
        }
        Some(("version", _)) => {
            println!("vscstar {}", env!("CARGO_PKG_VERSION"));
            println!("contract {}", schema.version);
            println!("digest {}", hex::encode(&schema.digest));
            Ok(())
        }
        _ => Err(CliError::Usage("a command is required; try --help".into())),
    }
}

fn login(schema: &Schema, m: &ArgMatches, lm: &ArgMatches) -> Result<(), CliError> {
    let url = lm
        .get_one::<String>("url")
        .unwrap()
        .trim_end_matches('/')
        .to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(CliError::Usage(
            "url must start with http:// or https://".into(),
        ));
    }
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(&url)
        .split('/')
        .next()
        .unwrap_or(&url)
        .to_string();
    let name = lm.get_one::<String>("name").cloned().unwrap_or(host);
    let token = if lm.get_flag("token-stdin") {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("read token from stdin")?;
        s.trim().to_string()
    } else if let Some(f) = lm.get_one::<String>("token-file") {
        std::fs::read_to_string(f)
            .with_context(|| format!("read {f}"))?
            .trim()
            .to_string()
    } else {
        device_login(schema, &url, lm)?
    };
    if token.is_empty() {
        return Err(CliError::Usage("token is empty".into()));
    }
    let conn = Conn {
        transport: HttpTransport::new(&url, Some(token.clone())),
        url: url.clone(),
    };
    let info = call(
        schema,
        &conn,
        "Meta",
        "server-info",
        Json::Object(Default::default()),
    )?;
    let remote_digest = info
        .get("contract_digest")
        .and_then(Json::as_str)
        .map(str::to_string);
    let local_digest =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &schema.digest);
    if remote_digest.as_deref() != Some(local_digest.as_str()) {
        eprintln!("warning: server contract differs from this CLI (server {}); some operations may be missing or differ", info.get("contract_version").and_then(Json::as_str).unwrap_or("?"));
    }
    let who = call(
        schema,
        &conn,
        "Auth",
        "whoami",
        Json::Object(Default::default()),
    )?;
    let identity = who.get("identity").map(|i| {
        format!(
            "{}@{}",
            i.get("handle").and_then(Json::as_str).unwrap_or("?"),
            i.get("domain").and_then(Json::as_str).unwrap_or("?")
        )
    });
    let mut store = profiles::load()?;
    store.profiles.insert(
        name.clone(),
        profiles::Profile {
            url,
            token: Some(token),
            identity: identity.clone(),
            contract_digest: remote_digest.map(|d| {
                hex::encode(
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, d)
                        .unwrap_or_default(),
                )
            }),
        },
    );
    if !lm.get_flag("no-default") || store.default.is_none() {
        store.default = Some(name.clone());
    }
    profiles::save(&store)?;
    eprintln!("logged in to {name} as {}", identity.unwrap_or_default());
    let _ = m;
    Ok(())
}

/// Device flow: request a code, show it, poll until approved.
fn device_login(schema: &Schema, url: &str, lm: &ArgMatches) -> Result<String, CliError> {
    let anon = Conn {
        transport: HttpTransport::new(url, None),
        url: url.to_string(),
    };
    let scopes: Vec<Json> = lm
        .get_one::<String>("scopes")
        .map(|s| {
            s.split(',')
                .map(|x| Json::String(x.trim().to_string()))
                .filter(|x| x.as_str().is_some_and(|v| !v.is_empty()))
                .collect()
        })
        .unwrap_or_default();
    let label = lm.get_one::<String>("label").cloned().unwrap_or_else(|| {
        let host = std::env::var("HOSTNAME")
            .ok()
            .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
            .map(|h| h.trim().to_string())
            .unwrap_or_else(|| "host".into());
        format!("vscstar-cli@{host}")
    });
    let started = call(
        schema,
        &anon,
        "Auth",
        "begin-device-login",
        serde_json::json!({"client_label": label, "scopes": scopes}),
    )?;
    let device_code = started
        .get("device_code")
        .and_then(Json::as_str)
        .ok_or_else(|| CliError::Transport("server returned no device_code".into()))?
        .to_string();
    let user_code = started
        .get("user_code")
        .and_then(Json::as_str)
        .unwrap_or("?");
    let verification_url = started
        .get("verification_url")
        .and_then(Json::as_str)
        .unwrap_or(url);
    let interval = started
        .get("poll_interval_ms")
        .and_then(Json::as_u64)
        .unwrap_or(3000)
        .clamp(1000, 30000);
    let expires_at = started
        .get("expires_at")
        .and_then(Json::as_i64)
        .unwrap_or(0);
    eprintln!("Open {verification_url}");
    eprintln!("and confirm the code {user_code}");
    if !std::io::stdin().is_terminal() {
        eprintln!("(stdin is not a terminal; waiting for approval without prompting)");
    }
    loop {
        std::thread::sleep(std::time::Duration::from_millis(interval));
        let polled = call(
            schema,
            &anon,
            "Auth",
            "poll-device-login",
            serde_json::json!({"device_code": device_code}),
        )?;
        if polled.get("pending").and_then(Json::as_bool) == Some(false) {
            let secret = polled
                .get("token")
                .and_then(|t| t.get("secret"))
                .and_then(Json::as_str)
                .ok_or_else(|| {
                    CliError::Transport("approved, but the server returned no token secret".into())
                })?;
            return Ok(secret.to_string());
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if expires_at > 0 && now_ms > expires_at {
            return Err(CliError::Auth(
                "the device code expired before it was approved".into(),
            ));
        }
    }
}

/// Human-readable shape of a type for `--describe` and `schema`.
fn describe_ty(schema: &Schema, ty: &descriptor::Ty, depth: usize) -> Json {
    use descriptor::{Occurrence, Resolved, Ty};
    if depth > 6 {
        return Json::String(ty_name(ty));
    }
    match schema.resolve(ty) {
        Resolved::Group(g, name) => {
            let mut fields = serde_json::Map::new();
            for e in &g.entries {
                let key = match &e.key {
                    Some(k) => k.clone(),
                    None => "*".into(),
                };
                let mut inner = describe_ty(schema, &e.value, depth + 1);
                if e.occurrence == Occurrence::Optional {
                    inner = Json::String(format!("? {}", flat(&inner)));
                }
                fields.insert(key, inner);
            }
            let mut obj = serde_json::Map::new();
            if let Some(n) = name {
                obj.insert("$type".into(), Json::String(n.to_string()));
            }
            obj.extend(fields);
            Json::Object(obj)
        }
        Resolved::GroupChoice(_, name) => Json::String(name.unwrap_or("group-choice").to_string()),
        Resolved::Unknown(n) => Json::String(format!("unknown:{n}")),
        Resolved::Ty(t) => match t {
            Ty::Builtin(b) => Json::String(b.clone()),
            Ty::Array { element, .. } => Json::Array(vec![describe_ty(schema, element, depth + 1)]),
            Ty::Map { key, value } => {
                serde_json::json!({ format!("* {}", ty_name(key)): describe_ty(schema, value, depth + 1) })
            }
            Ty::Choice(arms) => Json::Array(
                arms.iter()
                    .map(|a| describe_ty(schema, a, depth + 1))
                    .collect(),
            ),
            Ty::Literal(l) => Json::String(format!("literal {l:?}")),
            other => Json::String(ty_name(other)),
        },
    }
}

fn flat(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Object(o) => o
            .get("$type")
            .and_then(Json::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "record".into()),
        Json::Array(a) => format!("[{}]", a.iter().map(flat).collect::<Vec<_>>().join(" / ")),
        other => other.to_string(),
    }
}

#[allow(dead_code)]
fn unreachable_guard() -> Result<()> {
    bail!("unreachable");
}

#[allow(dead_code)]
fn never(_: ()) -> anyhow::Error {
    anyhow!("never")
}
