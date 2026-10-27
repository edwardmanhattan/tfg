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
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;

/// The name this client reports itself under. Attribution only — the bearer
/// token is what authorizes — but a watching operator sees `from tfgctl`
/// instead of a generic `remote`.
const CLIENT_ID: &str = "tfgctl";

/// How often a display says it is still there. The server reaps after thirty
/// seconds, so two missed beats are the margin.
const DISPLAY_HEARTBEAT: Duration = Duration::from_secs(10);

const USAGE: &str = "\
usage: tfgctl [--server URL] [--token TOKEN] [--json] [--timeout 3s] <command>

commands:
  hello                                   protocol version and revision
  status                                  the shared state, as the server sees it
  camera get [--view NAME]                the camera a view renders
  camera left|right|up|down               nudge by a quarter viewport
  camera zoom-in|zoom-out                 one zoom step
  camera set --lat LAT --lon LON [--zoom Z]
                                          move to a place (keeps the zoom if omitted)
  watch                                   stream events until interrupted

observation (what the phone and the command center see):
  session status                          the exercise, its phase, readiness
  session watch [--until ready] [--interval 1s]
                                          print readiness transitions; with
                                          --until ready, exit 0 once ready
  session set --file PATH                 publish a projection (`-` reads stdin)
  session clear                           say the console holds no exercise
  units                                   the hulls a viewer may look at
  focus N | --id ID [--zoom Z]            centre the observer view on a unit
  display list                            command centers attached right now
  display attach [--id ID] [--label L] [--width W --height H]
                                          hold this terminal open as a command
                                          center (needs the operator's token)
  panel                                   which observer windows are open
  panel crew|scenario|comms on|off        open or close one on the wall

  --view NAME                             which view camera/focus commands are
                                          about; `main` (default) or `observer`

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
    let view = take_option(&mut args, "--view");

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
            camera(
                &client,
                &server,
                &token,
                action,
                rest,
                view.as_deref(),
                json_out,
            )
        }
        [command, action, rest @ ..] if command == "session" => {
            let token = require_token(token)?;
            let mut rest = rest.to_vec();
            match action.as_str() {
                "status" => session_status(&client, &server, &token, json_out),
                "watch" => {
                    let until = take_option(&mut rest, "--until");
                    let until_ready = match until.as_deref() {
                        None => false,
                        Some("ready") => true,
                        Some(other) => {
                            return Err(format!(
                                "session watch --until takes `ready`, not {other:?}"
                            ));
                        }
                    };
                    let interval = match take_option(&mut rest, "--interval") {
                        Some(raw) => parse_duration(&raw)?,
                        None => Duration::from_secs(1),
                    };
                    if !rest.is_empty() {
                        return Err(format!("unexpected arguments after watch: {rest:?}"));
                    }
                    session_watch(&client, &server, &token, until_ready, interval, json_out)
                }
                "set" => {
                    let file = take_option(&mut rest, "--file")
                        .ok_or_else(|| "session set needs --file PATH (`-` reads stdin)".to_string())?;
                    session_set(&client, &server, &token, &file, json_out)
                }
                "clear" => session_clear(&client, &server, &token, json_out),
                other => Err(format!(
                    "unknown session command {other}; use status, watch, set, or clear"
                )),
            }
        }
        [command] if command == "units" => {
            let token = require_token(token)?;
            units(&client, &server, &token, json_out)
        }
        [command, rest @ ..] if command == "focus" => {
            let token = require_token(token)?;
            focus(
                &client,
                &server,
                &token,
                rest,
                view.as_deref(),
                json_out,
            )
        }
        [command, action, rest @ ..] if command == "display" => {
            let token = require_token(token)?;
            display(&server, &token, action, rest, timeout, json_out)
        }
        [command, rest @ ..] if command == "panel" => {
            let token = require_token(token)?;
            panel(&client, &server, &token, rest, json_out)
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
    view: Option<&str>,
    json_out: bool,
) -> Result<(), String> {
    match action {
        "get" => {
            let view = view.unwrap_or("main");
            let state = fetch_state(client, server, token)?;
            let camera = camera_of(&state, view)?;
            if json_out {
                println!("{camera}");
            } else {
                println!("{}", camera_line(camera));
            }
            Ok(())
        }
        "left" | "west" => nudge(client, server, token, 0.25, 0.0, view, json_out),
        "right" | "east" => nudge(client, server, token, -0.25, 0.0, view, json_out),
        "up" | "north" => nudge(client, server, token, 0.0, 0.25, view, json_out),
        "down" | "south" => nudge(client, server, token, 0.0, -0.25, view, json_out),
        "zoom-in" => zoom(client, server, token, 1.0, view, json_out),
        "zoom-out" => zoom(client, server, token, -1.0, view, json_out),
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
                None => camera_of(&fetch_state(client, server, token)?, view.unwrap_or("main"))?["zoom"]
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
                command_body(
                    view,
                    "camera.set",
                    json!({
                        "center": { "latitude": latitude, "longitude": longitude },
                        "zoom": zoom,
                    }),
                ),
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
    view: Option<&str>,
    json_out: bool,
) -> Result<(), String> {
    post(
        client,
        server,
        token,
        command_body(
            view,
            "camera.nudge",
            json!({ "dx_fraction": dx_fraction, "dy_fraction": dy_fraction }),
        ),
        json_out,
    )
}

fn zoom(
    client: &Client,
    server: &str,
    token: &str,
    delta: f64,
    view: Option<&str>,
    json_out: bool,
) -> Result<(), String> {
    post(
        client,
        server,
        token,
        command_body(view, "camera.zoom", json!({ "delta": delta })),
        json_out,
    )
}

/// One command body. `view` is absent unless the operator named one, so a
/// plain `tfgctl camera left` still talks about the console.
fn command_body(view: Option<&str>, kind: &str, payload: Value) -> Value {
    let mut body = json!({
        "client_id": CLIENT_ID,
        "command_id": unique_id(),
        "type": kind,
        "payload": payload,
    });
    if let Some(view) = view {
        body["view"] = json!(view);
    }
    body
}

/// `panel`: the observer's windows. With no arguments, list them; with
/// `crew|scenario|comms on|off`, open or close one on every attached wall.
fn panel(
    client: &Client,
    server: &str,
    token: &str,
    rest: &[String],
    json_out: bool,
) -> Result<(), String> {
    let state = fetch_state(client, server, token)?;
    if rest.is_empty() {
        let ui = &state["ui"];
        if json_out {
            println!("{ui}");
            return Ok(());
        }
        let mut open: Vec<&str> = Vec::new();
        for name in ["crew", "scenario", "comms"] {
            if ui["panels"][name].as_bool().unwrap_or(false) {
                open.push(name);
            }
        }
        if open.is_empty() {
            println!("no observer window is open");
        } else {
            println!("open: {}", open.join(", "));
        }
        return Ok(());
    }
    if rest.len() != 2 {
        return Err("panel takes `crew|scenario|comms on|off`, or nothing to list".to_string());
    }
    let name = rest[0].as_str();
    if !matches!(name, "crew" | "scenario" | "comms") {
        return Err(format!(
            "unknown panel {name:?}; the observer has crew, scenario and comms"
        ));
    }
    let open = match rest[1].as_str() {
        "on" | "open" | "true" => true,
        "off" | "close" | "false" => false,
        other => return Err(format!("panel state must be on or off, not {other:?}")),
    };
    post(
        client,
        server,
        token,
        command_body(
            Some("observer"),
            "ui.set",
            json!({ "panel": name, "open": open }),
        ),
        json_out,
    )
}

/// `GET /v1/session`: the exercise the console projected, and whether a phone
/// may steer this second.
fn session_status(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let response = client
        .get(format!("{server}/v1/session"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    if json_out {
        println!("{value}");
        return Ok(());
    }
    println!("{}", session_summary(&value));
    Ok(())
}

/// `session watch`: readiness, one line per transition.
///
/// A poll rather than the event stream, because readiness is what a waiting
/// screen asks for and the phone asks for it the same way: `GET /v1/session`.
/// Printing only transitions is the point — a line a second saying the same
/// thing is a log nobody reads.
fn session_watch(
    client: &Client,
    server: &str,
    token: &str,
    until_ready: bool,
    interval: Duration,
    json_out: bool,
) -> Result<(), String> {
    let mut printed: Option<String> = None;
    loop {
        let response = client
            .get(format!("{server}/v1/session"))
            .bearer_auth(token)
            .send()
            .map_err(|error| format!("request failed: {error}"))?;
        let value = read_json(response)?;
        let line = observation_line(&value);
        if printed.as_deref() != Some(line.as_str()) {
            // `--json` prints the whole read, not a sentence: the caller that
            // asked for JSON wants the fields, and one object per transition
            // keeps the stream parseable line by line.
            if json_out {
                println!("{value}");
            } else {
                println!("{line}");
            }
            printed = Some(line);
        }
        let ready = value["readiness"]["ready"].as_bool().unwrap_or(false);
        if ready && until_ready {
            return Ok(());
        }
        std::thread::sleep(interval);
    }
}

/// What a waiting screen says, in one line. The same four reasons the wire
/// carries, spelled as the phone spells them.
fn observation_line(value: &Value) -> String {
    let ready = value["readiness"]["ready"].as_bool().unwrap_or(false);
    let session = &value["session"];
    if session.is_null() {
        return "waiting  no session".to_string();
    }
    let id = session["game_id"].as_i64().unwrap_or(0);
    let name = session["name"].as_str().unwrap_or("?");
    let phase = session["phase"].as_str().unwrap_or("?");
    if ready {
        let units = session["units"].as_array().map(Vec::len).unwrap_or(0);
        // The viewport is the display's, not the console's: a phone steering
        // the observer view nudges by fractions of what the command center
        // says it can show.
        let viewport = value["displays"]
            .as_array()
            .and_then(|displays| {
                displays
                    .iter()
                    .find_map(|display| display["viewport"].as_object())
            })
            .map(|viewport| {
                format!(
                    "{}x{}",
                    viewport["width"].as_u64().unwrap_or(0),
                    viewport["height"].as_u64().unwrap_or(0)
                )
            })
            .unwrap_or_else(|| "unknown".to_string());
        return format!(
            "ready    session {id} \u{00b7} observer {viewport} \u{00b7} {units} unit(s)"
        );
    }
    match value["readiness"]["reason"].as_str().unwrap_or("") {
        "no_display" => format!(
            "waiting  session {id} \"{name}\" in {phase}, no command center attached"
        ),
        "stale" => format!(
            "waiting  session {id} \"{name}\": the console has stopped refreshing it"
        ),
        "not_execution" => format!("waiting  session {id} \"{name}\" is {phase}"),
        _ => format!("waiting  session {id} \"{name}\" in {phase}"),
    }
}

/// The one-line verdict both `session status` and `status` print.
fn session_summary(value: &Value) -> String {
    let readiness = &value["readiness"];
    let ready = readiness["ready"].as_bool().unwrap_or(false);
    let reason = readiness["reason"].as_str();
    let session = &value["session"];
    let phase = session["phase"].as_str().unwrap_or("none");
    let name = session["name"].as_str().unwrap_or("no exercise");
    let units = session["units"].as_array().map(Vec::len).unwrap_or(0);
    let displays = value["displays"].as_array().map(Vec::len).unwrap_or(0);
    let verdict = if ready {
        "ready".to_string()
    } else {
        format!("waiting ({})", reason.unwrap_or("not_ready"))
    };
    format!(
        "{name} [{phase}]  {units} unit(s)  {displays} display(s)  {verdict}"
    )
}

/// `PUT /v1/session`: publish the console's projection.
///
/// The body is a whole session, not a patch, and it is also the heartbeat: a
/// console that re-sends the same projection is saying "still this", and the
/// hub answers `unchanged` without publishing an event.
fn session_set(
    client: &Client,
    server: &str,
    token: &str,
    file: &str,
    json_out: bool,
) -> Result<(), String> {
    let raw = if file == "-" {
        let mut raw = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw)
            .map_err(|error| format!("reading stdin: {error}"))?;
        raw
    } else {
        std::fs::read_to_string(file).map_err(|error| format!("reading {file}: {error}"))?
    };
    let body: Value =
        serde_json::from_str(&raw).map_err(|error| format!("{file} is not JSON: {error}"))?;
    let response = client
        .put(format!("{server}/v1/session"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    print_value(&value, json_out);
    Ok(())
}

fn session_clear(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let response = client
        .delete(format!("{server}/v1/session"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    print_value(&value, json_out);
    Ok(())
}

/// The hulls a viewer may look at, numbered the way the phone's grid numbers
/// them. The number is the hub's, not this tool's: that is the point of it.
fn units(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let response = client
        .get(format!("{server}/v1/session"))
        .bearer_auth(token)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let value = read_json(response)?;
    if json_out {
        println!("{}", value["session"]["units"]);
        return Ok(());
    }
    let Some(list) = value["session"]["units"].as_array() else {
        println!("no exercise is open");
        return Ok(());
    };
    if list.is_empty() {
        println!("the exercise has no units");
        return Ok(());
    }
    // Column widths from the data, so a long hull name widens its column
    // instead of pushing every other one out of line.
    let rows: Vec<[String; 6]> = list
        .iter()
        .map(|unit| {
            [
                unit["index"].as_u64().unwrap_or(0).to_string(),
                unit["id"].as_i64().unwrap_or(0).to_string(),
                unit["name"].as_str().unwrap_or("?").to_string(),
                unit["hull_number"].as_str().unwrap_or("").to_string(),
                unit["class_name"].as_str().unwrap_or("").to_string(),
                unit_note(unit),
            ]
        })
        .collect();
    let mut widths = [1usize; 6];
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let line = |cells: [&str; 6]| {
        format!(
            "{:>w0$}  {:>w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
            cells[0],
            cells[1],
            cells[2],
            cells[3],
            cells[4],
            cells[5],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
            w4 = widths[4],
        )
    };
    println!("{}", line(["#", "id", "name", "hull", "class", "note"]));
    for row in &rows {
        println!(
            "{}",
            line([&row[0], &row[1], &row[2], &row[3], &row[4], &row[5]])
        );
    }
    Ok(())
}

/// The facts about a unit that do not fit a column: who commands it, what it
/// carries, where it is riding, and whether its fix is live.
fn unit_note(unit: &Value) -> String {
    let mut notes = Vec::new();
    if unit["position"].is_object() {
        notes.push(format!(
            "{:.3},{:.3}",
            unit["position"]["latitude"].as_f64().unwrap_or(0.0),
            unit["position"]["longitude"].as_f64().unwrap_or(0.0)
        ));
    } else {
        notes.push("no fix".to_string());
    }
    if let Some(commander) = unit["commander"]["name"].as_str() {
        notes.push(format!("CO {commander}"));
    }
    if let Some(aboard) = unit["embarked_in"]["name"].as_str() {
        notes.push(format!("aboard {aboard}"));
    }
    if let Some(aboard) = unit["embarked"].as_array()
        && !aboard.is_empty()
    {
        notes.push(format!(
            "carries {}",
            aboard
                .iter()
                .filter_map(|child| child["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if unit["stale"].as_bool().unwrap_or(false) {
        notes.push("stale fix".to_string());
    }
    notes.join("; ")
}

/// `camera.focus`: centre a view on one hull.
///
/// The unit is named by the index the grid shows or by its id; the server
/// resolves either against the session it holds, so this never sends a
/// position that could be out of date.
fn focus(
    client: &Client,
    server: &str,
    token: &str,
    rest: &[String],
    view: Option<&str>,
    json_out: bool,
) -> Result<(), String> {
    let mut rest = rest.to_vec();
    let by_id = take_option(&mut rest, "--id");
    let zoom = match take_option(&mut rest, "--zoom") {
        Some(raw) => Some(
            raw.parse::<f64>()
                .map_err(|_| "--zoom is not a number".to_string())?,
        ),
        None => None,
    };
    let unit_id = match by_id {
        Some(raw) => raw
            .parse::<i64>()
            .map_err(|_| "--id is not a unit id".to_string())?,
        None => {
            let index = rest
                .first()
                .ok_or_else(|| "focus needs a unit number, or --id ID".to_string())?
                .parse::<u64>()
                .map_err(|_| "the unit number is not a number".to_string())?;
            rest.remove(0);
            let state = fetch_state(client, server, token)?;
            state["session"]["units"]
                .as_array()
                .and_then(|units| {
                    units
                        .iter()
                        .find(|unit| unit["index"].as_u64() == Some(index))
                })
                .and_then(|unit| unit["id"].as_i64())
                .ok_or_else(|| format!("the session has no unit {index}"))?
        }
    };
    if !rest.is_empty() {
        return Err(format!("unexpected arguments after focus: {rest:?}"));
    }
    let mut payload = json!({ "unit_id": unit_id });
    if let Some(zoom) = zoom {
        payload["zoom"] = json!(zoom);
    }
    post(
        client,
        server,
        token,
        command_body(view.or(Some("observer")), "camera.focus", payload),
        json_out,
    )
}

/// `GET /v1/displays`, and the terminal that *is* one.
fn display(
    server: &str,
    token: &str,
    action: &str,
    rest: &[String],
    timeout: Duration,
    json_out: bool,
) -> Result<(), String> {
    match action {
        "list" => {
            let client = Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|error| format!("HTTP client: {error}"))?;
            let response = client
                .get(format!("{server}/v1/displays"))
                .bearer_auth(token)
                .send()
                .map_err(|error| format!("request failed: {error}"))?;
            let value = read_json(response)?;
            if json_out {
                println!("{value}");
                return Ok(());
            }
            let displays = value["displays"].as_array().cloned().unwrap_or_default();
            if displays.is_empty() {
                println!("no command center is attached");
                return Ok(());
            }
            for display in &displays {
                let viewport = if display["viewport"].is_object() {
                    format!(
                        "{}x{}",
                        display["viewport"]["width"].as_u64().unwrap_or(0),
                        display["viewport"]["height"].as_u64().unwrap_or(0)
                    )
                } else {
                    "no viewport".to_string()
                };
                println!(
                    "{:<24} {:<16} {}",
                    display["id"].as_str().unwrap_or("?"),
                    display["role"].as_str().unwrap_or("?"),
                    viewport
                );
            }
            Ok(())
        }
        "attach" => {
            let mut rest = rest.to_vec();
            let id = take_option(&mut rest, "--id").unwrap_or_else(|| "tfgctl".to_string());
            let label = take_option(&mut rest, "--label").unwrap_or_else(|| id.clone());
            let width = take_option(&mut rest, "--width")
                .map(|raw| raw.parse::<u32>())
                .transpose()
                .map_err(|_| "--width is not a number".to_string())?
                .unwrap_or(1280);
            let height = take_option(&mut rest, "--height")
                .map(|raw| raw.parse::<u32>())
                .transpose()
                .map_err(|_| "--height is not a number".to_string())?
                .unwrap_or(800);
            if !rest.is_empty() {
                return Err(format!("unexpected arguments after display attach: {rest:?}"));
            }
            display_attach(server, token, &id, &label, width, height, timeout, json_out)
        }
        other => Err(format!(
            "unknown display command {other}; use list or attach"
        )),
    }
}

/// Hold a WebSocket open as a command center.
///
/// The attachment is the socket: the server records this display the moment it
/// says hello and forgets it the moment this process exits. That is why the
/// phone waits for a command center before it may steer, and why this command
/// is enough to test the whole observation path without a second desktop.
fn display_attach(
    server: &str,
    token: &str,
    id: &str,
    label: &str,
    width: u32,
    height: u32,
    timeout: Duration,
    json_out: bool,
) -> Result<(), String> {
    let base = server
        .strip_prefix("http://")
        .ok_or_else(|| format!("the server URL must start with http://, got {server}"))?;
    let (host, port) = split_host_port(base)?;
    let mut socket = connect_websocket(&host, port, token, timeout)?;
    // A read timeout is what lets one loop both wait for events and beat the
    // heartbeat. Without it a quiet stream would let the display expire.
    let _ = socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(1)));
    let hello = json!({
        "type": "display.hello",
        "payload": {
            "id": id,
            "role": "command_center",
            "label": label,
            "viewport": { "width": width, "height": height },
        }
    });
    socket
        .send(tungstenite::Message::text(hello.to_string()))
        .map_err(|error| format!("sending display.hello: {error}"))?;

    let mut attached = false;
    let mut last_beat = Instant::now();
    loop {
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let value: Value = match serde_json::from_str(text.as_str()) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                match value["type"].as_str().unwrap_or_default() {
                    "display.rejected" => {
                        return Err(format!(
                            "the server refused this display: {}",
                            value["message"].as_str().unwrap_or("no reason given")
                        ));
                    }
                    "display.attached" => {
                        attached = true;
                        if json_out {
                            println!("{value}");
                        } else {
                            println!(
                                "attached as {} ({label}); Ctrl-C to leave",
                                value["id"].as_str().unwrap_or(id)
                            );
                        }
                    }
                    "session.changed" => {
                        if json_out {
                            println!("{value}");
                        } else {
                            println!("{}", session_summary(&value));
                        }
                    }
                    "camera.changed" if value["view"]["id"].as_str() == Some("observer") => {
                        if !json_out {
                            println!("{}", describe_event(&value));
                        }
                    }
                    _ => {}
                }
            }
            Ok(tungstenite::Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(format!("the event stream stopped: {error}")),
        }
        if attached && last_beat.elapsed() >= DISPLAY_HEARTBEAT {
            let beat = json!({ "type": "display.heartbeat", "payload": { "id": id } });
            socket
                .send(tungstenite::Message::text(beat.to_string()))
                .map_err(|error| format!("sending display.heartbeat: {error}"))?;
            last_beat = Instant::now();
        }
    }
}

fn status(client: &Client, server: &str, token: &str, json_out: bool) -> Result<(), String> {
    let state = fetch_state(client, server, token)?;
    if json_out {
        println!("{state}");
        return Ok(());
    }
    let revision = state["revision"].as_u64().unwrap_or(0);
    let version = state["protocol_version"].as_u64().unwrap_or(0);
    println!("server    {server}");
    println!("protocol  v{version}, revision {revision}");
    for view in ["main", "observer"] {
        let Ok(camera) = camera_of(&state, view) else {
            continue;
        };
        println!("camera    {}  {}", pad(view, 8), camera_line(camera));
    }
    println!("exercise  {}", session_summary(&state));
    Ok(())
}

/// `latitude, longitude at zoom Z (WxH)`, the one camera phrasing every verb
/// shares.
fn camera_line(camera: &Value) -> String {
    format!(
        "{:.5}, {:.5} at zoom {:.2} ({}x{})",
        camera["center"]["latitude"].as_f64().unwrap_or(0.0),
        camera["center"]["longitude"].as_f64().unwrap_or(0.0),
        camera["zoom"].as_f64().unwrap_or(0.0),
        camera["width"].as_u64().unwrap_or(0),
        camera["height"].as_u64().unwrap_or(0)
    )
}

fn pad(text: &str, width: usize) -> String {
    format!("{text:<width$}")
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

fn camera_of<'a>(state: &'a Value, view: &str) -> Result<&'a Value, String> {
    let camera = &state["views"][view]["camera"];
    if camera.is_object() {
        Ok(camera)
    } else {
        Err(format!("the server's state has no `{view}` view camera"))
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

    /// The waiting screen's four reasons, and the ready line that replaces
    /// them. This is what `session watch` prints, so a wrong word here is a
    /// wrong word on the phone's waiting card.
    #[test]
    fn a_watch_line_names_the_reason_it_is_waiting() {
        assert_eq!(
            observation_line(&json!({
                "session": null,
                "readiness": { "ready": false, "reason": "no_session" },
                "displays": []
            })),
            "waiting  no session"
        );
        let planning = json!({
            "session": { "game_id": 42, "name": "Exercise North", "phase": "planning", "units": [] },
            "readiness": { "ready": false, "reason": "not_execution" },
            "displays": []
        });
        assert_eq!(
            observation_line(&planning),
            "waiting  session 42 \"Exercise North\" is planning"
        );
        let alone = json!({
            "session": { "game_id": 42, "name": "Exercise North", "phase": "execution", "units": [{"id": 1}] },
            "readiness": { "ready": false, "reason": "no_display" },
            "displays": []
        });
        assert_eq!(
            observation_line(&alone),
            "waiting  session 42 \"Exercise North\" in execution, no command center attached"
        );
        let ready = json!({
            "session": { "game_id": 42, "name": "Exercise North", "phase": "execution", "units": [{"id": 1}, {"id": 2}] },
            "readiness": { "ready": true },
            "displays": [{ "id": "wall-1", "label": "wall-1", "role": "command_center",
                           "viewport": { "width": 1920, "height": 1080 } }]
        });
        assert_eq!(
            observation_line(&ready),
            "ready    session 42 \u{00b7} observer 1920x1080 \u{00b7} 2 unit(s)"
        );
    }
}
