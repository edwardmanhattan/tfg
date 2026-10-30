//! `tfg-server`, the control plane with no window.
//!
//! The embedded host (`tfg`) binds the control plane to a running console: the
//! server reads the camera the shell is rendering, and commands go down a
//! channel to the shell, which moves the map a person can see. This binary is
//! the other deployment: there is no shell, so the process itself owns the
//! camera and resolves commands with [`ControlHub::apply`] — the same
//! Mercator arithmetic and the same zoom rails the shell uses, compiled from
//! the same modules.
//!
//! What that buys: a machine with no display can host the map's *state* for
//! phones, tablets and other operators, and `tfgctl` (or any client speaking
//! `docs/control-api-v1.md`) can drive it. What it does not buy: tiles, marks
//! or a picture. This is a headless state host, not a renderer, and saying so
//! matters — a client that expects `camera.changed` to mean "somebody
//! redrew" would be reading an event about a camera nobody is drawing.
//!
//! ```text
//! tfg-server                                  # loopback:8787, generated token
//! tfg-server --listen 0.0.0.0:8787 --token …  # on the LAN
//! tfg-server --ephemeral                      # forget every device on exit
//! ```

use std::process::ExitCode;
use std::sync::mpsc;

use tfg::control::{
    AcceptedCommand, CameraSnapshot, CameraSource, Command, ControlHub, ServerConfig, spawn_server,
};

const USAGE: &str = "\
usage: tfg-server [--listen ADDR] [--token TOKEN] [--devices PATH] [--ephemeral]
                  [--lat LAT] [--lon LON] [--zoom Z]

  --listen ADDR    default 127.0.0.1:8787 (or TFG_CONTROL_LISTEN)
  --token TOKEN    bearer token for the operator (or TFG_CONTROL_TOKEN).
                   Generated and printed when absent.
  --devices PATH   paired-device table (or TFG_CONTROL_DEVICES)
                   default $XDG_DATA_HOME/tfg/control-devices.json
  --ephemeral      keep the device table in memory; nothing survives exit
  --lat/--lon/--zoom
                   the camera this host starts at; defaults to the console's";

/// Loopback is the default, and it is a decision rather than a placeholder: a
/// headless server started by accident must not be reachable from the LAN.
const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tfg-server: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(());
    }
    let ephemeral = take_flag(&mut args, "--ephemeral");

    let listen = take_option(&mut args, "--listen")
        .or_else(|| std::env::var("TFG_CONTROL_LISTEN").ok())
        .filter(|listen| !listen.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());

    let supplied_token = take_option(&mut args, "--token")
        .or_else(|| std::env::var("TFG_CONTROL_TOKEN").ok())
        .filter(|token| !token.is_empty());
    // A generated token is only ever for loopback. On a LAN the operator has
    // to have made the decision to expose a port, and should be holding the
    // credential that goes with it rather than reading one off a log.
    let lan = !is_loopback(&listen);
    let token = match (&supplied_token, lan) {
        (Some(token), _) => token.clone(),
        (None, false) => generate_token()?,
        (None, true) => {
            return Err(
                "binding a non-loopback address needs --token (or TFG_CONTROL_TOKEN); \
                 a generated token on the LAN would be a credential printed to a terminal"
                    .to_string(),
            );
        }
    };

    let devices = if ephemeral {
        None
    } else {
        match take_option(&mut args, "--devices")
            .or_else(|| std::env::var("TFG_CONTROL_DEVICES").ok())
            .filter(|path| !path.is_empty())
        {
            Some(path) => Some(std::path::PathBuf::from(path)),
            None => tfg::control::Devices::default_path(),
        }
    };

    let camera = match camera_from_args(&mut args)? {
        Some(camera) => camera,
        None => CameraSnapshot::from_tuple((
            tfg::map_geometry::MAP_CENTER.0,
            tfg::map_geometry::MAP_CENTER.1,
            tfg::map_geometry::MAP_ZOOM,
            tfg::map_geometry::MAP_W as u32,
            tfg::map_geometry::MAP_H as u32,
        )),
    };
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {args:?}\n\n{USAGE}"));
    }

    let hub = ControlHub::new(camera);
    let (command_tx, commands) = mpsc::channel::<AcceptedCommand>();
    let handle = spawn_server(
        ServerConfig {
            listen: listen.clone(),
            token: token.clone(),
            devices: devices.clone(),
        },
        command_tx,
        hub.clone(),
    )?;

    // The banner is the only place a person learns where this is reachable and
    // how to pair. Printed before the drain loop so it appears even if the
    // first client arrives immediately.
    eprintln!("tfg-server listening on {}", handle.url());
    eprintln!("  reachable at {}", handle.advertised_url());
    eprintln!(
        "  devices    {}",
        match &devices {
            Some(path) => path.display().to_string(),
            None if ephemeral => "in memory (--ephemeral)".to_string(),
            // `Devices::default_path()` is None without HOME or XDG_DATA_HOME.
            // Saying so is better than a paired phone that forgets itself.
            None => "in memory (no HOME or XDG_DATA_HOME)".to_string(),
        }
    );
    eprintln!(
        "  camera     {:.5}, {:.5} zoom {:.2} ({}x{})",
        camera.center.latitude, camera.center.longitude, camera.zoom, camera.width, camera.height
    );
    if supplied_token.is_none() {
        eprintln!();
        eprintln!("  operator token  {token}");
        eprintln!("  (generated for this run; pass --token or TFG_CONTROL_TOKEN to fix it)");
    }
    eprintln!();
    eprintln!(
        "  pair a device   tfgctl --server {} --token … pair",
        handle.url()
    );
    eprintln!("  then            tfgctl --server … --token … status");

    drain(commands, hub)
}

/// The one writer in a headless deployment: every accepted command is resolved
/// here, on this thread, and nothing else touches the hub's camera.
///
/// `recv` returns only when every sender is dropped, which is when the server
/// thread has gone away. That is the process's signal to exit rather than sit
/// in front of a port nobody is listening on.
fn drain(commands: mpsc::Receiver<AcceptedCommand>, hub: ControlHub) -> Result<(), String> {
    loop {
        let accepted = match commands.recv() {
            Ok(accepted) => accepted,
            Err(_) => return Err("the control server stopped; exiting".to_string()),
        };
        let source = CameraSource {
            client_id: accepted.client_id.clone(),
            command_id: Some(accepted.command_id.clone()),
        };
        let changed = hub.apply(&accepted.command, source);
        let camera = hub.camera();
        match changed {
            Some(revision) => eprintln!(
                "r{revision}  {:<22} {:.5}, {:.5} zoom {:.2}  from {} ({})",
                describe(&accepted.command),
                camera.center.latitude,
                camera.center.longitude,
                camera.zoom,
                accepted.client_id,
                accepted.command_id
            ),
            None => eprintln!(
                "        {:<22} no change                from {} ({})",
                describe(&accepted.command),
                accepted.client_id,
                accepted.command_id
            ),
        }
    }
}

fn describe(command: &Command) -> String {
    match command {
        Command::CameraSet { center, zoom } => {
            format!(
                "camera.set {:.4},{:.4}@{zoom:.2}",
                center.latitude, center.longitude
            )
        }
        Command::CameraNudge {
            dx_fraction,
            dy_fraction,
        } => format!("camera.nudge {dx_fraction:+.2},{dy_fraction:+.2}"),
        Command::CameraZoom { delta } => format!("camera.zoom {delta:+.2}"),
        Command::CameraFocus { unit_id, zoom } => match zoom {
            Some(zoom) => format!("camera.focus unit {unit_id} @{zoom:.2}"),
            None => format!("camera.focus unit {unit_id}"),
        },
        Command::CameraFocusGroup { group_id, zoom } => match zoom {
            Some(zoom) => format!("camera.focus_group group {group_id} @{zoom:.2}"),
            None => format!("camera.focus_group group {group_id}"),
        },
        Command::ViewReport { width, height } => format!("view.report {width}x{height}"),
        Command::UiSet { panel, open } => {
            format!("ui.set {panel} {}", if *open { "open" } else { "close" })
        }
        Command::SettingsFont { step } => format!("settings.font step {step}"),
    }
}

fn camera_from_args(args: &mut Vec<String>) -> Result<Option<CameraSnapshot>, String> {
    let latitude = take_option(args, "--lat");
    let longitude = take_option(args, "--lon");
    let zoom = take_option(args, "--zoom");
    if latitude.is_none() && longitude.is_none() && zoom.is_none() {
        return Ok(None);
    }
    let fallback = CameraSnapshot::from_tuple((
        tfg::map_geometry::MAP_CENTER.0,
        tfg::map_geometry::MAP_CENTER.1,
        tfg::map_geometry::MAP_ZOOM,
        tfg::map_geometry::MAP_W as u32,
        tfg::map_geometry::MAP_H as u32,
    ));
    let latitude = parse_or(latitude, fallback.center.latitude, "--lat")?;
    let longitude = parse_or(longitude, fallback.center.longitude, "--lon")?;
    let zoom = parse_or(zoom, fallback.zoom, "--zoom")?;
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(format!("--lat must be between -90 and 90, got {latitude}"));
    }
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(format!(
            "--lon must be between -180 and 180, got {longitude}"
        ));
    }
    // The same rails the reducer clamps to, so a start outside them would be
    // silently moved rather than honoured.
    if !(tfg::view::ZOOM_MIN..=tfg::view::ZOOM_MAX).contains(&zoom) {
        return Err(format!(
            "--zoom must be between {} and {}, got {zoom}",
            tfg::view::ZOOM_MIN,
            tfg::view::ZOOM_MAX
        ));
    }
    Ok(Some(CameraSnapshot {
        center: tfg::control::GeoPoint {
            latitude,
            longitude,
        },
        zoom,
        ..fallback
    }))
}

fn parse_or(raw: Option<String>, fallback: f64, flag: &str) -> Result<f64, String> {
    match raw {
        Some(raw) => raw
            .parse::<f64>()
            .map_err(|error| format!("{flag} is not a number ({raw}): {error}")),
        None => Ok(fallback),
    }
}

/// Loopback detection that does not need the address to resolve: a DNS name
/// here would be an address the operator chose on purpose, and a name that
/// points at loopback is still a bind somebody meant to make reachable.
fn is_loopback(listen: &str) -> bool {
    let host = match listen.rsplit_once(':') {
        // Bare `[::1]:8787` style.
        Some((host, _port)) => host,
        None => listen,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "127.0.0.1" | "::1" | "localhost") || host.starts_with("127.")
}

/// A 256-bit token from the system's entropy, printed once.
fn generate_token() -> Result<String, String> {
    use std::io::Read;
    let mut buffer = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut buffer))
        .map_err(|error| format!("no /dev/urandom for an operator token: {error}"))?;
    Ok(buffer.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn take_option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    if index + 1 >= args.len() {
        args.remove(index);
        return None;
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Some(value)
}
