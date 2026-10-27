//! `tfgctl`, the reference client for the TFG control API.
//!
//! It speaks the documented v1 contract and nothing else: no linking against
//! the desktop's internals, no knowledge of GPUI. That is the point — the
//! mobile client will speak the same HTTP JSON and WebSocket messages, so every
//! quirk this CLI works around is a quirk a real client would meet too.
//!
//! Configuration, in precedence order: flags, environment, then
//! `~/.config/tfgctl/config.json`:
//!
//! ```json
//! { "server": "http://127.0.0.1:8787", "token": "…" }
//! ```
//!
//! A token in a config file is a local convenience for a loopback bind. On a
//! LAN, pass `--token` or `TFG_CONTROL_TOKEN` instead.

use std::env;
use std::io::{IsTerminal, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::ExitCode;
use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;

/// The name this client reports itself under. Attribution only — the bearer
/// token is what authorizes — but a watching operator sees `from tfgctl`
/// instead of a generic `remote`.
const CLIENT_ID: &str = "tfgctl";

const USAGE: &str = "\
usage: tfgctl [--server URL] [--token TOKEN] [--json] [--timeout 3s] <command>

commands:
  hello                                   protocol version and revision
  status                                  the shared state, as the server sees it
  camera get                              the camera the desktop renders
  camera left|right|up|down               nudge by a quarter viewport
  camera zoom-in|zoom-out                 one zoom step
  camera set --lat LAT --lon LON [--zoom Z]
                                          move to a place (keeps the zoom if omitted)
  watch                                   stream events until interrupted

pairing (needs the operator's own token):
  pair [--scopes observe,control_view]
                                          open a pairing window and wait for a
                                          device to use the code
  devices                                 list paired devices
  revoke DEVICE_ID                        make a device's token stop working
  rotate [--save]                         replace this device's own token
  whoami                                  which credential, and what it may do

flags:
  --server URL    default http://127.0.0.1:8787 (or TFG_CONTROL_URL)
  --token TOKEN   bearer token (or TFG_CONTROL_TOKEN)
  --json          stable JSON on stdout, one object per line
  --timeout 3s    request and connect timeout
  --no-color      plain glyphs for the pairing QR (NO_COLOR also works)
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tfgctl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(());
    }

    let json_out = take_flag(&mut args, "--json");
    let no_color = take_flag(&mut args, "--no-color");
    let timeout = match take_option(&mut args, "--timeout") {
        Some(raw) => parse_duration(&raw)?,
        None => Duration::from_secs(3),
    };
    let config = Config::load();
    let save_after_rotate = take_flag(&mut args, "--save");

    let server = take_option(&mut args, "--server")
        .or_else(|| env::var("TFG_CONTROL_URL").ok())
        .or(config.server)
        .unwrap_or_else(|| "http://127.0.0.1:8787".to_string());
    let server = server.trim_end_matches('/').to_string();
    let token = take_option(&mut args, "--token")
        .or_else(|| env::var("TFG_CONTROL_TOKEN").ok())
        .or(config.token);

    let client = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| format!("HTTP client: {error}"))?;

    match args.as_slice() {
        [command] if command == "hello" => {
            get(&client, &server, token.as_deref(), "/v1/hello", json_out)
        }
        [command] if command == "status" => {
            let token = require_token(token)?;
            status(&client, &server, &token, json_out)
        }
        [command] if command == "watch" => {
            let token = require_token(token)?;
            watch(&server, &token, timeout, json_out)
        }
        [command, action, rest @ ..] if command == "camera" => {
            let token = require_token(token)?;
            camera(&client, &server, &token, action, rest, json_out)
        }
        [command] if command == "whoami" => {
            let token = require_token(token)?;
            whoami(&client, &server, &token, json_out)
        }
        [command] if command == "devices" => {
            let token = require_token(token)?;
            devices(&client, &server, &token, json_out)
        }
        [command] if command == "rotate" => {
            let token = require_token(token)?;
            rotate(&client, &server, &token, save_after_rotate, json_out)
        }
        [command, id] if command == "revoke" => {
            let token = require_token(token)?;
            revoke(&client, &server, &token, id, json_out)
        }
        [command, rest @ ..] if command == "pair" => {
            let token = require_token(token)?;
            let mut rest = rest.to_vec();
            let scopes = match take_option(&mut rest, "--scopes") {
                Some(raw) => raw
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
                None => vec!["observe".to_string(), "control_view".to_string()],
            };
            if !rest.is_empty() {
                return Err(format!("unexpected arguments after pair: {rest:?}"));
            }
            pair(
                &client,
                &server,
                &token,
                scopes,
                json_out,
                !no_color && env::var_os("NO_COLOR").is_none(),
            )
        }
        _ => Err(USAGE.trim_end().to_string()),
    }
}

fn camera(
    client: &Client,
    server: &str,
    token: &str,
    action: &str,
    rest: &[String],
    json_out: bool,
) -> Result<(), String> {
    match action {
        "get" => {
            let state = fetch_state(client, server, token)?;
            let camera = camera_of(&state)?;
            if json_out {
                println!("{camera}");
            } else {
                let latitude = camera["center"]["latitude"].as_f64().unwrap_or(0.0);
                let longitude = camera["center"]["longitude"].as_f64().unwrap_or(0.0);
                let zoom = camera["zoom"].as_f64().unwrap_or(0.0);
                let width = camera["width"].as_u64().unwrap_or(0);
                let height = camera["height"].as_u64().unwrap_or(0);
                println!("{latitude:.5}, {longitude:.5} at zoom {zoom:.2} ({width}x{height})");
            }
            Ok(())
        }
        "left" | "west" => nudge(client, server, token, 0.25, 0.0, json_out),
        "right" | "east" => nudge(client, server, token, -0.25, 0.0, json_out),
        "up" | "north" => nudge(client, server, token, 0.0, 0.25, json_out),
        "down" | "south" => nudge(client, server, token, 0.0, -0.25, json_out),
        "zoom-in" => zoom(client, server, token, 1.0, json_out),
        "zoom-out" => zoom(client, server, token, -1.0, json_out),
        "set" => {
            let mut rest = rest.to_vec();
            let latitude = take_option(&mut rest, "--lat")
                .ok_or_else(|| "camera set needs --lat and --lon".to_string())?
                .parse::<f64>()
                .map_err(|_| "--lat is not a number".to_string())?;
            let longitude = take_option(&mut rest, "--lon")
                .ok_or_else(|| "camera set needs --lat and --lon".to_string())?
                .parse::<f64>()
                .map_err(|_| "--lon is not a number".to_string())?;
            let zoom = match take_option(&mut rest, "--zoom") {
                Some(raw) => raw
                    .parse::<f64>()
                    .map_err(|_| "--zoom is not a number".to_string())?,
                // Without a zoom the operator is centring, not scaling: read
                // the current one rather than guessing a number for them.
                None => camera_of(&fetch_state(client, server, token)?)?["zoom"]
                    .as_f64()
                    .ok_or_else(|| "the server's camera has no zoom".to_string())?,
            };
            if !rest.is_empty() {
                return Err(format!("unexpected arguments after camera set: {rest:?}"));
            }
            post(
                client,
                server,
                token,
                json!({
                    "client_id": CLIENT_ID,
                    "command_id": unique_id(),
                    "type": "camera.set",
                    "payload": {
                        "center": { "latitude": latitude, "longitude": longitude },
                        "zoom": zoom,
                    }
                }),
                json_out,
            )
        }
        other => Err(format!(
            "unknown camera command {other}; use get, left, right, up, down, zoom-in, zoom-out, or set"
        )),
    }
}

fn nudge(
    client: &Client,
    server: &str,
    token: &str,
    dx_fraction: f64,
    dy_fraction: f64,
    json_out: bool,
) -> Result<(), String> {
    post(
        client,
        server,
        token,
        json!({
            "client_id": CLIENT_ID,
            "command_id": unique_id(),
            "type": "camera.nudge",
            "payload": { "dx_fraction": dx_fraction, "dy_fraction": dy_fraction }
        }),
        json_out,
    )
}

fn zoom(
    client: &Client,
    server: &str,
    token: &str,
    delta: f64,
    json_out: bool,
) -> Result<(), String> {
    post(
        client,
        server,
        token,
        json!({
            "client_id": CLIENT_ID,
            "command_id": unique_id(),
            "type": "camera.zoom",
            "payload": { "delta": delta }
        }),
        json_out,
    )
}

fn status(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let state = fetch_state(client, server, token)?;
    if json_out {
        println!("{state}");
        return Ok(());
    }
    let revision = state["revision"].as_u64().unwrap_or(0);
    let version = state["protocol_version"].as_u64().unwrap_or(0);
    let camera = camera_of(&state)?;
    let latitude = camera["center"]["latitude"].as_f64().unwrap_or(0.0);
    let longitude = camera["center"]["longitude"].as_f64().unwrap_or(0.0);
    let zoom = camera["zoom"].as_f64().unwrap_or(0.0);
    println!("server    {server}");
    println!("protocol  v{version}, revision {revision}");
    println!("camera    {latitude:.5}, {longitude:.5} at zoom {zoom:.2}");
    Ok(())
}

/// Which credential the server resolved this token to, and what it may do.
///
/// The first thing to run when a command is refused: it separates "wrong
/// token" from "right token, not this scope".
fn whoami(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let response = client
        .get(format!("{server}/v1/whoami"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    if json_out {
        println!("{value}");
        return Ok(());
    }
    let scopes = value["scopes"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|scope| scope.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    println!("client  {}", value["client"].as_str().unwrap_or("?"));
    println!("label   {}", value["label"].as_str().unwrap_or("?"));
    println!("scopes  {scopes}");
    Ok(())
}

fn devices(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let response = client
        .get(format!("{server}/v1/devices"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    if json_out {
        println!("{value}");
        return Ok(());
    }
    let empty = Vec::new();
    let list = value["devices"].as_array().unwrap_or(&empty);
    if list.is_empty() {
        println!("no devices are paired");
        return Ok(());
    }
    for device in list {
        let scopes = device["scopes"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|scope| scope.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        println!(
            "{}  {:<16}  {:<22}  paired {}  {}{}",
            device["id"].as_str().unwrap_or("?"),
            device["label"].as_str().unwrap_or("?"),
            scopes,
            stamp(device["created_at"].as_i64()),
            "seen ",
            ago(device["last_seen"].as_i64()),
        );
    }
    Ok(())
}

fn revoke(
    client: &Client,
    server: &str,
    token: &str,
    id: &str,
    json_out: bool,
) -> Result<(), String> {
    let response = client
        .delete(format!("{server}/v1/devices/{id}"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    if json_out {
        println!("{value}");
    } else {
        println!("revoked {id}; its token stops working now");
    }
    Ok(())
}

/// Replace this credential's own token. The old one dies immediately, so the
/// new one is printed (and optionally written to the config file) rather than
/// being a secret the CLI keeps to itself.
fn rotate(
    client: &Client,
    server: &str,
    token: &str,
    save: bool,
    json_out: bool,
) -> Result<(), String> {
    let response = client
        .post(format!("{server}/v1/auth/rotate"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    let fresh = value["token"]
        .as_str()
        .ok_or_else(|| "the server did not return a token".to_string())?;
    if save {
        Config::save(server, fresh)?;
    }
    if json_out {
        println!("{value}");
    } else {
        println!("rotated {}", value["device"].as_str().unwrap_or("?"));
        println!("token   {fresh}");
        if save {
            println!(
                "saved to {}",
                config_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            );
        } else {
            println!("pass this as --token, or re-run with --save to write the config file");
        }
    }
    Ok(())
}

/// Open a pairing window and wait for the device that uses it.
///
/// The CLI cannot read the token it hands out — that goes to the device — so
/// "paired" means the device appeared in the table.
fn pair(
    client: &Client,
    server: &str,
    token: &str,
    scopes: Vec<String>,
    json_out: bool,
    colour: bool,
) -> Result<(), String> {
    let before = device_ids(client, server, token)?;
    let response = client
        .post(format!("{server}/v1/pairing"))
        .bearer_auth(token)
        .json(&json!({ "scopes": scopes }))
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let offer = read_json(response)?;
    let code = offer["code"]
        .as_str()
        .ok_or_else(|| "the server did not return a pairing code".to_string())?;
    let expires_in = offer["expires_in"].as_u64().unwrap_or(120);
    if json_out {
        println!("{offer}");
    } else {
        println!("pairing code  {code}");
        println!("server        {}", offer["url"].as_str().unwrap_or(server));
        println!("payload       {}", offer["payload"].as_str().unwrap_or(""));
        // A QR code is for a human with a phone, so it only appears when a
        // human is looking at a terminal. Piped output stays parseable and
        // `--json` stays one object per line.
        if let Some(payload) = offer["payload"]
            .as_str()
            .filter(|payload| !payload.is_empty())
        {
            if std::io::stdout().is_terminal() {
                match tfg::qr::encode(payload.as_bytes()) {
                    Ok(matrix) => {
                        println!();
                        print!("{}", matrix.render_terminal(colour));
                    }
                    // The payload comes from the server; an oversize one is a
                    // server bug, not a reason to abandon the pairing.
                    Err(error) => eprintln!("tfgctl: no QR code: {error}"),
                }
            }
        }
        println!();
        println!(
            "Enter that code on the device, or scan the QR code with a phone. \
             It expires in {expires_in}s."
        );
    }
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let after = device_ids(client, server, token)?;
        if let Some((id, name, device_scopes)) = after
            .iter()
            .find(|(id, ..)| !before.iter().any(|(known, ..)| known == id))
        {
            if json_out {
                println!(
                    "{}",
                    json!({ "paired": id, "label": name, "scopes": device_scopes })
                );
            } else {
                println!();
                println!("paired  {id}  {name}  ({device_scopes})");
            }
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(expires_in) {
            return Err(format!(
                "the pairing code expired after {expires_in}s with no device using it"
            ));
        }
    }
}

type DeviceRow = (String, String, String);

fn device_ids(client: &Client, server: &str, token: &str) -> Result<Vec<DeviceRow>, String> {
    let response = client
        .get(format!("{server}/v1/devices"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    let empty = Vec::new();
    Ok(value["devices"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|device| {
            (
                device["id"].as_str().unwrap_or("?").to_string(),
                device["label"].as_str().unwrap_or("?").to_string(),
                device["scopes"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|scope| scope.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default(),
            )
        })
        .collect())
}

/// A unix timestamp as `YYYY-MM-DD HH:MM`, or `-` when the server did not say.
fn stamp(seconds: Option<i64>) -> String {
    let Some(seconds) = seconds else {
        return "-".to_string();
    };
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|at| at.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// How long ago a device was last seen, in words an operator reads quickly.
fn ago(seconds: Option<i64>) -> String {
    let Some(seen) = seconds else {
        return "never".to_string();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(seen);
    let delta = (now - seen).max(0);
    match delta {
        0..=59 => format!("{delta}s ago"),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86_399 => format!("{}h ago", delta / 3600),
        _ => format!("{}d ago", delta / 86_400),
    }
}
fn fetch_state(client: &Client, server: &str, token: &str) -> Result<Value, String> {
    let response = client
        .get(format!("{server}/v1/state"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    read_json(response)
}

fn get(
    client: &Client,
    server: &str,
    token: Option<&str>,
    path: &str,
    json_out: bool,
) -> Result<(), String> {
    let mut request = client.get(format!("{server}{path}"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    print_value(&value, json_out);
    Ok(())
}

fn post(
    client: &Client,
    server: &str,
    token: &str,
    body: Value,
    json_out: bool,
) -> Result<(), String> {
    let response = client
        .post(format!("{server}/v1/commands"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    print_value(&value, json_out);
    Ok(())
}

fn read_json(response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("reading response: {error}"))?;
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::String(body));
    if !status.is_success() {
        let code = value["error"]["code"].as_str().unwrap_or("request_failed");
        let message = value["error"]["message"]
            .as_str()
            .unwrap_or("the server refused the request");
        return Err(format!("HTTP {status} {code}: {message}"));
    }
    Ok(value)
}

fn print_value(value: &Value, json_out: bool) {
    if json_out {
        println!("{value}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    }
}

fn camera_of(state: &Value) -> Result<&Value, String> {
    let camera = &state["views"]["main"]["camera"];
    if camera.is_object() {
        Ok(camera)
    } else {
        Err("the server's state has no main view camera".to_string())
    }
}

/// Read the event stream until the connection ends or the operator interrupts.
///
/// Events are printed one per line in `--json`, and summarised in human mode:
/// a watch that dumped whole snapshots every tick would be unreadable, and the
/// point of watching is to see the camera move.
fn watch(server: &str, token: &str, timeout: Duration, json_out: bool) -> Result<(), String> {
    let base = if let Some(rest) = server.strip_prefix("https://") {
        return Err(format!(
            "wss is not supported yet; point {} at an http server on the LAN",
            rest
        ));
    } else if let Some(rest) = server.strip_prefix("http://") {
        rest.to_string()
    } else {
        return Err(format!(
            "the server URL must start with http://, got {server}"
        ));
    };
    let (host, port) = split_host_port(&base)?;
    let mut socket = connect_websocket(&host, port, token, timeout)?;

    let mut stdout = std::io::stdout();
    loop {
        let message = socket
            .read()
            .map_err(|error| format!("watching the event stream: {error}"))?;
        match message {
            tungstenite::Message::Text(text) => {
                let value: Value = serde_json::from_str(text.as_str())
                    .map_err(|error| format!("event is not JSON: {error}"))?;
                if json_out {
                    let _ = writeln!(stdout, "{value}");
                } else {
                    let _ = writeln!(stdout, "{}", describe_event(&value));
                }
                let _ = stdout.flush();
            }
            tungstenite::Message::Close(_) => return Ok(()),
            tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => {}
            other => {
                let _ = writeln!(stdout, "{other:?}");
                let _ = stdout.flush();
            }
        }
    }
}

fn describe_event(value: &Value) -> String {
    let kind = value["type"].as_str().unwrap_or("event");
    let revision = value["revision"].as_u64().unwrap_or(0);
    match kind {
        "camera.changed" => {
            let camera = &value["view"]["camera"];
            let latitude = camera["center"]["latitude"].as_f64().unwrap_or(0.0);
            let longitude = camera["center"]["longitude"].as_f64().unwrap_or(0.0);
            let zoom = camera["zoom"].as_f64().unwrap_or(0.0);
            let source = value["source"].as_str().unwrap_or("desktop");
            let command = value["command_id"].as_str().unwrap_or("-");
            format!(
                "camera.changed  r{revision}  {latitude:.5}, {longitude:.5} at zoom {zoom:.2}  from {source} ({command})"
            )
        }
        "snapshot" => {
            let camera = &value["views"]["main"]["camera"];
            let latitude = camera["center"]["latitude"].as_f64().unwrap_or(0.0);
            let longitude = camera["center"]["longitude"].as_f64().unwrap_or(0.0);
            let zoom = camera["zoom"].as_f64().unwrap_or(0.0);
            format!("snapshot       r{revision}  {latitude:.5}, {longitude:.5} at zoom {zoom:.2}")
        }
        "command.accepted" => format!(
            "command.accepted  r{revision}  {}",
            value["command_id"].as_str().unwrap_or("-")
        ),
        "command.rejected" => format!(
            "command.rejected  {}  {}",
            value["command_id"].as_str().unwrap_or("-"),
            value["message"].as_str().unwrap_or("refused")
        ),
        other => format!("{other}  {value}"),
    }
}

fn connect_websocket(
    host: &str,
    port: u16,
    token: &str,
    timeout: Duration,
) -> Result<tungstenite::WebSocket<TcpStream>, String> {
    let url = format!("ws://{host}:{port}/v1/ws");
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("websocket request: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| "the token is not a valid header value".to_string())?,
    );
    let address = format!("{host}:{port}");
    let mut last = None;
    for candidate in address
        .to_socket_addrs()
        .map_err(|error| format!("resolving {address}: {error}"))?
    {
        match TcpStream::connect_timeout(&candidate, timeout) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                match tungstenite::client(request.clone(), stream) {
                    Ok((socket, _response)) => return Ok(socket),
                    Err(error) => return Err(format!("websocket handshake: {error}")),
                }
            }
            Err(error) => last = Some(error),
        }
    }
    Err(format!(
        "connecting to {address}: {}",
        last.map(|error| error.to_string())
            .unwrap_or_else(|| "no address resolved".to_string())
    ))
}

fn split_host_port(base: &str) -> Result<(String, u16), String> {
    let (authority, _path) = base.split_once('/').unwrap_or((base, ""));
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((
            host.to_string(),
            port.parse::<u16>()
                .map_err(|_| format!("{port} is not a port"))?,
        )),
        None => Ok((authority.to_string(), 80)),
    }
}

fn require_token(token: Option<String>) -> Result<String, String> {
    token.ok_or_else(|| {
        "missing a token: pass --token, set TFG_CONTROL_TOKEN, or add one to ~/.config/tfgctl/config.json"
            .to_string()
    })
}

fn parse_duration(raw: &str) -> Result<Duration, String> {
    let raw = raw.trim();
    let (number, scale) = if let Some(value) = raw.strip_suffix("ms") {
        (value, 0.001)
    } else if let Some(value) = raw.strip_suffix('s') {
        (value, 1.0)
    } else if let Some(value) = raw.strip_suffix('m') {
        (value, 60.0)
    } else {
        (raw, 1.0)
    };
    let value = number
        .parse::<f64>()
        .map_err(|_| format!("{raw} is not a duration; try 500ms, 2s, or 1m"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{raw} is not a duration"));
    }
    Ok(Duration::from_secs_f64(value * scale))
}

#[derive(Default)]
struct Config {
    server: Option<String>,
    token: Option<String>,
}

impl Config {
    /// Read `~/.config/tfgctl/config.json` if it exists. A malformed file is
    /// reported rather than ignored: a silent fallback to loopback would look
    /// like a server that is not running.
    fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Self {
                server: value["server"].as_str().map(str::to_string),
                token: value["token"].as_str().map(str::to_string),
            },
            Err(error) => {
                eprintln!(
                    "tfgctl: {} is not JSON ({error}); using flags and environment",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write `{ server, token }` back, for `rotate --save`.
    ///
    /// Mode 0600 on the way in, because the file holds a bearer token: the
    /// default umask would leave it readable by anyone on the machine.
    fn save(server: &str, token: &str) -> Result<(), String> {
        let path = config_path().ok_or_else(|| {
            "no HOME or XDG_CONFIG_HOME: pass the new token with --token instead".to_string()
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("creating {}: {error}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(&json!({ "server": server, "token": token }))
            .map_err(|error| format!("encoding the config: {error}"))?;
        std::fs::write(&path, text)
            .map_err(|error| format!("writing {}: {error}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("tfgctl").join("config.json"))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    let found = args.iter().any(|arg| arg == name);
    args.retain(|arg| arg != name);
    found
}

fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.remove(index);
    if index >= args.len() {
        return None;
    }
    Some(args.remove(index))
}

/// Unique per invocation: the server remembers command ids, so two nudges in
/// one second must not share one.
fn unique_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("cli-{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_accept_the_units_an_operator_types() {
        assert_eq!(parse_duration("2s").expect("2s"), Duration::from_secs(2));
        assert_eq!(
            parse_duration("500ms").expect("500ms"),
            Duration::from_millis(500)
        );
        assert_eq!(parse_duration("1m").expect("1m"), Duration::from_secs(60));
        assert_eq!(parse_duration("3").expect("3"), Duration::from_secs(3));
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn a_server_url_splits_into_a_host_and_a_port() {
        assert_eq!(
            split_host_port("127.0.0.1:8787").expect("host"),
            ("127.0.0.1".to_string(), 8787)
        );
        assert_eq!(
            split_host_port("desk.local:8787/some/path").expect("host"),
            ("desk.local".to_string(), 8787)
        );
        assert_eq!(
            split_host_port("desk.local").expect("host"),
            ("desk.local".to_string(), 80)
        );
    }

    #[test]
    fn options_are_taken_from_anywhere_in_the_line() {
        let mut args = vec![
            "camera".to_string(),
            "--server".to_string(),
            "http://x".to_string(),
            "left".to_string(),
        ];
        assert_eq!(
            take_option(&mut args, "--server").as_deref(),
            Some("http://x")
        );
        assert_eq!(args, vec!["camera".to_string(), "left".to_string()]);
    }

    #[test]
    fn an_event_summary_names_the_camera_and_its_cause() {
        let event = json!({
            "type": "camera.changed",
            "revision": 12,
            "source": "phone",
            "command_id": "c-7",
            "view": {
                "id": "main",
                "camera": {
                    "center": { "latitude": -6.2, "longitude": 106.8 },
                    "zoom": 11.5,
                    "width": 800,
                    "height": 600
                }
            }
        });
        let line = describe_event(&event);
        assert!(line.contains("camera.changed"));
        assert!(line.contains("r12"));
        assert!(line.contains("from phone (c-7)"));
    }
}
