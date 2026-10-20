//! The console (ADR-0020, cut over to GPUI at U8 step 5).
//!
//! One package, one toolkit: the egui console is gone, and this is the
//! console — the shell grew a panel at a time until it was. It reads the
//! kernel (`view`, `picker`, `visuals`, `readiness`, `map_gate`, `halo`)
//! and the same threads the old console used.
//!
//! The shell itself is `tfg::shell_gpui`, which `proto/gpui-island` also drives
//! through `#[path]` — so the parity proof and this binary cannot drift.
//!
//! What lives HERE rather than in the shell: everything runtime. The map
//! thread and its feed, the poll thread and the live wire, and the roster's
//! fetch. The shell takes prepared inputs and never names `map_render`,
//! `shell_threads` or `backend` — which is what keeps the parity proto free
//! of MapLibre's 2 GB C++ tree and of the HTTP stack.

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("tfg {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    // Endpoints live in optional env files, never UI fields — the same
    // two loads as the egui console, before paths resolve anything.
    tfg::paths::load_dotenv(None);
    let paths = tfg::paths::AppPaths::discover().expect("data dir available");
    tfg::paths::load_dotenv(Some(&paths));
    if std::env::args().any(|arg| arg == "--check-runtime") {
        if let Err(e) = tfg::selfcheck::runtime_self_check(&paths, "tfg") {
            eprintln!("runtime check failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--probe-gpu") {
        if let Err(e) = tfg::gpuprobe::probe() {
            eprintln!("probe failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    let cache = tfg::map_render::prepare_runtime_cache(&paths.map_cache)
        .expect("map cache prepared");

    // The REAL map thread: the same spawn the egui console uses (U6). It
    // carries raw RGBA; the swizzle to BGRA happens on this side of the
    // handoff, which is the whole point of the seam.
    let (req_tx, resp_rx, _handle) = tfg::shell_threads::spawn_map_thread(
        tfg::shell_threads::MapConfig {
            center: tfg::map_render::MAP_CENTER,
            zoom: tfg::map_render::MAP_ZOOM,
            width: tfg::map_render::MAP_W as u32,
            height: tfg::map_render::MAP_H as u32,
            style: tfg::map_render::MAP_STYLE.to_string(),
            cache,
            jump_pump: tfg::map_render::JUMP_PUMP,
        },
    );
    let mut warmups: u64 = 0;

    // U8 step 4's first panel: the roster sheet's rows are the real hull
    // register. Credentials from the environment, never from the source —
    // the shipped crate carries no secret. The console's real path is the
    // login card plus the OS keyring: a card sign-in leaves a refresh
    // token in the keyring, and the next launch rotates it into a session
    // before the shell opens (see `try_keyring_login`).
    let host = std::env::var("TFG_MINOS_HOST")
        .unwrap_or_else(|_| "https://api.tfg.development.crossnet.co.id/api/v1".to_string());
    // Credentials, in order: the environment (a capture's explicit
    // superuser), then the keyring's stored refresh token (a returning
    // launch skips the card), then the card. `TFG_SIGNED_OUT` wins over
    // all of them — it forces the card for a headless capture.
    let (roster, roster_total, token, user) = if std::env::var("TFG_SIGNED_OUT").as_deref()
        == Ok("1")
    {
        eprintln!("roster: TFG_SIGNED_OUT, the card is forced");
        (tfg::shell_gpui::fixture_roster(), 125, String::new(), String::new())
    } else {
        match (
            std::env::var("TFG_API_USER"),
            std::env::var("TFG_API_PASS"),
        ) {
            (Ok(user), Ok(pass)) => match fetch_roster(&host, &user, &pass) {
                Some((rows, token, _)) => {
                    let total = rows.len();
                    eprintln!("roster: {total} hulls from {host}");
                    (rows, total, token, user)
                }
                None => {
                    eprintln!("roster: fetch failed, falling back to the fixture");
                    (tfg::shell_gpui::fixture_roster(), 125, String::new(), user)
                }
            },
            _ => match try_keyring_login(&host, &paths.last_user) {
                Some((rows, token, user)) => {
                    let total = rows.len();
                    eprintln!("roster: keyring session for {user} from {host}");
                    (rows, total, token, user)
                }
                None => {
                    eprintln!("roster: no credentials and no keyring session, the card");
                    (tfg::shell_gpui::fixture_roster(), 125, String::new(), String::new())
                }
            },
        }
    };

    // The session's READ path: the newest game and its room. Read-only on
    // purpose — `join_game` needs a room key and `set_readiness` WRITES, and
    // neither is the prototype's job. `game_participants` answers the
    // headcount for anyone with read on the game, which is what the top zone
    // shows and what a seat needs before a dashboard can exist. Best-effort
    // like the register: no /games still runs, with an empty bar.
    let session = {
        // `TFG_GAME` pins the held game for a headless capture — the same
        // seed-the-state precedent as `TFG_SEL`.
        let pinned = std::env::var("TFG_GAME")
            .ok()
            .and_then(|v| v.parse::<i64>().ok());
        let mut s = if token.is_empty() {
            tfg::shell_gpui::SessionView::default()
        } else if std::env::var("TFG_SESSIONS").as_deref() == Ok("1") {
            // Seeded open of the session picker: hold nothing, so the
            // column lands on NoSession with the shell's fixture rows to
            // pick. The roster stays real; only the hold is staged.
            tfg::shell_gpui::SessionView::default()
        } else {
            fetch_session(&host, &token, pinned).unwrap_or_default()
        };
        // The camera's opening zoom comes from the MAP's own constant, not the
        // shell's: the shell shows what it asked for, and this is the ask.
        s.zoom = tfg::map_render::MAP_ZOOM;
        // No token means no session was ever established — the shell shows the
        // sign-in card rather than the console, which is the honest state and
        // the one a shipped build starts in.
        s.signed_in = !token.is_empty();
        s
    };

    // The register's class NAMES, for the symbol bridge's fallback.
    let classes: std::collections::HashMap<String, String> = roster
        .iter()
        .map(|r| (r.unit_id.clone(), r.class.clone()))
        .collect();

    // The local store, opened and synced (the console's own path). A hull's
    // symbol resolves from the taxonomy's STABLE IDS when the operator has
    // assigned one, and falls back to the register's class names otherwise;
    // affiliation comes from the same store, Unknown until a session declares
    // any. Best-effort: a machine that cannot reach /units still runs.
    let mut store = tfg::store::open(&paths.local_db).ok();
    if let Some(conn) = store.as_mut().filter(|_| !token.is_empty()) {
        if let Ok(master) = tfg::backend::MinosMaster::new(&host) {
            match tfg::store::sync_from(&master, &token, conn) {
                Ok(counts) => eprintln!("store: synced {} tables", counts.len()),
                Err(e) => {
                    eprintln!("store: sync failed ({e}); symbols fall back to class names")
                }
            }
            // Each held hull's published figures, mirrored per hull. The
            // store's `unit_specs` is where a hull's measured figures live
            // (speed, draft, displacement), and the sim's catalog is what
            // registers classes from it (`catalog::upsert_class`) — a table
            // nothing ever fills is a table the next screen reads as "no
            // published specification" for every hull. Best-effort per hull:
            // a hull with no published version stays unpublished, which is a
            // real state and not a failure.
            //
            // Map thumbnails do NOT come from here: the picture manifest
            // declares each pictured hull's length and beam beside its
            // image, so the map is sized by the same entry that supplies the
            // photograph — see `ThumbRaw`.
            for piece in &session.pieces {
                if let Err(e) = master.sync_spec(&token, conn, piece.id) {
                    eprintln!("store: spec for hull {} not synced ({e})", piece.id);
                }
            }
        }
    }
    // Read once: the taxonomy and the assignment tables do not move under us.
    // The scope is this machine's own (`local:0`): the GPUI binary has no
    // session, so every declaration table is empty and every hull resolves
    // Unknown — the honest answer, not a missing one.
    let store_ctx = store.as_ref().map(|conn| {
        let resolver = tfg::store::SymbolResolver::new(
            tfg::store::unit_type_symbols(conn).unwrap_or_default(),
            tfg::store::unit_category_symbols(conn).unwrap_or_default(),
            tfg::store::unit_movement_domain_symbols(conn).unwrap_or_default(),
        );
        let affiliations = tfg::store::affiliation_resolver(conn, "local:0").unwrap_or_default();
        let tax: std::collections::HashMap<
            String,
            (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
        > = tfg::store::fleet_units(conn)
            .unwrap_or_default()
            .into_iter()
            .map(|u| {
                (
                    u.id.clone(),
                    (u.type_id, u.category_id, u.domain_id, u.branch_id),
                )
            })
            .collect();
        (resolver, affiliations, tax)
    });

    // The poll thread and the live wire (U8 step 4). The registry is the
    // console's own: the poll thread only produces fixes, and staleness is
    // the registry's judgement (three missed polls). We keep `poll_rx` and
    // feed a registry from it, then report liveness to the shell.
    let (poll_tx, poll_rx) = mpsc::channel();
    let (_sim_cmd_tx, sim_cmd_rx) = mpsc::channel::<tfg::sim::SimCommand>();
    let (sim_evt_tx, _sim_evt_rx) = mpsc::channel::<tfg::sim::SimEvent>();
    let (wire_ctl_tx, wire_ctl_rx) = mpsc::channel::<tfg::shell_threads::WireKind>();
    let (wire_wake_tx, wire_wake_rx) = mpsc::channel::<()>();
    let shutdown = Arc::new(AtomicBool::new(false));
    let _poll_handle = tfg::shell_threads::spawn_poll_thread(tfg::shell_threads::PollThreadConfig {
        wire_ctl_rx,
        wire_wake_rx,
        sim_cmd_rx,
        sim_evt_tx,
        poll_tx,
        shutdown: shutdown.clone(),
        armed: Arc::new(AtomicBool::new(false)),
        initial_log: paths.initial_log.clone(),
    });
    // The GAME FEED: the authoritative movement of the held exercise.
    //
    // The socket's command channel belongs to the wire that is up NOW, and
    // the wire is re-made whenever the token changes — so the console's
    // handle on it is a cell, not a value. The watches are re-declared on
    // the new wire, because a fresh actor watches nothing until it is told.
    let live_ctl: tfg::gamefeed::LiveCtl = Arc::new(Mutex::new(None));
    // The socket's events. The order publications arrive here, and an order
    // publication carries a POSITION: it is the first word about where the
    // hull went, and it arrives before any plot read.
    let (live_event_tx, live_event_rx) = mpsc::channel();
    // The held session, shared with the plot reader: which game, which
    // token, which state. The binary writes it; the reader reads it.
    let hold: tfg::gamefeed::GameHoldCell = Arc::new(Mutex::new(tfg::gamefeed::GameHold {
        token: token.clone(),
        game_id: session.game_id,
        state: session.state.clone(),
        kick: 0,
    }));
    // The game fixes, on their own channel: the plot reader, the order
    // publications and the order's own answer all land here, and the
    // registry ingests them on the GAME path — never the wire's.
    let (game_fix_tx, game_fix_rx) = mpsc::channel::<Vec<tfg::geo::track::Fix>>();
    // Swap the boot wire for the live one immediately: the same `WireKind`
    // the egui console sends, over the same socket URL the kernel derives.
    // No token (no credentials) means no live wire — the empty boot wire
    // stands, and staleness stays seeded, honestly.
    if !token.is_empty() {
        start_live_wire(
            &host,
            &token,
            &wire_ctl_tx,
            &live_ctl,
            &live_event_tx,
            &wire_wake_tx,
        );
    }
    // The plot reader: where a Minos-driven hull is, read on a cadence while
    // a game is held. It is the working source of movement — the positions
    // channel is specified but silent (see `gamefeed`).
    let _plot_handle = tfg::gamefeed::spawn_plot_thread(tfg::gamefeed::PlotThreadConfig {
        host: host.clone(),
        hold: hold.clone(),
        fix_tx: game_fix_tx.clone(),
        live: Some(live_ctl.clone()),
        shutdown: shutdown.clone(),
    });
    // The watches follow the session: the broadcast channel for the held
    // game, and the positions channel once the exercise is running.
    sync_session(&hold, &live_ctl, &token, session.game_id, &session.state);

    // The registry the fix feed drains into. Owned here, beside the poll
    // thread it serves: the shell stays a renderer of liveness, and the
    // track's trail/blend logic never leaves the kernel.
    let mut registry = tfg::geo::track::Registry::new(tfg::geo::track::TrailBound::default());

    // The ORDER path: the shell WRITES through a channel and this side owns
    // the API call. Same seam as every feed, in the other direction — the
    // shell never learns a base URL, and a shell that cannot reach the
    // network cannot leak the token either.
    let (order_tx, order_rx) = mpsc::channel::<tfg::shell_gpui::ToBinary>();
    let (status_tx, status_rx) = mpsc::channel::<tfg::shell_gpui::HelmStatus>();
    let (signin_tx, signin_rx) = mpsc::channel::<Result<tfg::shell_gpui::SignedIn, String>>();
    let (create_tx, create_rx) =
        mpsc::channel::<Result<tfg::shell_gpui::CreatedSession, tfg::shell_gpui::WriteRefusal>>();
    let (timeline_tx, timeline_rx) =
        mpsc::channel::<Result<(tfg::backend::TimelinePage, bool), String>>();
    let (judgements_tx, judgements_rx) =
        mpsc::channel::<Result<Vec<tfg::backend::Judgement>, String>>();
    let (reviews_tx, reviews_rx) =
        mpsc::channel::<Result<Vec<tfg::backend::Review>, String>>();
    let (judge_write_tx, judge_write_rx) =
        mpsc::channel::<Result<tfg::backend::Judgement, String>>();
    let (review_write_tx, review_write_rx) =
        mpsc::channel::<Result<(tfg::backend::Review, bool), String>>();
    let (clock_tx, clock_rx) =
        mpsc::channel::<Result<tfg::backend::GameClock, tfg::shell_gpui::ClockError>>();
    let (embarked_tx, embarked_rx) =
        mpsc::channel::<Result<(i64, Vec<tfg::backend::EmbarkedUnit>), String>>();
    let (embark_write_tx, embark_write_rx) = mpsc::channel::<
        Result<(i64, tfg::backend::EmbarkationResult), String>,
    >();
    // The camera, SHARED with the frame and fix feeds: the map thread renders
    // at it and the fix feed projects at it, so a zoom or a pan moves the
    // picture and the markers together instead of decoupling them.
    let camera = std::sync::Arc::new(std::sync::Mutex::new((
        tfg::map_render::MAP_CENTER.0,
        tfg::map_render::MAP_CENTER.1,
        tfg::map_render::MAP_ZOOM,
    )));
    // Hull photographs ride their own channel: the send end is cloned
    // into the order loop (every card sign-in fetches on the new token)
    // and the receive end feeds the shell below.
    let (thumb_tx, thumb_rx) = mpsc::channel::<Vec<tfg::shell_gpui::ThumbRaw>>();
    {
        let (host2, token2, game_id, user2) =
            (host.clone(), token.clone(), session.game_id, user.clone());
        let camera2 = camera.clone();
        let req_tx2 = req_tx.clone();
        // The photo channel's send end rides into the order loop: every
        // card sign-in fetches again on the new token — a launch without
        // credentials must not glyph-hold the rows forever.
        let thumb_fetch_tx = thumb_tx.clone();
        // The game feed rides in too: the order's own answer is a position,
        // and the session the console holds is what the socket watches.
        let hold2 = hold.clone();
        let live_ctl2 = live_ctl.clone();
        let wire_ctl2 = wire_ctl_tx.clone();
        let live_event2 = live_event_tx.clone();
        let wire_wake2 = wire_wake_tx.clone();
        let game_fix2 = game_fix_tx.clone();
        // The held session's state, as the server last read it. The socket's
        // positions channel is watched only in execution, and this is the
        // word that says so. Read before the closure, because the session
        // itself belongs to the shell.
        let mut state2 = session.state.clone();
        std::thread::spawn(move || {
            // Mutated by the sign-in/sign-out answers below: a card sign-in
            // replaces the startup token (empty when the shell opened on the
            // card), and a sign-out clears it so nothing else posts on it.
            let mut token2 = token2;
            let mut game_id = game_id;
            let mut user2 = user2;
            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                return;
            };
            // The camera's requests take sequence numbers well clear of the
            // warm-up burst's, so the two can never be mistaken for each
            // other on the way back.
            let mut seq: u64 = 1000;
            while let Ok(msg) = order_rx.recv() {
                match msg {
                    tfg::shell_gpui::ToBinary::Helm(order) => {
                        let unit_id = order.unit_id.parse::<i64>().unwrap_or(0);
                        // Signed out means no token to post with — and the
                        // shell already gates on execution, so this is the
                        // backstop, not the rule. A refusal the panel can show,
                        // never a silent drop.
                        if token2.is_empty() {
                            let _ = status_tx.send(tfg::shell_gpui::HelmStatus {
                                unit_id: order.unit_id.clone(),
                                ok: false,
                                text: "signed out \u{2014} sign in to order".to_string(),
                            });
                            continue;
                        }
                        let status = match master.order_unit(
                            &token2,
                            game_id,
                            unit_id,
                            f64::from(order.heading_deg),
                            f64::from(order.speed_kn),
                            None,
                        ) {
                            Ok(tfg::backend::OrderOutcome::Applied(fix)) => {
                                // The order's ANSWER is a position: the hull
                                // is here, on a new leg, at the scenario's
                                // instant. It is ingested on the game path, so
                                // the marker moves the moment the order lands
                                // instead of waiting out the plot's cadence.
                                let applied = tfg::backend::GameOrderEvent {
                                    game_id,
                                    fix: fix.clone(),
                                };
                                let _ = game_fix2.send(vec![tfg::gamefeed::order_fix(&applied)]);
                                // And the plot is asked for at once, so the
                                // hull advances along the new leg rather than
                                // sitting at its origin until the next read.
                                tfg::gamefeed::kick(&hold2);
                                tfg::shell_gpui::HelmStatus {
                                    unit_id: order.unit_id.clone(),
                                    ok: true,
                                    text: format!(
                                        "applied {:.0}\u{00b0} / {:.1} kn{}",
                                        fix.heading,
                                        fix.speed,
                                        if fix.clamped { " (clamped)" } else { "" }
                                    ),
                                }
                            }
                            // A HELD ORDER HAS NO FIX, so there is nothing to
                            // ingest and nothing to draw: the hull has not
                            // moved. The panel says so, and says what will
                            // release it.
                            Ok(tfg::backend::OrderOutcome::Staged(held)) => {
                                tfg::shell_gpui::HelmStatus {
                                    unit_id: order.unit_id.clone(),
                                    ok: true,
                                    text: format!(
                                        "staged {:.0}\u{00b0} / {:.1} kn{} \u{2014} {}",
                                        held.heading_deg,
                                        held.speed_kn,
                                        if held.clamped { " (clamped)" } else { "" },
                                        held.released_by
                                    ),
                                }
                            }
                            Err(e) => tfg::shell_gpui::HelmStatus {
                                unit_id: order.unit_id.clone(),
                                ok: false,
                                text: format!("refused: {e}"),
                            },
                        };
                        if status_tx.send(status).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::Zoom(z) => {
                        let mut at = tfg::map_render::MAP_CENTER;
                        if let Ok(mut cam) = camera2.lock() {
                            cam.2 = z;
                            at = (cam.0, cam.1);
                        }
                        seq += 1;
                        let _ = req_tx2.send((
                            seq,
                            at,
                            z,
                            (
                                tfg::map_render::MAP_W as u32,
                                tfg::map_render::MAP_H as u32,
                            ),
                            // A zoom step is an ordinary gesture: spend
                            // GESTURE_PUMP, not the jump budget. The view
                            // updates at motion speed and the tiles catch
                            // up on the following frames.
                            tfg::map_render::GESTURE_PUMP,
                        ));
                    }
                    tfg::shell_gpui::ToBinary::Jump { lat, lon, shift_px } => {
                        // The console's `request_frame`, without the glide:
                        // the shell named the GROUND and the chrome's shift,
                        // and the visible centre is unprojected here because
                        // only this side has the mercator. A centred hull
                        // lands on the visible middle, not under the zone.
                        let (mw, mh) = (tfg::map_render::MAP_W, tfg::map_render::MAP_H);
                        let zoom = camera2.lock().map(|c| c.2).unwrap_or(tfg::map_render::MAP_ZOOM);
                        let goal = jump_goal(lat, lon, shift_px, zoom, mw, mh);
                        if let Ok(mut cam) = camera2.lock() {
                            *cam = (goal.0, goal.1, zoom);
                        }
                        eprintln!("centering on {lat:.4}, {lon:.4} (shift {shift_px}px)");
                        seq += 1;
                        let _ = req_tx2.send((
                            seq,
                            goal,
                            zoom,
                            (mw as u32, mh as u32),
                            tfg::map_render::JUMP_PUMP,
                        ));
                    }
                    tfg::shell_gpui::ToBinary::Pan { dx, dy } => {
                        // The drag pointed at a map pixel; that pixel becomes
                        // the new centre, unprojected. The SIGN lives here and
                        // not in the shell: dragging the picture right asks
                        // for the ground under the cursor to stay put, which
                        // moves the centre LEFT, and the unprojection is the
                        // binary's to do because the shell has no mercator.
                        let (mw, mh) = (tfg::map_render::MAP_W, tfg::map_render::MAP_H);
                        let moved = camera2.lock().ok().map(|cam| {
                            tfg::map_render::pan_center((cam.0, cam.1), cam.2, dx, dy, mw, mh)
                        });
                        let Some((lat, lon)) = moved else {
                            continue;
                        };
                        let zoom = if let Ok(mut cam) = camera2.lock() {
                            *cam = (lat, lon, cam.2);
                            cam.2
                        } else {
                            continue;
                        };
                        seq += 1;
                        let _ = req_tx2.send((
                            seq,
                            (lat, lon),
                            zoom,
                            (mw as u32, mh as u32),
                            // A drag step is an ordinary gesture: spend
                            // GESTURE_PUMP, not the jump budget, for the
                            // same motion-speed reason as the zoom step.
                            tfg::map_render::GESTURE_PUMP,
                        ));
                    }
                    tfg::shell_gpui::ToBinary::DeployUnit { unit_id, fx, fy } => {
                        // The picker's drop (or its click-to-place fallback):
                        // fractions of the map view, unprojected at this
                        // side's camera — the shell has no mercator — then
                        // `set_placement`, then the session re-read, like
                        // every force write. Planning|preparation only: past
                        // play the first fix leg is frozen and Minos refuses,
                        // and the refusal is the picker's sentence.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to deploy".to_string())
                        } else if game_id == 0 {
                            Err("hold a session before deploying".to_string())
                        } else if !fx.is_finite() || !fy.is_finite() {
                            Err("that drop point is nowhere".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            let (mw, mh) = (tfg::map_render::MAP_W, tfg::map_render::MAP_H);
                            let (clat, clon, zoom) = camera2
                                .lock()
                                .map(|c| (c.0, c.1, c.2))
                                .unwrap_or((
                                    tfg::map_render::MAP_CENTER.0,
                                    tfg::map_render::MAP_CENTER.1,
                                    tfg::map_render::MAP_ZOOM,
                                ));
                            let (lat, lon) = tfg::map_render::unproject_mercator(
                                fx.clamp(0.0, 1.0) * mw,
                                fy.clamp(0.0, 1.0) * mh,
                                (clat, clon),
                                zoom,
                                mw,
                                mh,
                            );
                            match master.set_placement(&token2, game_id, unit_id, lat, lon) {
                                Ok(_) => {
                                    eprintln!("placed {unit_id} at {lat:.4}, {lon:.4}");
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => {
                                            Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Deploy,
                                            ))
                                        }
                                        None => Err(
                                            "the hull was placed, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("not placed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Deploy, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::DeployAssign {
                        unit_id,
                        commander_id,
                        fx,
                        fy,
                    } => {
                        // Placing IS assigning: the drop of a hull the
                        // exercise does not hold yet. `assign_unit` first —
                        // a hull with no seat is refused before anything is
                        // written — then the same unproject-and-place the
                        // plain deploy does. Either step names its own
                        // failure, and a hull assigned-but-unplaced is a
                        // good state: the next drop places it outright.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to deploy".to_string())
                        } else if game_id == 0 {
                            Err("hold a session before deploying".to_string())
                        } else if !fx.is_finite() || !fy.is_finite() {
                            Err("that drop point is nowhere".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.assign_unit(&token2, game_id, unit_id, commander_id) {
                                Err(e) => Err(format!("not assigned: {e}")),
                                Ok(_) => {
                                    let (mw, mh) = (tfg::map_render::MAP_W, tfg::map_render::MAP_H);
                                    let (clat, clon, zoom) = camera2
                                        .lock()
                                        .map(|c| (c.0, c.1, c.2))
                                        .unwrap_or((
                                            tfg::map_render::MAP_CENTER.0,
                                            tfg::map_render::MAP_CENTER.1,
                                            tfg::map_render::MAP_ZOOM,
                                        ));
                                    let (lat, lon) = tfg::map_render::unproject_mercator(
                                        fx.clamp(0.0, 1.0) * mw,
                                        fy.clamp(0.0, 1.0) * mh,
                                        (clat, clon),
                                        zoom,
                                        mw,
                                        mh,
                                    );
                                    match master.set_placement(
                                        &token2, game_id, unit_id, lat, lon,
                                    ) {
                                        Ok(_) => {
                                            eprintln!(
                                                "assigned and placed {unit_id} at {lat:.4}, {lon:.4}"
                                            );
                                            match fetch_session(&host2, &token2, Some(game_id)) {
                                                Some(s) => Ok(
                                                    tfg::shell_gpui::CreatedSession::for_write(
                                                        s,
                                                        tfg::shell_gpui::WriteId::Deploy,
                                                    ),
                                                ),
                                                None => Err(
                                                    "the hull was placed, but the session could not be read"
                                                        .to_string(),
                                                ),
                                            }
                                        }
                                        Err(e) => Err(format!(
                                            "assigned, but not placed: {e}"
                                        )),
                                    }
                                }
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Deploy, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SignIn { identifier, password } => {
                        // Sign in, then STOP: the roster and the games list
                        // arrive together, but no game is held — holding is
                        // the picker's verb, and an auto-hold here is what
                        // dropped fresh logins mid-execution with no say.
                        // The refusal is a sentence the card can show —
                        // never a swallowed error.
                        let outcome = match fetch_roster(&host2, &identifier, &password) {
                            Some((rows, token, refresh_token)) => {
                                let total = rows.len();
                                let sessions =
                                    fetch_session_rows(&host2, &token).unwrap_or_default();
                                // The shell gets the roster and the games;
                                // THIS side keeps the token with no game
                                // yet, so the first hold posts as the
                                // picked session. Photos fetch on the new
                                // token too — a launch without credentials
                                // must not glyph-hold the rows forever.
                                token2 = token.clone();
                                // The refresh token outlives the access
                                // one: the keyring stores it so the next
                                // launch skips the card, and the last
                                // user is remembered to key it by.
                                user2 = identifier.clone();
                                if let Some(rt) = refresh_token.as_deref() {
                                    let _ = tfg::backend::keyring_save(&identifier, rt);
                                    tfg::backend::last_user_save(
                                        &paths.last_user,
                                        &identifier,
                                    );
                                }
                                spawn_thumb_fetch(
                                    host2.clone(),
                                    token.clone(),
                                    thumb_fetch_tx.clone(),
                                );
                                // The socket comes up on the new token: a
                                // console that signed in from the card had
                                // no wire at all, and the wire is where the
                                // exercise's order publications arrive.
                                start_live_wire(
                                    &host2,
                                    &token,
                                    &wire_ctl2,
                                    &live_ctl2,
                                    &live_event2,
                                    &wire_wake2,
                                );
                                game_id = 0;
                                state2 = String::new();
                                sync_session(&hold2, &live_ctl2, &token2, game_id, &state2);
                                Ok(tfg::shell_gpui::SignedIn {
                                    roster: rows,
                                    roster_total: total,
                                    sessions,
                                    session: tfg::shell_gpui::SessionView::default(),
                                })
                            }
                            None => Err(
                                "sign-in refused \u{2014} check the identifier and password"
                                    .to_string(),
                            ),
                        };
                        if signin_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SignOut => {
                        // Local, like the shell's half: drop the token so
                        // nothing else posts on it. No server revocation —
                        // the refresh token lives in the OS keyring, not
                        // here — and the live wire keeps its socket,
                        // honestly idle. The keyring entry goes with it:
                        // a signed-out operator stays signed out.
                        let _ = tfg::backend::keyring_clear(&user2);
                        tfg::backend::last_user_clear(&paths.last_user);
                        token2.clear();
                        user2.clear();
                        // The socket goes with the token: an idle wire that
                        // still holds a session is a wire the server will
                        // soon refuse, and the console says nothing about
                        // it. The empty wire is the honest one.
                        let _ = wire_ctl2.send(tfg::shell_threads::WireKind::Empty);
                        if let Ok(mut slot) = live_ctl2.lock() {
                            *slot = None;
                        }
                        game_id = 0;
                        state2 = String::new();
                        sync_session(&hold2, &live_ctl2, "", game_id, &state2);
                        eprintln!("signed out — token and keyring cleared");
                    }
                    tfg::shell_gpui::ToBinary::HoldSession { game_id: hold } => {
                        // The picker's verb: hold the named game, read back
                        // rather than assumed — `fetch_session` pinned, the
                        // same read startup holds. An unknown id refuses
                        // with the row's own name for it, never silence.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to hold a session".to_string())
                        } else {
                            match fetch_session(&host2, &token2, Some(hold)) {
                                Some(mut s) => {
                                    s.signed_in = true;
                                    s.zoom = tfg::map_render::MAP_ZOOM;
                                    game_id = s.game_id;
                                    state2 = s.state.clone();
                                    // The watches follow the held game, and
                                    // the plot reader starts reading it.
                                    sync_session(&hold2, &live_ctl2, &token2, game_id, &state2);
                                    eprintln!("holding {} ({})", s.game, s.state);
                                    Ok(tfg::shell_gpui::CreatedSession::for_write(
                                        s,
                                        tfg::shell_gpui::WriteId::Hold,
                                    ))
                                }
                                None => Err("could not hold that session".to_string()),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Hold, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::CreateSession { draft } => {
                        // The console's `setup_create_game`, as a message: a
                        // signed-out shell has no token to create with, and an
                        // empty name never reaches here (the panel refuses it
                        // locally) but is refused again anyway — the seam must
                        // not trust the renderer. On success the new game is
                        // HELD, the way `apply_created_game` holds it: born in
                        // planning by contract, read back rather than assumed.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to create a session".to_string())
                        } else if draft.name.trim().is_empty() {
                            Err("name the session first".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.create_game(
                                &token2,
                                draft.name.trim(),
                                &draft.mode,
                                &draft.description,
                                &draft.purpose,
                                &draft.target,
                                &draft.area,
                                &draft.map_tag,
                            ) {
                                Ok(g) => {
                                    eprintln!("created {} (planning)", g.name);
                                    match fetch_session(&host2, &token2, Some(g.id)) {
                                        Some(mut s) => {
                                            s.zoom = camera2
                                                .lock()
                                                .map(|c| c.2)
                                                .unwrap_or(tfg::map_render::MAP_ZOOM);
                                            game_id = s.game_id;
                                            state2 = s.state.clone();
                                            sync_session(
                                                &hold2,
                                                &live_ctl2,
                                                &token2,
                                                game_id,
                                                &state2,
                                            );
                                            Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Session,
                                            ))
                                        }
                                        None => Err(
                                            "created, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("create failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Session, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::AddScenario { title } => {
                        // The composer's left column, minus the staging: the
                        // console stages a scenario and syncs it on advance,
                        // and this shell has no advance, so the title posts at
                        // once. Same guards as creation — signed-out and
                        // titleless never reach the wire — and the same
                        // read-back: the book arrives re-read, not assumed.
                        let title = title.trim().to_string();
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to author the book".to_string())
                        } else if title.is_empty() {
                            Err("a scenario needs a title".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.create_game_scenario(&token2, game_id, &title, "") {
                                Ok(sc) => {
                                    eprintln!("added scenario {} {:?}", sc.id, sc.title);
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Book,
                                            )),
                                        None => Err(
                                            "added, but the book could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("add failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Book, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::AddStep { scenario_id, content, window } => {
                        // The composer's step row, minus the staging: the
                        // console's `stage_step_add` waits for the advance and
                        // this shell posts at once. Empty content is LEGAL —
                        // an unwritten step is an authoring state — so only
                        // the window rule can refuse, re-checked here because
                        // the seam must not trust the renderer.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to author the book".to_string())
                        } else {
                            let window_ok = match window.as_ref() {
                                None => true,
                                Some((a, b)) => {
                                    tfg::backend::hhmm_ok(a)
                                        && tfg::backend::hhmm_ok(b)
                                        && tfg::backend::hhmm_window_ok(a, b)
                                }
                            };
                            if !window_ok {
                                Err("a window is HHMM like 1000, both ends, end after start"
                                    .to_string())
                            } else {
                                let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                    return;
                                };
                                let w = window.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
                                match master.add_scenario_step(
                                    &token2, game_id, scenario_id, &content, w,
                                ) {
                                    Ok(sc) => {
                                        eprintln!("added a step to scenario {}", sc.id);
                                        match fetch_session(&host2, &token2, Some(game_id)) {
                                            Some(s) => {
                                                Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                    s,
                                                    tfg::shell_gpui::WriteId::Step,
                                                ))
                                            }
                                            None => Err(
                                                "added, but the book could not be read".to_string(),
                                            ),
                                        }
                                    }
                                    Err(e) => Err(format!("add failed: {e}")),
                                }
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Step, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::RemoveStep { scenario_id, step_id } => {
                        // Deletes AT ONCE — harsher than the console's staged
                        // cut, and the panel says so.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to author the book".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.delete_scenario_step(&token2, game_id, scenario_id, step_id)
                            {
                                Ok(sc) => {
                                    eprintln!("removed step {step_id} from scenario {}", sc.id);
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => {
                                            Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Step,
                                            ))
                                        }
                                        None => Err(
                                            "removed, but the book could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("remove failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Step, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SetReadiness { ready } => {
                        // The console's `set_own_readiness`, as a message: a
                        // player declares in preparation — proven on game 12,
                        // unlike the helm, which waits for execution. The
                        // answer echoes the held game, so the hold follows it
                        // by re-read rather than assumption.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to declare readiness".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.set_readiness(&token2, game_id, ready) {
                                Ok(j) => {
                                    eprintln!(
                                        "{}",
                                        if ready {
                                            format!(
                                                "{} declared ready ({} · {})",
                                                j.user_name, j.role_name, j.game_name
                                            )
                                        } else {
                                            format!("{} withdrew readiness", j.user_name)
                                        }
                                    );
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Readiness,
                                            )),
                                        None => Err(
                                            "declared, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("readiness failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Readiness, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SyncRoster { writes } => {
                        // The roster, whole, in `roster::plan`'s order. Each
                        // write is one endpoint; the first refusal stops the
                        // plan, because a half-seated room is worse than one
                        // that reports.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to seat the room".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match flush_roster(&master, &token2, game_id, &writes) {
                                Err(why) => Err(why),
                                Ok(()) => {
                                    eprintln!("roster synced: {} write(s)", writes.len());
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Roster,
                                            )),
                                        None => Err(
                                            "the room was written, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Roster, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SyncForce { writes } => {
                        // The force, whole, in `force::plan`'s order. Each write
                        // is one endpoint and the order is the order the server
                        // accepts them in. The first refusal stops the plan:
                        // posting the rest would half-commit a force the
                        // operator cannot see, which is the failure
                        // `ForceGap` exists to prevent.
                        //
                        // This path is reached only OUTSIDE planning — while
                        // planning each edit stages and posts nothing — so the
                        // writes here are one decision's worth and a `Place`
                        // or `Lift` is an ordinary position write rather than a
                        // bug.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to pick the fleet".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match flush_force(&master, &token2, game_id, &writes) {
                                Err(why) => Err(why),
                                Ok(()) => {
                                    eprintln!("force synced: {} write(s)", writes.len());
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Force,
                                            )),
                                        None => Err(
                                            "the force was written, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Force, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::AdvancePreparation {
                        force,
                        live_hulls,
                        roster,
                        live_seats,
                        groups,
                            window,
                            book,
                            live_scenarios,
                    } => {
                        // LEAVE PLANNING. The drafts cross whole because the
                        // token lives HERE and the drafts live THERE: the
                        // shell cannot compute the writes and this side cannot
                        // see the drafts.
                        //
                        // `tfg::advance::plan` lays the steps out in the order
                        // the server takes them and refuses the WHOLE move on
                        // any rule it breaks — the force's, the room's and
                        // the roster's. Nothing is posted, because issuing
                        // the rest would half-commit the very draft the
                        // advance exists to record.
                        //
                        // THE ORDER IS THE PLAN'S, not this arm's: the
                        // window (the exercise's own terms, and the one write
                        // that needs the administrator's ticket) first, then
                        // the seats a commander write names, then the force,
                        // then the role changes — a role change onto the
                        // judge side RELEASES the hulls that seat commands,
                        // so it lands after the force has given those
                        // commands away — then the unseats, the task
                        // organisation, and the transition last.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to move the plan to preparation"
                                .to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                                advance_to_preparation(
                                    &master,
                                    &host2,
                                    &token2,
                                    game_id,
                                    force,
                                    live_hulls,
                                    roster,
                                    live_seats,
                                    groups,
                                    window,
                                    book,
                                    live_scenarios,
                                )
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Prepare, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SyncStaged {
                        force,
                        live_hulls,
                        roster,
                        live_seats,
                        groups,
                        book,
                        live_scenarios,
                    } => {
                        // THE SYNC. The staged writes reach the server and
                        // the exercise's PHASE does not move — the one
                        // exception to local-first planning, and the one the
                        // operator asked for.
                        //
                        // It is the advance's flush without the transition,
                        // in the same order, so the two cannot disagree
                        // about what the server accepts and in what order:
                        // seats, then the force, then the unseats, then the
                        // task organisation, then the book. It is the same
                        // PLAN, too — the same rules, checked before the
                        // first write, because a sync that half-commits a
                        // force is the same fault as an advance that does.
                        //
                        // THE WINDOW IS NOT PART OF IT. The terms are the
                        // administrator's ticket, and a sync that asked for
                        // them would fail for a Game Master who holds
                        // everything else; the terms ride the advance, which
                        // is where the exercise moves on.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to sync the staged writes"
                                .to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            // THE SERVER'S FRAME, re-read: the same reason the
                            // advance re-reads it. A diff against a stale frame
                            // re-issues the writes the server already took.
                            let live_seats = read_live_seats(&master, &token2, game_id, live_seats);
                            let live_hulls = read_live_hulls(&master, &token2, game_id, live_hulls);
                            let book_writes = tfg::book::plan(&book, &live_scenarios);
                            let steps = tfg::advance::plan(
                                &force,
                                &live_hulls,
                                &roster,
                                &live_seats,
                                &groups,
                                &book_writes,
                                None,
                            );
                            match steps {
                                Err(refusal) => Err(refusal.to_string()),
                                Ok(steps) => {
                                    match run_plan(
                                        &master,
                                        &token2,
                                        game_id,
                                        &steps,
                                        &live_scenarios,
                                        false,
                                    ) {
                                        Err(why) => Err(why),
                                        Ok(written) => {
                                            eprintln!(
                                                "staged writes synced ({written} write(s), phase unchanged)"
                                            );
                                            match fetch_session(
                                                &host2,
                                                &token2,
                                                Some(game_id),
                                            ) {
                                                Some(s) => {
                                                    Ok(tfg::shell_gpui::CreatedSession {
                                                        session: s,
                                                        note: format!(
                                                            "{written} write(s) sent \u{2014} the phase did not move"
                                                        ),
                                                        refusal: None,
                                                        write: tfg::shell_gpui::WriteId::Sync,
                                                    })
                                                }
                                                None => Err(
                                                    "the writes were sent, but the session could not be read"
                                                        .to_string(),
                                                ),
                                            }
                                        }
                                    }
                                }
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Sync, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SetTimeFactor { factor } => {
                        // Control's Apply: one field, one endpoint. The answer
                        // is a re-read like every other write here, so the
                        // readout on the next frame is Minos's and never the
                        // draft's — which is the whole reason a multiplier is
                        // worth writing through rather than faking locally.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to set the pace".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.set_time_factor(&token2, game_id, factor) {
                                Ok(_) => {
                                    eprintln!("time factor set to {factor:.0}\u{00d7}");
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Factor,
                                            )),
                                        None => Err(
                                            "pace set, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                // No grant, no pace — and the sentence is the
                                // shared grant one, so every pane that asks
                                // names the same gate.
                                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                                    Err(tfg::shell_gpui::CLOCK_GRANT.to_string())
                                }
                                Err(e) => Err(format!("pace refused: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Factor, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::AdvanceExecution => {
                        // Leave preparation. The server gates this itself —
                        // the readiness checklist is advisory and this is the
                        // verdict — so a refusal is a sentence, never silence.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to advance".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.transition_game(&token2, game_id, "execution") {
                                Ok(g) => {
                                    eprintln!("{} advanced to {}", g.name, g.state);
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => {
                                            // The exercise is running: the
                                            // positions channel is declared
                                            // and the plot reader starts
                                            // reading, both at once.
                                            state2 = s.state.clone();
                                            sync_session(
                                                &hold2,
                                                &live_ctl2,
                                                &token2,
                                                game_id,
                                                &state2,
                                            );
                                            Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Advance,
                                            ))
                                        }
                                        None => Err(
                                            "advanced, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("advance refused: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Advance, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::EndSession => {
                        // Leave execution. The console's `close_game`: the
                        // server gates the transition itself and the verdict
                        // comes back as a sentence. On success the answer
                        // echoes the held game, now closed — the column
                        // recomposes to the debrief around it.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to end the session".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.transition_game(&token2, game_id, "closure") {
                                Ok(g) => {
                                    eprintln!("{} ended ({})", g.name, g.state);
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => {
                                            // The exercise is closed: the
                                            // positions channel is let go,
                                            // because nothing publishes on
                                            // it any more.
                                            state2 = s.state.clone();
                                            sync_session(
                                                &hold2,
                                                &live_ctl2,
                                                &token2,
                                                game_id,
                                                &state2,
                                            );
                                            Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::End,
                                            ))
                                        }
                                        None => Err(
                                            "ended, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("end refused: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::End, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SetPaused { paused } => {
                        // The console's `pause_or_resume_minos`, as a
                        // message: the write IS the read — no clock GET
                        // exists — and the answer is the GameClock to
                        // display. A 403 is the control grant, answered
                        // distinctly so the shell latches it instead of
                        // re-asking; anything else fails loudly.
                        let outcome = if token2.is_empty() {
                            Err(tfg::shell_gpui::ClockError::Failed(
                                "signed out \u{2014} sign in to hold the clock".to_string(),
                            ))
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            let res = if paused {
                                master.pause_game(&token2, game_id)
                            } else {
                                master.resume_game(&token2, game_id)
                            };
                            match res {
                                Ok(clock) => {
                                    eprintln!(
                                        "scenario clock {} ({}x)",
                                        if clock.running { "running" } else { "held" },
                                        clock.time_factor
                                    );
                                    Ok(clock)
                                }
                                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                                    Err(tfg::shell_gpui::ClockError::Denied)
                                }
                                Err(e) => Err(tfg::shell_gpui::ClockError::Failed(format!(
                                    "clock {} refused: {e}",
                                    if paused { "pause" } else { "resume" }
                                ))),
                            }
                        };
                        if clock_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::ReadEmbarked { host } => {
                        // The console's `ensure_embarked`, as a message: one
                        // hull's passengers in this exercise. The host rides
                        // the answer back with its contents, because answers
                        // can overtake opens and one hull's passengers under
                        // another's is the misfile to avoid.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to see who is aboard".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.game_embarkations(&token2, game_id, host) {
                                Ok(units) => {
                                    eprintln!("embarked aboard {host}: {} unit(s)", units.len());
                                    Ok((host, units))
                                }
                                Err(e) => Err(format!("embarked failed: {e}")),
                            }
                        };
                        if embarked_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::UnembarkUnit { host, unit } => {
                        // The console's `unembark`, as a message: one piece
                        // out of its host, and the host's remaining contents
                        // answered back. She was a piece before she embarked
                        // and still is one, so no session fixup rides along.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to unembark".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.game_detach_units(&token2, game_id, host, &[unit]) {
                                Ok(result) => {
                                    eprintln!("unembarked {unit} from {host}");
                                    Ok((host, result))
                                }
                                Err(e) => Err(format!("unembark failed: {e}")),
                            }
                        };
                        if embark_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::EmbarkDrop { host, child } => {
                        // The picker's drop onto a placed host: one hull
                        // aboard another, through `game_attach_units`. The
                        // console's `try_embark_drop`, as a message, without
                        // its capacity pre-checks — the server evaluates and
                        // the answer carries what it said.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to embark".to_string())
                        } else if game_id == 0 {
                            Err("hold a session before embarking".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.game_attach_units(&token2, game_id, host, &[child]) {
                                Ok(result) => {
                                    eprintln!("embarked {child} aboard {host}");
                                    Ok((host, result))
                                }
                                Err(e) => Err(format!("not embarked: {e}")),
                            }
                        };
                        if embark_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::EmbarkAssign { host, child, commander_id } => {
                        // Embarking IS assigning, the host-side twin of the
                        // drop above: `assign_unit` first, then the attach.
                        // Answered on the embark feed like every embark
                        // write — the host's contents after the act.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to embark".to_string())
                        } else if game_id == 0 {
                            Err("hold a session before embarking".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.assign_unit(&token2, game_id, child, commander_id) {
                                Err(e) => Err(format!("not assigned: {e}")),
                                Ok(_) => {
                                    match master.game_attach_units(&token2, game_id, host, &[child]) {
                                        Ok(result) => {
                                            eprintln!("assigned and embarked {child} aboard {host}");
                                            Ok((host, result))
                                        }
                                        Err(e) => Err(format!(
                                            "assigned, but not embarked: {e}"
                                        )),
                                    }
                                }
                            }
                        };
                        if embark_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::ClearAboard { host, unit } => {
                        // The console's `embark_clear`, as a message:
                        // permission to act, not release. Answered back the
                        // same way: the host's contents now.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to clear aboard".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.game_clear_aboard(&token2, game_id, host, &[unit]) {
                                Ok(result) => {
                                    eprintln!("cleared {unit} aboard {host}");
                                    Ok((host, result))
                                }
                                Err(e) => Err(format!("clear failed: {e}")),
                            }
                        };
                        if embark_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::RevokeClearance { host, unit } => {
                        // The console's `embark_revoke`, as a message. Legs
                        // already sailed stay sailed; anyone-but-author fails
                        // with the server's explicit 403, carried on the
                        // line.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to revoke clearance".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.game_revoke_clearance(&token2, game_id, host, &[unit]) {
                                Ok(result) => {
                                    eprintln!("revoked {unit} aboard {host}");
                                    Ok((host, result))
                                }
                                Err(e) => Err(format!("revoke failed: {e}")),
                            }
                        };
                        if embark_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SetRelated { scenario_id, step_id, related } => {
                        // The relate editor's save, minus the staging: the
                        // console stages relations and syncs them on advance,
                        // and this shell posts the working set whole — empty
                        // included, which is legal and clears the list. The
                        // server refuses hulls outside the exercise with a
                        // 400, so the editor only offers pieces; a refusal
                        // still comes back as a sentence, never silence.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to author the book".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.set_step_related(
                                &token2,
                                game_id,
                                scenario_id,
                                step_id,
                                &related,
                            ) {
                                Ok(r) => {
                                    eprintln!(
                                        "related step {step_id}: {} post(s)",
                                        r.posts.len()
                                    );
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Relate,
                                            )),
                                        None => Err(
                                            "saved, but the book could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("save failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Relate, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::DeclareStep { scenario_id, step_id, ready } => {
                        // The console's `declare_step`, as a message: the
                        // participant's own write on the asking step — proven
                        // on game 44 — unlike the Game Master's verbs, which
                        // aim from tracked runs this shell has none of. The
                        // answer echoes the held game, so the counts follow
                        // it by re-read rather than assumption.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to declare on a step".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            let op = if ready {
                                master.declare_step_readiness(&token2, game_id, scenario_id, step_id)
                            } else {
                                master.withdraw_step_readiness(&token2, game_id, scenario_id, step_id)
                            };
                            match op {
                                Ok(()) => {
                                    eprintln!(
                                        "{}",
                                        if ready {
                                            "ready for the step — the Game Master plays it once everyone is"
                                        } else {
                                            "declaration withdrawn — declare again when set"
                                        }
                                    );
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::StepDeclare,
                                            )),
                                        None => Err(
                                            "declared, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("step declaration failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::StepDeclare, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::StepVerb {
                        scenario_id,
                        step_id,
                        verb,
                    } => {
                        // The Game Master's four verbs, on ONE message: the
                        // console's `play_step`, `skip_step`, `end_step` and
                        // `rewind_step`, which Minos has published all along.
                        //
                        // The shell does not track runs and does not need to:
                        // every one of these names a step in the path, and the
                        // step the shell names is the row the operator pressed.
                        // The server arbitrates the seat on top of that, and
                        // its refusal comes back as the sentence the board
                        // shows.
                        //
                        // The answer echoes the new pointer by RE-READ rather
                        // than by assumption (`current-step` is the only route
                        // that publishes it). A rewind answers with what the
                        // erasure cost instead of a run, so that sentence is
                        // carried back with the session — the one write here
                        // with something to say.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to drive a step".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            let op: Result<Option<String>, String> = match verb {
                                tfg::shell_gpui::StepVerb::Play => master
                                    .play_step(&token2, game_id, scenario_id, step_id)
                                    .map(|_| None)
                                    .map_err(|e| e.to_string()),
                                tfg::shell_gpui::StepVerb::Skip => master
                                    .skip_step(&token2, game_id, scenario_id, step_id)
                                    .map(|_| None)
                                    .map_err(|e| e.to_string()),
                                tfg::shell_gpui::StepVerb::End => master
                                    .end_step(&token2, game_id, scenario_id, step_id)
                                    .map(|_| None)
                                    .map_err(|e| e.to_string()),
                                tfg::shell_gpui::StepVerb::Replay => master
                                    .rewind_step(&token2, game_id, scenario_id, step_id)
                                    .map(|r| {
                                        Some(format!(
                                            "step {step_id} replayed \u{2014} erased {} order(s), {} report(s), {} telegram(s), {} judgement(s), {} step(s)",
                                            r.erased_orders,
                                            r.erased_reports,
                                            r.erased_telegrams,
                                            r.erased_judgements,
                                            r.erased_steps
                                        ))
                                    })
                                    .map_err(|e| e.to_string()),
                            };
                            match op {
                                Ok(note) => {
                                    if let Some(n) = &note {
                                        eprintln!("{n}");
                                    }
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession {
                                            session: s,
                                            note: note.unwrap_or_default(),
                                            refusal: None,
                                            write: tfg::shell_gpui::WriteId::StepVerb,
                                        }),
                                        None => Err(
                                            "the step moved, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("step {} refused: {e}", verb.label().to_lowercase())),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::StepVerb, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::SendTelegram { content } => {
                        // The comms composer's send, cut to the proven path:
                        // a broadcast telegram, TERBUKA, BIASA (degree 1) —
                        // the exact body the write probe posted for its 201.
                        // Empty `to`/`cc` IS the broadcast by contract. The
                        // answer echoes the held game, so the thread follows
                        // it by re-read rather than assumption.
                        let content = content.trim().to_string();
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to send messages".to_string())
                        } else if content.is_empty() {
                            Err("write the message first".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            let draft = tfg::backend::MsgDraft {
                                kind: "telegram".to_string(),
                                classification: "TERBUKA".to_string(),
                                content,
                                to: Vec::new(),
                                cc: Vec::new(),
                                assumed_role: None,
                                reply_to: None,
                                degree: 1,
                                msg_type: None,
                                callsign: String::new(),
                                sending_note: String::new(),
                                group_name: String::new(),
                                per: String::new(),
                                registration_number: String::new(),
                            };
                            match master.send_message(&token2, game_id, &draft) {
                                Ok(m) => {
                                    eprintln!("sent telegram #{} (broadcast)", m.id);
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession::for_write(
                                                s,
                                                tfg::shell_gpui::WriteId::Message,
                                            )),
                                        None => Err(
                                            "sent, but the thread could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("send failed: {e}")),
                            }
                        };
                        let outcome = outcome.map_err(|why| {
                            tfg::shell_gpui::WriteRefusal::new(tfg::shell_gpui::WriteId::Message, why)
                        });
                        if create_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::Timeline {
                        append,
                        source,
                        personnel,
                        unit,
                        from,
                        to,
                        cursor,
                    } => {
                        // The console's `load_timeline`, as a message: the
                        // Closure timeline page under the tab's filters, first
                        // page or the cursor's next. A signed-out shell has no
                        // token to read with; the ids arrived parsed (the tab
                        // refuses a bad one before asking) and the window
                        // arrives trimmed-or-absent, because the server owns
                        // timestamp parsing and answers a bad one loudly.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to read the timeline".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.timeline_page(
                                &token2,
                                game_id,
                                source.as_deref(),
                                personnel,
                                unit,
                                from.as_deref(),
                                to.as_deref(),
                                cursor.as_deref(),
                                50,
                            ) {
                                Ok(page) => {
                                    eprintln!(
                                        "timeline: {} event(s){}",
                                        page.events.len(),
                                        if page.has_more { " · more below" } else { "" }
                                    );
                                    Ok((page, append))
                                }
                                Err(e) => Err(format!("timeline failed: {e}")),
                            }
                        };
                        if timeline_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::ReloadJudgements => {
                        // The console's `load_judgements`, as a message: the
                        // whole marks list, re-read. No personnel narrow —
                        // the console always passes `None`, and the tab draws
                        // no filter to carry one.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to read judgements".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.judgements_list(&token2, game_id, None) {
                                Ok(marks) => {
                                    eprintln!("judgements: {} mark(s)", marks.len());
                                    Ok(marks)
                                }
                                Err(e) => Err(format!("judgements failed: {e}")),
                            }
                        };
                        if judgements_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::ReloadReviews { author } => {
                        // The console's `load_reviews`, as a message: the
                        // whole documents list, or the authorship's own while
                        // `mine only` is checked. The narrow arrives resolved
                        // (this seat or `None`), because this side holds the
                        // token but never the seating.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to read reviews".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.reviews_list(&token2, game_id, None, author) {
                                Ok(documents) => {
                                    eprintln!("reviews: {} document(s)", documents.len());
                                    Ok(documents)
                                }
                                Err(e) => Err(format!("reviews failed: {e}")),
                            }
                        };
                        if reviews_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::RecordJudgement { personnel_id, score } => {
                        // The console's `record_judgement`, as a message: one
                        // mark POSTed, and the filed mark answered back — the
                        // shell names it on its status line and re-reads
                        // through the reload, the way `JudgementSent`
                        // re-calls `load_judgements`.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to record judgements".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.record_judgement(&token2, game_id, personnel_id, &score) {
                                Ok(view) => {
                                    eprintln!("recorded judgement #{} on {personnel_id}", view.id);
                                    Ok(view)
                                }
                                Err(e) => Err(format!("record failed: {e}")),
                            }
                        };
                        if judge_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::FileReview { personnel_id, body } => {
                        // The console's `file_review`, as a message: one
                        // document POSTed, answered back with `false` — filed,
                        // not revised — so the shell clears the file drafts
                        // rather than the edit.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to file reviews".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.file_review(&token2, game_id, personnel_id, &body) {
                                Ok(view) => {
                                    eprintln!("filed review #{} on {personnel_id}", view.id);
                                    Ok((view, false))
                                }
                                Err(e) => Err(format!("file failed: {e}")),
                            }
                        };
                        if review_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                    tfg::shell_gpui::ToBinary::ReviseReview { review_id, body } => {
                        // The console's `revise_review`, as a message: words
                        // only, subject fixed — answered back with `true` so
                        // the shell clears the edit rather than the file
                        // drafts. Anyone-but-author fails with the server's
                        // explicit 403, carried on the line.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to revise reviews".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            match master.revise_review(&token2, game_id, review_id, &body) {
                                Ok(view) => {
                                    eprintln!("revised review #{review_id}");
                                    Ok((view, true))
                                }
                                Err(e) => Err(format!("revision failed: {e}")),
                            }
                        };
                        if review_write_tx.send(outcome).is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
    let status_feed: tfg::shell_gpui::OrderStatusFeed =
        std::boxed::Box::new(move || status_rx.try_recv().ok());
    let signin_feed: tfg::shell_gpui::SignInFeed =
        std::boxed::Box::new(move || signin_rx.try_recv().ok());
    let create_feed: tfg::shell_gpui::CreateFeed =
        std::boxed::Box::new(move || create_rx.try_recv().ok());
    let timeline_feed: tfg::shell_gpui::TimelineFeed =
        std::boxed::Box::new(move || timeline_rx.try_recv().ok());
    let judgements_feed: tfg::shell_gpui::JudgementsFeed =
        std::boxed::Box::new(move || judgements_rx.try_recv().ok());
    let reviews_feed: tfg::shell_gpui::ReviewsFeed =
        std::boxed::Box::new(move || reviews_rx.try_recv().ok());
    let judge_write_feed: tfg::shell_gpui::JudgeWriteFeed =
        std::boxed::Box::new(move || judge_write_rx.try_recv().ok());
    let review_write_feed: tfg::shell_gpui::ReviewWriteFeed =
        std::boxed::Box::new(move || review_write_rx.try_recv().ok());
    let clock_feed: tfg::shell_gpui::ClockFeed =
        std::boxed::Box::new(move || clock_rx.try_recv().ok());
    let embarked_feed: tfg::shell_gpui::EmbarkedFeed =
        std::boxed::Box::new(move || embarked_rx.try_recv().ok());
    let embark_write_feed: tfg::shell_gpui::EmbarkWriteFeed =
        std::boxed::Box::new(move || embark_write_rx.try_recv().ok());
    // One clone per feed: both closures are `move`, and each needs the camera
    // it renders or projects at.
    let camera_frame = camera.clone();
    let camera_fix = camera.clone();
    // The socket's own log cadence: one line every ten publications, so
    // the wire says what it is carrying without drowning the plot's line.
    let mut wire_log_ticks: u32 = 0;

    // The picker's drill tree, read once off the synced mirror like the
    // symbol resolver above it: the taxonomy does not move under us, and a
    // picker that re-read sqlite per frame would be a fan-out for nothing.
    // Empty when the store never synced — the picker falls back to flat.
    let fleet_tax = store.as_ref().map(|conn| {
        build_fleet_taxonomy(conn, |id| resolve_symbol(id, &store_ctx, &classes))
    }).unwrap_or_default();


    // The photo channel lives as long as the shell: startup fetches when
    // it can, and every sign-in fetches again — a launch without
    // credentials must not glyph-hold the rows forever.
    if !token.is_empty() {
        spawn_thumb_fetch(host.clone(), token.clone(), thumb_tx.clone());
    }
    let thumb_feed: tfg::shell_gpui::ThumbFeed =
        std::boxed::Box::new(move || thumb_rx.try_recv().ok());

    tfg::shell_gpui::run(
        Box::new(move || {
            // Newest-wins, the discipline the map thread answers with: a
            // burst of frames paints once.
            // The camera rides along now. `MapResp` has always carried it as
            // its second and third fields; this drain read them as `_at` and
            // `_zoom` and threw them away, which is why the console had no
            // scale bar — the number was arriving every frame and nobody kept
            // it. Newest-wins on the whole tuple, so the centre and the frame
            // can never disagree about which render they describe.
            let mut newest: Option<(u64, u32, u32, Vec<u8>, (f64, f64), f64)> = None;
            while let Ok((no, at, zoom, (w, h), mut rgba)) = resp_rx.try_recv() {
                tfg::map_gate::swizzle_rgba_to_bgra(&mut rgba);
                newest = Some((no, w, h, rgba, at, zoom));
            }
            // A WARM-UP BURST, not camera behaviour: this shell has no camera
            // yet, so it asks on a timer until the tiles have had time to
            // arrive. The console's per-gesture requests are the next step —
            // same channel, same spawn.
            warmups += 1;
            if warmups <= WARMUP_FRAMES {
                let (clat, clon, zoom) = camera_frame.lock().map(|c| *c).unwrap_or((
                    tfg::map_render::MAP_CENTER.0,
                    tfg::map_render::MAP_CENTER.1,
                    tfg::map_render::MAP_ZOOM,
                ));
                let _ = req_tx.send((
                    warmups,
                    (clat, clon),
                    zoom,
                    (tfg::map_render::MAP_W as u32, tfg::map_render::MAP_H as u32),
                    4,
                ));
            }
            newest
        }),
        Box::new(move || {
            // Drain this tick's fix rounds into the registry, then report
            // every tracked hull as a live marker: liveness keyed by unit id,
            // and its fix projected into the map frame's pixel space (the
            // shell scales that to the window). `None` when nothing arrived —
            // the registry is quiet, not wrong, and the shell keeps its rows
            // and its markers as they were.
            let mut batch = Vec::new();
            // The GAME fixes first: the exercise's own positions, on the
            // game path. They are ingested apart from the wire's, because a
            // game fix is scenario-stamped (never aged against the wall
            // clock) and because the wire's silence must not mark a running
            // exercise's hulls stale.
            while let Ok(fixes) = game_fix_rx.try_recv() {
                registry.poll_game(fixes);
            }
            // The socket's events. The order publication is the one that
            // moves a hull: it carries the position the order put it at,
            // which arrives before any plot read can report one.
            while let Ok(event) = live_event_rx.try_recv() {
                match event {
                    tfg::backend::LiveEvent::GamePositions(update) => {
                        let fixes = tfg::gamefeed::game_fixes(&update);
                        // The socket's own line, throttled to match the
                        // plot's: which path is carrying the movement is
                        // worth saying out loud, and a line a second is
                        // noise.
                        wire_log_ticks += 1;
                        if wire_log_ticks % 10 == 0 {
                            eprintln!(
                                "wire feed: {} hull(s) on game {} @ {}",
                                fixes.len(),
                                update.game_id,
                                update.assumed_time
                            );
                        }
                        if !fixes.is_empty() {
                            registry.poll_game(fixes);
                        }
                    }
                    tfg::backend::LiveEvent::OrderIssued(event) => {
                        let fix = tfg::gamefeed::order_fix(&event);
                        registry.poll_game(vec![fix]);
                    }
                    tfg::backend::LiveEvent::Vanished(ids) => {
                        for id in ids {
                            registry.remove_ship(&id);
                        }
                    }
                    tfg::backend::LiveEvent::Connected { client_id } => {
                        eprintln!("wire: connected ({client_id})");
                    }
                    tfg::backend::LiveEvent::Reconnecting { attempt, wait_secs } => {
                        eprintln!("wire: reconnecting (attempt {attempt}, {wait_secs}s)");
                    }
                    tfg::backend::LiveEvent::Refused { code, reason } => {
                        eprintln!("wire: refused ({code}): {reason}");
                    }
                    tfg::backend::LiveEvent::SocketError(e) => {
                        eprintln!("wire: socket error: {e}");
                    }
                    tfg::backend::LiveEvent::RefreshDue => {
                        eprintln!("wire: token expired (109) — the socket needs a new one");
                    }
                    tfg::backend::LiveEvent::Message(m) => {
                        eprintln!("wire: message ({}) {}", m.kind, m.content);
                    }
                    tfg::backend::LiveEvent::Announced(list) => {
                        eprintln!("wire: {} vessel(s) announced", list.len());
                    }
                    tfg::backend::LiveEvent::Resynced => {
                        eprintln!("wire: resynced");
                    }
                }
            }
            while let Ok(fixes) = poll_rx.try_recv() {
                registry.poll(fixes);
            }
            for view in registry.ships() {
                let (clat, clon, zoom) = camera_fix.lock().map(|c| *c).unwrap_or((
                    tfg::map_render::MAP_CENTER.0,
                    tfg::map_render::MAP_CENTER.1,
                    tfg::map_render::MAP_ZOOM,
                ));
                let (x, y) = tfg::map_render::project_mercator(
                    view.latest.position.latitude,
                    view.latest.position.longitude,
                    (clat, clon),
                    zoom,
                    tfg::map_render::MAP_W,
                    tfg::map_render::MAP_H,
                );
                let symbol = resolve_symbol(&view.ship_id, &store_ctx, &classes);
                let affiliation = resolve_affiliation(&view.ship_id, &store_ctx);
                batch.push(tfg::shell_gpui::LiveMarker {
                    id: view.ship_id,
                    x,
                    y,
                    stale: view.stale,
                    symbol,
                    affiliation,
                    heading_deg: view.latest.heading_deg,
                    speed_kn: view.latest.speed_kn,
                    latitude: view.latest.position.latitude,
                    longitude: view.latest.position.longitude,
                    trail: view.trail.len(),
                });
            }
            if batch.is_empty() { None } else { Some(batch) }
        }),
        status_feed,
        signin_feed,
        create_feed,
        timeline_feed,
        judgements_feed,
        reviews_feed,
        judge_write_feed,
        review_write_feed,
        clock_feed,
        embarked_feed,
        embark_write_feed,
        thumb_feed,
        order_tx,
        roster,
        roster_total,
        session,
        fleet_tax,
    );
}

/// Decode a unit picture, honouring the orientation its own metadata declares.
///
/// A photograph from a camera carries an EXIF `Orientation` tag instead of
/// rotating its pixels (JPEG's most common encoding of "this was shot
/// sideways"), and `image::load_from_memory` deliberately ignores it — the tag
/// is metadata, and decoding does what the bytes say. A top-down shot stored
/// as "rotate 90" therefore decodes on its side, and a hull on its side on a
/// north-up chart reads as a hull steaming north when it is not. That looks
/// like a projection bug and is not one.
///
/// The tag is part of what the pixels MEAN, so it is applied here, once, before
/// the picture goes anywhere: every consumer downstream (the map, the picker's
/// rows, the drag ghost) then shares one upright frame.
///
/// A format without an orientation tag decodes exactly as before: PNG says
/// `NoTransforms`, and so does a JPEG that carries none.
fn decode_picture(bytes: &[u8]) -> Option<image::DynamicImage> {
    use image::ImageDecoder as _;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    Some(img)
}

/// The camera centre that puts a hull on the map's VISIBLE centre.
///
/// `shift_px` is the shell's `camera_centre_shift`: how far the CONTENT moves to
/// clear the floating zone, positive to the right for a left dock. The camera
/// therefore moves the other way — a hull that must appear 180 px right of the
/// frame centre is one whose camera sits 180 px to its WEST. Unprojecting
/// `mw/2 + shift` would move the camera east and park the hull UNDER the zone,
/// which is exactly the half of the frame the shift exists to clear; that sign
/// error made the Inspector's `Centre` and the Fleet row's focus door look like
/// they did nothing at all, when both were centring the hull behind the column.
fn jump_goal(lat: f64, lon: f64, shift_px: f32, zoom: f64, w: f64, h: f64) -> (f64, f64) {
    tfg::map_render::unproject_mercator(
        w / 2.0 - f64::from(shift_px),
        h / 2.0,
        (lat, lon),
        zoom,
        w,
        h,
    )
}

/// Hull photographs, fetched off-thread: the manifest says which hulls
/// have pictures, one presigned URL per hull resolves the bytes, and each
/// decoded picture rides the channel the moment it lands — the picker fills
/// in progressively while the glyphs hold the rows. The picture keeps its own
/// resolution (see the cap below): the map paints it at the hull's real size,
/// so a thumbnail-sized copy would be a blurred hull at high zoom. Memory only
/// (no disk cache yet); a hull with no picture costs no request.
///
/// Runs at startup AND after every sign-in: a launch without credentials
/// fetches nothing, and a card sign-in later must not leave the rows
/// glyph-held forever. The shell keys by unit id and skips what it holds,
/// so a second pass re-fetches but never repaints.
fn spawn_thumb_fetch(
    host: String,
    token: String,
    thumb_tx: mpsc::Sender<Vec<tfg::shell_gpui::ThumbRaw>>,
) {
    std::thread::spawn(move || {
        let Ok(master) = tfg::backend::MinosMaster::new(&host) else {
            return;
        };
        let Ok(manifest) = master.unit_image_manifest(&token) else {
            eprintln!("thumbs: no manifest, glyphs hold every row");
            return;
        };
        eprintln!(
            "thumbs: manifest {} ({} pictured)",
            manifest.version,
            manifest.entries.len()
        );
        let dl = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build();
        let Ok(dl) = dl else { return };
        for e in manifest
            .entries
            .iter()
            .filter(|e| e.asset_kind == "unit_image")
        {
            let Ok(Some(url)) = master.hull_image_url(&token, e.unit_id) else {
                continue;
            };
            let bytes = dl.get(&url).send().and_then(|r| r.bytes());
            let Ok(bytes) = bytes else { continue };
            let Some(img) = decode_picture(&bytes) else {
                continue;
            };
            // The picture keeps its OWN resolution, capped only so a huge
            // upload cannot cost tens of megabytes per hull.
            //
            // It used to be `img.thumbnail(192, 192)` — a thumbnail in the
            // strict sense — and that is what made the map blurry: the map
            // paints the picture at the hull's REAL size, which at the
            // console's own `ZOOM_MAX` (18) is about 400 px for a destroyer,
            // so a 192 px bitmap was upscaled into mush exactly when the
            // operator zoomed in to look at it. The cap sits above the
            // largest box the map can ask for (a 400 m hull at 18 is ~1.3 k
            // px); the rows, the ghost and the banner downscale it
            // themselves, which is what a whole-picture element is for.
            const PICTURE_MAX_PX: u32 = 2048;
            let long = img.width().max(img.height());
            let picture = if long > PICTURE_MAX_PX {
                img.resize(
                    PICTURE_MAX_PX,
                    PICTURE_MAX_PX,
                    image::imageops::FilterType::Lanczos3,
                )
            } else {
                img
            };
            let upright = picture.to_rgba8();
            let (w, h) = (upright.width(), upright.height());
            if thumb_tx
                .send(vec![tfg::shell_gpui::ThumbRaw {
                    unit_id: e.unit_id,
                    w,
                    h,
                    rgba: upright.into_raw(),
                    // The manifest declares the hull's published measurements
                    // beside the picture, so the map can draw the photograph at
                    // true scale without a second request per hull. Both ride
                    // the one entry, and neither is invented: an unpublished
                    // figure travels as `None` and the map draws the glyph.
                    loa_m: e.loa_m,
                    beam_m: e.beam_m,
                }])
                .is_err()
            {
                return;
            }
        }
        eprintln!("thumbs: fetch pass done");
    });
}

/// The fleet picker's drill tree, built once off the synced mirror.
///
/// Lives here and not in the shell: the tree reads sqlite, and the shell
/// module is also compiled into the sqlite-free parity proto. Best-effort
/// per rung — a rung that fails to read comes back empty rather than
/// failing the whole tree, because a picker with three groups is honest
/// and a picker that refused to open over one bad row is not. Counts are
/// the readers' own live-hull counts, so a tab's number and the rows
/// behind it agree.
fn build_fleet_taxonomy(
    conn: &rusqlite::Connection,
    mut symbol_of: impl FnMut(&str) -> tfg::view::MapSymbolShape,
) -> tfg::shell_gpui::FleetTaxonomy {
    use tfg::shell_gpui::{FleetTaxCat, FleetTaxClass, FleetTaxGroup, FleetTaxLeaf, FleetTaxType};
    use tfg::store::{
        asset_groups, tax_categories_in_asset_group, tax_classes, tax_types, tax_units,
    };
    let mut groups = Vec::new();
    for g in asset_groups(conn).unwrap_or_default() {
        let mut cats = Vec::new();
        for c in tax_categories_in_asset_group(conn, Some(g.id)).unwrap_or_default() {
            let mut types = Vec::new();
            for t in tax_types(conn, c.id).unwrap_or_default() {
                let mut classes = Vec::new();
                for cl in tax_classes(conn, t.id).unwrap_or_default() {
                    let units = tax_units(conn, cl.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|u| FleetTaxLeaf {
                            symbol: symbol_of(&u.id),
                            id: u.id,
                            name: u.name,
                            hull: u.hull,
                            class_name: u.class_name,
                        })
                        .collect();
                    classes.push(FleetTaxClass {
                        id: cl.id,
                        name: cl.name,
                        count: cl.count,
                        units,
                    });
                }
                types.push(FleetTaxType {
                    id: t.id,
                    name: t.name,
                    count: t.count,
                    classes,
                });
            }
            cats.push(FleetTaxCat {
                id: c.id,
                name: c.name,
                count: c.count,
                types,
            });
        }
        groups.push(FleetTaxGroup {
            id: g.id,
            name: g.name,
            count: g.count,
            cats,
        });
    }
    tfg::shell_gpui::FleetTaxonomy { groups }
}

/// The hull register over the dev API, as the console's own client reads it.
///
/// Returns the rows and the access token together: the token opens the live
/// wire too, so logging in once serves both the register and the feed.
///
/// `MinosMaster::units_named` rather than `units`: the sheet wants a class
/// NAME, and `units` flattens the taxonomy to ids by design (the console
/// resolves them from the catalog after a sync).
/// The register behind an access token — the one read both the
/// password login and the keyring refresh go through, so the rows are
/// built once.
fn roster_with_token(host: &str, token: &str) -> Option<Vec<tfg::shell_gpui::RosterRow>> {
    let master = tfg::backend::MinosMaster::new(host).ok()?;
    let units = master.units_named(token).ok()?;
    Some(
        units
            .into_iter()
            .map(|u| tfg::shell_gpui::RosterRow {
                unit_id: u.id.to_string(),
                name: u.name,
                hull: u.hull_number,
                class: format!("{} · {}", u.class_name, u.category_name),
                stale: false,
            })
            .collect(),
    )
}

fn fetch_roster(
    host: &str,
    user: &str,
    pass: &str,
) -> Option<(Vec<tfg::shell_gpui::RosterRow>, String, Option<String>)> {
    let auth = tfg::backend::MinosAuth::new(host).ok()?;
    let pair = auth.login(user, pass).ok()?;
    let rows = roster_with_token(host, &pair.access_token)?;
    Some((rows, pair.access_token, pair.refresh_token))
}

/// The keyring's stored refresh token for the last user, rotated into a
/// live session so a returning launch skips the card. `None` when
/// nothing is stored, the store is unreadable, or the rotation is
/// refused — each a fresh card, never a crash. The rotation mints a new
/// refresh token; storing it back keeps the next launch on the keyring.
fn try_keyring_login(
    host: &str,
    last_user_path: &Path,
) -> Option<(Vec<tfg::shell_gpui::RosterRow>, String, String)> {
    let user = tfg::backend::last_user_load(last_user_path)?;
    let stored = match tfg::backend::keyring_load(&user) {
        Ok(Some(rt)) => rt,
        _ => return None,
    };
    let auth = tfg::backend::MinosAuth::new(host).ok()?;
    let pair = auth.refresh(&stored).ok()?;
    if let Some(rt) = &pair.refresh_token {
        let _ = tfg::backend::keyring_save(&user, rt);
    }
    let rows = roster_with_token(host, &pair.access_token)?;
    Some((rows, pair.access_token, user))
}

/// The store's resolved picture, read once: the id-keyed symbol resolver, the
/// session's affiliation resolver, and each hull's taxonomy.
type StoreCtx = (
    tfg::store::SymbolResolver,
    tfg::store::AffiliationResolver,
    std::collections::HashMap<String, (Option<i64>, Option<i64>, Option<i64>, Option<i64>)>,
);

/// A hull's glyph shape: the store's id-keyed resolver first, the register's
/// class name as the fallback.
///
/// The console resolves only from the ids (type, then category, then domain)
/// and never guesses from a name; the GPUI binary keeps that as the rule. The
/// name bridge fires for a hull the resolver has NEVER SEEN — no assignment
/// for its type, category or domain — which on a fresh store is every hull,
/// because a fresh store has no assignments at all. An assignment the
/// operator DID make wins even when it says `UnknownShip`: the tables are the
/// operator's statement, and overriding it from a class name is exactly the
/// guessing the console refuses to do.
fn resolve_symbol(
    id: &str,
    ctx: &Option<StoreCtx>,
    classes: &std::collections::HashMap<String, String>,
) -> tfg::view::MapSymbolShape {
    let from_store = ctx.as_ref().and_then(|(resolver, _, tax)| {
        let (type_id, category_id, domain_id, _) = tax.get(id).copied()?;
        let known = type_id.is_some_and(|t| resolver.by_type.contains_key(&t))
            || category_id.is_some_and(|c| resolver.by_category.contains_key(&c))
            || domain_id.is_some_and(|d| resolver.by_domain.contains_key(&d));
        known.then(|| {
            resolver.resolve(tfg::store::UnitTaxonomy {
                type_id,
                category_id,
                domain_id,
            })
        })
    });
    let symbol = from_store.unwrap_or_else(|| {
        tfg::markers::symbol_from_class_name(classes.get(id).map(String::as_str).unwrap_or(""))
    });
    tfg::markers::map_symbol_shape(symbol)
}

/// A hull's affiliation, from the store's session declarations.
///
/// This machine has no session, so the declaration tables are empty and every
/// hull resolves Unknown — which is what the resolver is FOR: it answers, and
/// "nobody has said" is a real answer. The branch is the one fact the mirror
/// carries, and `owned` is false because nothing here is commanded.
fn resolve_affiliation(id: &str, ctx: &Option<StoreCtx>) -> tfg::symbology::Affiliation {
    let Some((_, affiliations, tax)) = ctx.as_ref() else {
        return tfg::symbology::Affiliation::Unknown;
    };
    let branch_id = tax.get(id).map(|(_, _, _, b)| *b).flatten();
    affiliations.resolve(&tfg::store::AffiliationSubject {
        unit_id: id.parse::<i64>().ok(),
        group_id: None,
        branch_id,
        owned: false,
    })
}

/// Bring the live socket up on a token, and keep the console's handle on it.
///
/// The wire's command channel belongs to the wire that is up NOW: the actor
/// owns the receiving end, so a wire that is swapped for a new one takes a
/// new channel with it. The console's handle is therefore a cell, and the
/// watches are re-declared on the new channel — a fresh actor watches
/// nothing until it is told, which is the rule the egui console followed
/// when it re-declared after a sign-in.
///
/// The events channel is shared: every wire reports into the same place, and
/// the console reads one stream.
fn start_live_wire(
    host: &str,
    token: &str,
    wire_ctl_tx: &mpsc::Sender<tfg::shell_threads::WireKind>,
    live_ctl: &tfg::gamefeed::LiveCtl,
    event_tx: &mpsc::Sender<tfg::backend::LiveEvent>,
    wake_tx: &mpsc::Sender<()>,
) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let _ = wire_ctl_tx.send(tfg::shell_threads::WireKind::Live {
        ws_url: tfg::backend::websocket_url(host),
        rest_base: host.to_string(),
        token: token.to_string(),
        cmd_tx: cmd_tx.clone(),
        cmd_rx,
        event_tx: event_tx.clone(),
        wake_tx: wake_tx.clone(),
    });
    if let Ok(mut slot) = live_ctl.lock() {
        *slot = Some(cmd_tx);
    }
    eprintln!("wire: live {}", tfg::backend::websocket_url(host));
}

/// Write the held session into the shared cell, and re-declare the socket's
/// watches for it.
///
/// One call, because the two belong together: a watch declared for a
/// session the cell does not name is a watch on somebody else's exercise.
fn sync_session(
    hold: &tfg::gamefeed::GameHoldCell,
    live_ctl: &tfg::gamefeed::LiveCtl,
    token: &str,
    game_id: i64,
    state: &str,
) {
    tfg::gamefeed::set_hold(hold, token, game_id, state);
    tfg::gamefeed::sync_watch(live_ctl, hold);
}

/// The held session: the game this account is SEATED in, and its room.
///
/// Read-only on purpose. The console JOINS (a room key, a write) and reads the
/// seat from the answer; the prototype reads `/users/me` and looks for its own
/// row in each game's roster instead — no write, and the same seat. A game
/// whose participants are unreadable (403) is skipped, because a room this
/// account cannot read is not a room it holds.
/// The games one account can see, for the session picker: every game in
/// the server's own order, each with this seat in it (role name and judge
/// side, empty/false when watching). One participants read per game — the
/// same fan-out `fetch_session` already pays to choose the hold — so the
/// picker and the hold never disagree about where this account sits.
fn fetch_session_rows(host: &str, token: &str) -> Option<Vec<tfg::shell_gpui::SessionRow>> {
    let master = tfg::backend::MinosMaster::new(host).ok()?;
    let (me, _name) = master.me(token).ok()?;
    let games = master.games_list(token).ok()?;
    let mut rows = Vec::new();
    for g in &games {
        let Ok(parts) = master.game_participants(token, g.id) else {
            continue;
        };
        let (role, judge) = match parts.iter().find(|p| p.user_id == me) {
            Some(p) => (p.role_name.clone(), p.judge),
            None => (String::new(), false),
        };
        rows.push(tfg::shell_gpui::SessionRow {
            id: g.id,
            name: g.name.clone(),
            state: g.state.clone(),
            my_role: role,
            judge,
            mode: g.mode.clone(),
        });
    }
    Some(rows)
}

fn fetch_session(
    host: &str,
    token: &str,
    pinned: Option<i64>,
) -> Option<tfg::shell_gpui::SessionView> {
    let master = tfg::backend::MinosMaster::new(host).ok()?;
    let (me, _name) = master.me(token).ok()?;
    let games = master.games_list(token).ok()?;
    // The newest game this account is seated in wins; the newest readable one
    // is the fallback, so a caller seated nowhere still sees the exercise.
    // `pinned` holds one game regardless of seating: `TFG_GAME` at startup
    // (the headless-capture seed and the way to photograph a panel whose data
    // lives in a game this account is not seated in), or a just-created game
    // the composer holds the way the console's `apply_created_game` does.
    let held: Option<(i64, String, String, Vec<tfg::backend::Participant>)> =
        if let Some(id) = pinned {
            games.iter().find(|g| g.id == id).map(|g| {
                let parts = master.game_participants(token, id).unwrap_or_default();
                (id, g.name.clone(), g.state.clone(), parts)
            })
        } else {
            // Prefer, in order: a game where this account is seated as a
            // PLAYER (a judge commands nothing, so a judge seat can never fill
            // the panel), a game where it is seated at all, then the newest
            // readable one. The player pass is what finds the game a commander
            // is actually in; without it the newest seated game wins and this
            // account's judge seats shadow the one seat it commands from.
            let mut player = None;
            let mut seated = None;
            let mut readable = None;
            for g in &games {
                let Ok(parts) = master.game_participants(token, g.id) else {
                    continue;
                };
                let (is_seated, is_player) = match parts.iter().find(|p| p.user_id == me) {
                    Some(p) => (true, !p.judge),
                    None => (false, false),
                };
                if is_player && player.is_none() {
                    player = Some((g.id, g.name.clone(), g.state.clone(), parts));
                } else if is_seated && seated.is_none() {
                    seated = Some((g.id, g.name.clone(), g.state.clone(), parts));
                } else if readable.is_none() {
                    readable = Some((g.id, g.name.clone(), g.state.clone(), parts));
                }
            }
            player.or(seated).or(readable)
        };
    let (game_id, game, state, parts) = held?;
    // The game's own terms. `GET /games/{id}` — the read the shell spent its
    // whole life without, which is why Essentials, Control and Review had no
    // bodies and their slots were omitted from the column rather than filled.
    // `facts.read` records whether it ANSWERED: a read this account may not
    // make is a different thing from a game with nothing written on it, and
    // Essentials says which it is looking at.
    let facts = match master.game_detail(token, game_id) {
        Ok(d) => tfg::shell_gpui::GameFacts {
            mode: d.mode,
            // An unrecognised pace keeps its own token AND its ceremony: the
            // server's word is shown as read, and nothing is inferred from it.
            pace: match d.pace {
                Some(tfg::backend::GamePace::Standard) => "standard".to_string(),
                Some(tfg::backend::GamePace::Fast) => "fast".to_string(),
                None => "unrecognised".to_string(),
            },
            description: d.description,
            purpose: d.purpose,
            target: d.target,
            actual_start: d.window.actual_start.unwrap_or_default(),
            actual_end: d.window.actual_end.unwrap_or_default(),
            assumed_start: d.window.assumed_start.unwrap_or_default(),
            assumed_end: d.window.assumed_end.unwrap_or_default(),
            time_factor: d.time_factor,
            room_key: d.room_key.unwrap_or_default(),
            read: true,
        },
        Err(_) => tfg::shell_gpui::GameFacts::default(),
    };
    // The published checklist. `None` for a planning room is the ordinary
    // case (the route 404s until preparation), not a failure to report.
    let readiness = master.game_readiness(token, game_id).ok().map(|r| {
        tfg::shell_gpui::ReadinessView {
            can_execute: r.can_execute,
            blockers: r.blockers,
        }
    });
    // The hulls this seat commands: `id_commander` is the same key the seat
    // carries, so the join needs no name lookup — the roster's rule again.
    let units = master.game_units_list(token, game_id).unwrap_or_default();
    let mine = units
        .iter()
        .filter(|u| u.commander_id == Some(me))
        .map(|u| tfg::shell_gpui::MyShip {
            id: u.unit_id.to_string(),
            label: format!("{} ({})", u.unit_name, u.hull_number),
        })
        .collect();
    // The pieces every relate checkbox names: hulls assigned to THIS game,
    // because the server refuses any other hull with a 400. Same list the
    // seat joins its hulls from — one read, two readers.
    // Which of them sits on the map, and where. A separate read because it
    // is a separate fact: a hull is in the exercise whether or not it has
    // been placed, and the console's draft carries the same two fields side
    // by side. Best-effort — an unreadable placements list leaves every hull
    // unplaced rather than failing the whole session read.
    //
    // The coordinates are kept, not only the fact: a focus control in the
    // crew screen has no live fix to aim at in a fresh execution, and the
    // placement is the ground the exercise actually recorded.
    let placements: Vec<(i64, f64, f64)> = master
        .placements_list(token, game_id)
        .map(|p| {
            p.placements
                .iter()
                .map(|g| (g.unit_id, g.latitude, g.longitude))
                .collect()
        })
        .unwrap_or_default();
    let pieces: Vec<tfg::shell_gpui::PieceView> = units
        .iter()
        .map(|u| {
            let start = placements
                .iter()
                .find(|(id, _, _)| *id == u.unit_id)
                .map(|(_, lat, lon)| (*lat, *lon));
            tfg::shell_gpui::PieceView {
                id: u.unit_id,
                label: format!("{} ({})", u.unit_name, u.hull_number),
                commander_id: u.commander_id,
                commander_name: u.commander_name.clone(),
                hierarchy_node: u.hierarchy_node,
                start,
                placed: start.is_some(),
            }
        })
        .collect();
    // The task organisation, flattened from the server's recursive forest.
    // The read names the LEVEL, never a rank; an echelon we do not
    // recognise is not this client's rank to invent, so its node is
    // dropped. The node's commander IS carried — the crew screen shows a
    // commander's task group beneath their seat. Member hulls arrive on the
    // pieces (each carries its `hierarchy_node`), so the draft adds them
    // from there rather than from this tree.
    //
    // THE MEMBERSHIP IS JOINED HERE, from the pieces read above: the server
    // publishes a unit's group on the unit's own row and nowhere else, so
    // the join is one map away. Without it every group reads as "0 hulls"
    // and the group diff cannot see a unit that is in a node the server
    // holds — which is what makes an un-place unreachable.
    fn flatten_groups(
        nodes: &[tfg::backend::HierarchyNode],
        out: &mut Vec<tfg::groups::LiveGroup>,
    ) {
        for n in nodes {
            if let Some(kind) = tfg::groups::GroupKind::from_echelon(&n.echelon_name) {
                out.push(tfg::groups::LiveGroup {
                    group_id: n.id,
                    name: n.name.clone(),
                    kind,
                    parent: n.parent_id,
                    commander_id: n.commander_id,
                    units: Vec::new(),
                });
            }
            flatten_groups(&n.children, out);
        }
    }
    let mut groups = master
        .game_hierarchy(token, game_id)
        .map(|forest| {
            let mut out = Vec::new();
            flatten_groups(&forest, &mut out);
            out
        })
        .unwrap_or_default();
    // The membership, joined: the unit rows carry the node they sit in, so
    // the hulls of a node are the rows that name it.
    let members: Vec<(i64, i64)> = pieces
        .iter()
        .filter_map(|p| p.hierarchy_node.map(|node| (p.id, node)))
        .collect();
    tfg::groups::join_members(&mut groups, &members);
    // WHICH PIECE IS ABOARD WHICH. A hull's host is on no piece row: the
    // server publishes it as the CONTENTS of the host, and the only route
    // is `GET /games/{id}/units/{unit_id}/embarkations`. So this is one
    // read per piece in the exercise — the same fan-out the related read
    // already makes per step, and paid for ONCE per session read.
    //
    // A refused read means "this piece carries nothing", not a broken
    // session: the route 404s for a piece that was never a host, and the
    // answer we want for such a piece is exactly "no contents".
    let mut hosts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for piece in &pieces {
        if let Ok(contents) = master.game_embarkations(token, game_id, piece.id) {
            for child in contents {
                hosts.insert(child.id, piece.id);
            }
        }
    }
    let echelons = master.echelon_levels(token).unwrap_or_default();
    // The game's own roles, as the seat form offers them. Read rather than
    // hardcoded: a deployment that relabels a seat keeps working, and the
    // judge flag is the only thing the mirror says about who steers.
    let roles: Vec<tfg::shell_gpui::RoleRow> = master
        .game_roles(token, game_id)
        .unwrap_or_default()
        .into_iter()
        .map(|r| tfg::shell_gpui::RoleRow {
            id: r.id,
            name: if r.name.is_empty() { r.id_name.clone() } else { r.name.clone() },
            judge: r.judge.unwrap_or(false),
        })
        .collect();
    // The directory, which is a question about ACCOUNTS rather than about the
    // exercise. Read here and not on the session's every refresh: it is large,
    // it changes on Minos's own schedule, and the console reads it on the way
    // INTO the picker for the same reason.
    let directory: Vec<tfg::shell_gpui::DirectoryRow> = master
        .users_list(token, "", None, None)
        .unwrap_or_default()
        .into_iter()
        .map(|u| tfg::shell_gpui::DirectoryRow {
            id: u.id,
            name: if u.name.is_empty() { u.username.clone() } else { u.name.clone() },
            unit: u.satuan,
            rank: u.pangkat,
            appointment: u.jabatan,
        })
        .collect();
    // The book, then what each of its steps obliges: steps arrive only
    // nested, so the related read fans out per step. Best-effort per step —
    // one unreadable step leaves an absent entry, never no session.
    let scenarios = master.game_scenarios(token, game_id).unwrap_or_default();
    let related: Vec<tfg::shell_gpui::StepRelatedView> = scenarios
        .iter()
        .flat_map(|s| {
            s.steps.iter().filter_map(|st| {
                master
                    .step_related(token, game_id, s.id, st.id)
                    .ok()
                    .map(|r| tfg::shell_gpui::StepRelatedView {
                        scenario_id: s.id,
                        step_id: st.id,
                        posts: r.posts.iter().map(|p| (p.kind.clone(), p.id)).collect(),
                        names: r.posts.iter().map(|p| p.name.clone()).collect(),
                        personnel: r
                            .personnel
                            .iter()
                            .map(|p| {
                                if p.call_sign.trim().is_empty() {
                                    format!("user {}", p.user_id)
                                } else {
                                    p.call_sign.clone()
                                }
                            })
                            .collect(),
                    })
            })
        })
        .collect();
    // Each step's GATE, beside its related list and only while the exercise
    // runs: a planning room is not playing a step, so the read there would be
    // asking about a gate that has not opened. Best-effort per step like the
    // related fan-out — one unreadable step leaves an absent gate, never no
    // session — and it is what the Scenario board needs, because
    // `current-step` carries a gate only for the caller a step ASKS, and no
    // step asks the Game Master.
    let gates: Vec<tfg::shell_gpui::StepGateView> = if state == "execution" {
        scenarios
            .iter()
            .flat_map(|s| {
                s.steps.iter().filter_map(|st| {
                    master
                        .step_readiness(token, game_id, s.id, st.id)
                        .ok()
                        .map(|r| tfg::shell_gpui::StepGateView {
                            scenario_id: s.id,
                            step_id: st.id,
                            may_start: r.may_start,
                            required: r.required,
                            ready: r.ready,
                            outstanding: r
                                .personnel
                                .iter()
                                .filter(|p| !p.ready)
                                .map(|p| {
                                    if p.call_sign.trim().is_empty() {
                                        format!("user {}", p.user_id)
                                    } else {
                                        p.call_sign.clone()
                                    }
                                })
                                .collect(),
                        })
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let book = scenarios
        .into_iter()
        .map(|s| tfg::shell_gpui::BookScenario {
            id: s.id,
            title: s.title,
            steps: s
                .steps
                .into_iter()
                .map(|st| tfg::shell_gpui::BookStep {
                    id: st.id,
                    content: st.content,
                    window: st.start_hour.zip(st.end_hour),
                })
                .collect(),
        })
        .collect();
    // The Assessment island's three reads, and ONLY for a closed game.
    //
    // Assessment is a Closure-only island, so asking Minos for a debrief of a
    // game that is still being planned is a read this state does not need --
    // and on the evidence of the routes, one it may refuse. Gating on the
    // state rather than fetching always and hiding is the honest version: the
    // data is absent because it was not asked for, which is a different
    // sentence from "Minos has none".
    let closed = state == "closure";
    // The first page AND its cursor block: dropping the cursor would strand
    // "load more" undrawable even while Minos holds more. The block rides
    // the session beside the events — one read, not two answers — so a
    // session installed whole carries its own block with it.
    let (timeline, timeline_cursor, timeline_has_more) = if closed {
        master
            .timeline_page(token, game_id, None, None, None, None, None, None, 50)
            .map(|page| (page.events, page.next_cursor, page.has_more))
            .unwrap_or_default()
    } else {
        (Vec::new(), None, false)
    };
    let judgements = if closed {
        master.judgements_list(token, game_id, None).unwrap_or_default()
    } else {
        Vec::new()
    };
    let reviews = if closed {
        master.reviews_list(token, game_id, None, None).unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(tfg::shell_gpui::SessionView {
        // The camera's opening zoom is the MAP's constant and the caller sets
        // it: the session read has no business knowing the map's defaults.
        zoom: 0.0,
        signed_in: true,
        timeline,
        timeline_cursor,
        timeline_has_more,
        judgements,
        reviews,
        game,
        state: state.clone(),
        seats: parts.len(),
        present: parts.iter().filter(|p| p.joined_at.is_some()).count(),
        ready: parts.iter().filter(|p| p.ready).count(),
        judges: parts.iter().filter(|p| p.judge).count(),
        me,
        my_role: parts
            .iter()
            .find(|p| p.user_id == me)
            .map(|p| p.role_name.clone())
            .unwrap_or_default(),
        my_ready: parts.iter().find(|p| p.user_id == me).is_some_and(|p| p.ready),
        execution: state == "execution",
        mine,
        game_id,
        messages: master
            .inbox_list(token, game_id, None, false)
            .unwrap_or_default()
            .into_iter()
            .map(|m| tfg::shell_gpui::MessageView {
                sender: m.sender,
                kind: m.kind,
                class_label: m.class_label,
                content: m.content,
                at: m.created_at,
                broadcast: m.broadcast,
            })
            .collect(),
        // The step read. `None` is a real state — the server stamps a step
        // only once the scenario clock has an anchor — and the panel says so
        // rather than drawing an empty step.
        step: master
            .game_current_step(token, game_id)
            .ok()
            .map(|v| tfg::shell_gpui::StepView {
                assumed_time: v.assumed_time.unwrap_or_default(),
                playing: v.current.map(|p| tfg::shell_gpui::StepLine {
                    scenario_id: p.scenario.id,
                    step_id: p.step.id,
                    scenario: p.scenario.title,
                    content: p.step.content,
                    state: p.state,
                }),
                asking: v.asking.map(|a| tfg::shell_gpui::StepAsk {
                    scenario_id: a.scenario.id,
                    step_id: a.step.id,
                    scenario: a.scenario.title,
                    content: a.step.content,
                    required: a.required,
                    outstanding: a.outstanding,
                    you_ready: a.you_ready,
                }),
            }),
        people: parts
            .iter()
            .map(|p| tfg::shell_gpui::PersonView {
                id: p.user_id,
                name: p.user_name.clone(),
                call_sign: p.call_sign.clone(),
                role: p.role_name.clone(),
                role_id: p.role_id,
                judge: p.judge,
                ready: p.ready,
                present: p.joined_at.is_some(),
            })
            .collect(),
        // The book, the pieces and the related lists computed above: one
        // session read, three readers — the panel, the relate editor and the
        // step counts all drink from it.
        book,
        pieces,
        hosts,
        groups,
        echelons,
        related,
        gates,
        facts,
        readiness,
        directory,
        roles,
    })
}

/// LEAVE PLANNING: write the whole staged draft through, then move the
/// exercise to preparation.
///
/// The plan is `tfg::advance::plan`'s, and this function only executes it:
/// the order, the rules and the reasons all live there, where they can be
/// read and tested without a session. What lives HERE is the three things
/// only this side can do — ask the server whether this seat may drive the
/// exercise at all, read the server's own frame so the diff is against the
/// truth, and name the server's refusals in the console's words.
///
/// THE THREE READS, AND WHY THE FIRST ONE COMES FIRST.
///
/// - THE AUTHORITY. `GET /game-roles/{id}/permissions` is the only read
///   that carries a role's grants, and it is gated on the ADMINISTRATOR's
///   grant — so an administrator learns the answer and a Game Master
///   participant cannot. The console does not guess on the participant's
///   behalf: an answer it cannot have is an answer it does not have, and
///   the server's own refusal is the one that counts.
/// - THE LIVE FRAME. The server holds whatever the last attempt managed to
///   write, and a diff against the truth is what makes a retry emit NOTHING
///   for it instead of re-issuing a write the server answers 409.
/// - THE TERMS. The planned window is written against the window the server
///   holds, so only the clock keys that changed go on the wire.
fn advance_to_preparation(
    master: &tfg::backend::MinosMaster,
    host: &str,
    token: &str,
    game_id: i64,
    force: tfg::force::ForceDraft,
    live_hulls: Vec<tfg::force::LiveHull>,
    roster: tfg::roster::RosterDraft,
    live_seats: Vec<tfg::roster::LiveSeat>,
    groups: Vec<tfg::groups::GroupWrite>,
    window: tfg::window::WindowDraft,
    book: tfg::book::BookDraft,
    live_scenarios: Vec<tfg::book::LiveScenario>,
) -> Result<tfg::shell_gpui::CreatedSession, String> {
    // THE AUTHORITY, before anything is written. `Some(false)` is a seat
    // that KNOWS it may not drive the exercise; `None` is a seat that cannot
    // find out, and the server decides for it.
    match may_run_the_exercise(master, host, token, game_id) {
        Some(false) => {
            return Err(
                "this seat does not run the exercise \u{2014} the Game Master drives it, \
                 so nothing was written"
                    .to_string(),
            )
        }
        None => eprintln!(
            "advance: this seat's grants are not knowable to the console \
             (the permission matrix is the administrator's read), so the server decides"
        ),
        Some(true) => {}
    }

    // THE SERVER'S FRAME, re-read. The mirror that crossed with the drafts
    // is the console's last word about the server; after a failed attempt
    // it is a stale one, and a diff against a stale frame re-issues the
    // writes the server already took.
    let live_seats = read_live_seats(master, token, game_id, live_seats);
    let live_hulls = read_live_hulls(master, token, game_id, live_hulls);

    // THE TERMS. The window is written only against the window the server
    // holds: a window the console could not compare would be a stale copy
    // of somebody else's plan, and the terms are not the console's to
    // overwrite.
    let window_write = match master.game_detail(token, game_id) {
        Ok(detail) => tfg::advance::window_update(&window, &detail.window),
        Err(e) => {
            eprintln!("advance: the exercise's terms could not be read ({e}); the window is left alone");
            Ok(None)
        }
    };
    let window_write = match window_write {
        Ok(write) => write,
        Err(why) => return Err(why),
    };

    // THE BOOK, planned against the book's own live frame. The book is
    // written through LAST, and the order is the server's: a step's related
    // list names hulls and task-organisation nodes, so the force and the
    // groups have to exist before it is posted, and a new scenario's steps
    // have to come after the scenario.
    let book_writes = tfg::book::plan(&book, &live_scenarios);

    // THE PLAN. Every rule is checked here, before the first write.
    let steps = match tfg::advance::plan(
        &force,
        &live_hulls,
        &roster,
        &live_seats,
        &groups,
        &book_writes,
        window_write,
    ) {
        Ok(steps) => steps,
        Err(refusal) => return Err(refusal.to_string()),
    };

    // THE WRITES, in the plan's order. The first refusal stops the rest.
    //
    // THE CREATED NODES TRAVEL FROM THE GROUPS TO THE BOOK, because a step's
    // related list may name a node this same flush created, and the node has
    // no server id until the flush that created it has made one.
    match run_plan(
        master,
        token,
        game_id,
        &steps,
        &live_scenarios,
        true,
    ) {
        Ok(written) => {
            eprintln!("advance: {written} write(s) and the exercise moved to preparation");
            match fetch_session(host, token, Some(game_id)) {
                Some(session) => Ok(tfg::shell_gpui::CreatedSession::held(session)),
                None => Err("moved to preparation, but the session could not be read".to_string()),
            }
        }
        Err(why) => {
            // THE SERVER TOOK PART OF THE DRAFT. The exercise is re-read so
            // the operator sees the truth, and the answer carries it: the
            // next attempt diffs against what the server holds, not against
            // what it held before. The operator's staged work is theirs and
            // is not touched here.
            match fetch_session(host, token, Some(game_id)) {
                Some(session) => Ok(tfg::shell_gpui::CreatedSession {
                    session,
                    note: why.clone(),
                    refusal: Some(why),
                    write: tfg::shell_gpui::WriteId::Prepare,
                }),
                None => Err(why),
            }
        }
    }
}

/// Run a plan's steps, in the plan's order, and report how many writes
/// landed.
///
/// `transition` says whether the plan's last step — the move to preparation
/// — is part of this run. It is not the same run for the advance and for the
/// sync: the advance moves the exercise on, and the sync writes the same
/// staged records and leaves the exercise where it is. The two share the
/// order because they share the server's rules, and a console that wrote the
/// records in two orders would be two answers to one question.
///
/// THE CREATED NODES TRAVEL FROM THE GROUPS TO THE BOOK inside this
/// function, because a step's related list may name a node this same flush
/// created, and the node has no server id until the flush that created it
/// has made one.
fn run_plan(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    steps: &[tfg::advance::Step],
    live_scenarios: &[tfg::book::LiveScenario],
    transition: bool,
) -> Result<usize, String> {
    let mut created: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    let mut written = 0;
    for step in steps {
        if !transition && matches!(step, tfg::advance::Step::Transition) {
            continue;
        }
        let outcome: Result<(), String> = match step {
            // THE TERMS. The window is the exercise's own terms, and the
            // ticket for them is the ADMINISTRATOR's — a different grant
            // from the one that moves the exercise, which is the Game
            // Master's. So a refusal here stops the run BEFORE anything is
            // written: the seats, the force and the groups are untouched,
            // the game is still in `planning` (so the terms can still be
            // written by somebody who holds the ticket), and the operator's
            // staged work is theirs.
            tfg::advance::Step::Window(update) => master
                .update_game(token, game_id, update)
                .map(|_| ())
                .map_err(|e| window_refusal(&e)),
            tfg::advance::Step::Seat(write)
            | tfg::advance::Step::Role(write)
            | tfg::advance::Step::Unseat(write) => {
                flush_roster(master, token, game_id, std::slice::from_ref(write))
            }
            tfg::advance::Step::Force(write) => {
                flush_force(master, token, game_id, std::slice::from_ref(write))
            }
            tfg::advance::Step::Groups(writes) => {
                flush_groups(master, token, game_id, writes).map(|made| {
                    created = made;
                })
            }
            tfg::advance::Step::Book(writes) => {
                flush_book(master, token, game_id, writes, live_scenarios, &created)
            }
            tfg::advance::Step::Transition => master
                .transition_game(token, game_id, "preparation")
                .map(|_| ())
                .map_err(|e| transition_refusal(&e)),
        };
        match outcome {
            Ok(()) => written += 1,
            Err(why) => return Err(why),
        }
    }
    Ok(written)
}

/// Whether this seat may move the exercise on, as far as the console can
/// know.
///
/// THE ONLY READ THAT CARRIES A ROLE'S GRANTS IS THE ADMINISTRATOR'S.
/// `GET /game-roles/{id}/permissions` answers with a role's whole matrix,
/// and it is gated on the application-level role-vocabulary read — so an
/// administrator learns the answer and a Game Master participant cannot.
/// `None` means "not knowable", and the console says so rather than
/// guessing: a participant's refusal is the server's to make, and the
/// console's job is to name it when it comes.
///
/// The grant itself is `/games/transitions` with the action `transition` —
/// "Drive the exercise" in the vocabulary the server publishes.
fn may_run_the_exercise(
    master: &tfg::backend::MinosMaster,
    host: &str,
    token: &str,
    game_id: i64,
) -> Option<bool> {
    let (me, _name) = master.me(token).ok()?;
    let parts = master.game_participants(token, game_id).ok()?;
    let role_id = parts.iter().find(|p| p.user_id == me)?.role_id;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let response = client
        .get(&format!("{host}/game-roles/{role_id}/permissions"))
        .bearer_auth(token)
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().ok()?;
    let permissions = body["data"]["permissions"].as_array()?;
    Some(permissions.iter().any(|p| {
        p["object"].as_str() == Some("/games/transitions")
            && p["action"].as_str() == Some("transition")
    }))
}

/// The seats as the server holds them NOW, or the ones that crossed if the
/// read failed.
///
/// The read is what makes a retry honest: the server answers 409 to a
/// second `POST /games/{id}/participants`, so a diff against a stale frame
/// would die on the first seat the last attempt had already taken.
fn read_live_seats(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    carried: Vec<tfg::roster::LiveSeat>,
) -> Vec<tfg::roster::LiveSeat> {
    match master.game_participants(token, game_id) {
        Ok(parts) => live_seats_from(&parts),
        Err(_) => carried,
    }
}

/// The roster's mirror, from the server's own participants answer.
///
/// A free function so the rule can be tested without a session: what the
/// re-read believes about the room is the whole of the retry's honesty, and
/// a claim about the retry deserves a test that does not need a server.
fn live_seats_from(parts: &[tfg::backend::Participant]) -> Vec<tfg::roster::LiveSeat> {
    parts
        .iter()
        .map(|p| tfg::roster::LiveSeat {
            user_id: p.user_id,
            user_name: p.user_name.clone(),
            role_id: p.role_id,
            role_name: p.role_name.clone(),
            call_sign: p.call_sign.clone(),
            judge: p.judge,
        })
        .collect()
}

/// The hulls as the server holds them NOW, or the ones that crossed if the
/// read failed.
///
/// The fresh read says WHICH HULLS EXIST and WHO COMMANDS THEM; the
/// placements read says where they sit. Neither says what a hull is aboard:
/// no single read carries the embarkation links, so those ride in from the
/// mirror that crossed — the console's own last word about them.
fn read_live_hulls(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    carried: Vec<tfg::force::LiveHull>,
) -> Vec<tfg::force::LiveHull> {
    let Ok(units) = master.game_units_list(token, game_id) else {
        return carried;
    };
    let placements: Vec<(i64, f64, f64)> = master
        .placements_list(token, game_id)
        .map(|p| {
            p.placements
                .iter()
                .map(|g| (g.unit_id, g.latitude, g.longitude))
                .collect()
        })
        .unwrap_or_default();
    live_hulls_from(&units, &placements, &carried)
}

/// The force's mirror, from the server's own units and placements.
///
/// The commander and the hull's own existence come from the server, because
/// those are the two facts a re-issued write would be refused for: a second
/// `POST /games/{id}/units` answers 409, and a second assignment for a
/// commander the server already has answers "One person commands one thing".
fn live_hulls_from(
    units: &[tfg::backend::GameUnit],
    placements: &[(i64, f64, f64)],
    carried: &[tfg::force::LiveHull],
) -> Vec<tfg::force::LiveHull> {
    units
        .iter()
        .map(|u| {
            let carried_hull = carried.iter().find(|c| c.unit_id == u.unit_id);
            let start = placements
                .iter()
                .find(|(id, _, _)| *id == u.unit_id)
                .map(|(_, lat, lon)| tfg::force::Start { lat: *lat, lon: *lon });
            tfg::force::LiveHull {
                unit_id: u.unit_id,
                name: u.unit_name.clone(),
                commander_id: u.commander_id,
                start: start.or_else(|| carried_hull.and_then(|c| c.start)),
                host: carried_hull.and_then(|c| c.host),
            }
        })
        .collect()
}

/// The console's sentence for a refusal of the exercise's own terms.
///
/// The server says "Forbidden" and leaves the reason to the reader, and
/// "forbidden: Forbidden" is not an answer an operator can act on. The
/// window is the ADMINISTRATOR's ticket — `PATCH /games/{id}` is
/// `administer(ActionUpdate)` — so the console names that.
fn window_refusal(e: &tfg::backend::BackendError) -> String {
    match e {
        tfg::backend::BackendError::Forbidden { message, .. } => format!(
            "the planned window is the administrator's to write, and this seat does not \
             hold that ticket (the server said: {message}) \u{2014} the exercise's terms \
             were not written"
        ),
        other => format!("the planned window was refused: {other}"),
    }
}

/// The console's sentence for a refusal of the move itself.
///
/// The transition is the Game Master's — `POST /games/{id}/transitions` asks
/// whether the caller's ROLE may drive the exercise — so the console names
/// that rather than the server's status word.
fn transition_refusal(e: &tfg::backend::BackendError) -> String {
    match e {
        tfg::backend::BackendError::Forbidden { message, .. } => format!(
            "this seat does not run the exercise \u{2014} the Game Master drives it \
             (the server said: {message})"
        ),
        other => format!("move to preparation refused: {other}"),
    }
}

/// Write a force plan through, one write per endpoint, in `force::plan`'s
/// order.
///
/// `Err` carries the whole refusal sentence — the hull's name and the
/// server's own words — and means the plan STOPPED there: the caller decides
/// whether to post the rest, and here it never does, because posting the
/// remainder would half-commit a force the operator cannot see.
///
/// Shared by the planning advance and the ordinary per-edit sync: outside
/// planning each edit plans and posts at once, and the same order the server
/// accepts the writes in is the same order the plan emits them in.
fn flush_force(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    writes: &[tfg::force::ForceWrite],
) -> Result<(), String> {
    for w in writes {
        let step = match w {
            tfg::force::ForceWrite::Assign {
                unit_id, commander_id, ..
            } => master
                .assign_unit(token, game_id, *unit_id, *commander_id)
                .map(|_| ()),
            tfg::force::ForceWrite::HandOver {
                unit_id, commander_id, ..
            } => master
                .set_unit_commander(token, game_id, *unit_id, *commander_id)
                .map(|_| ()),
            tfg::force::ForceWrite::Remove { unit_id, .. } => {
                master.remove_unit(token, game_id, *unit_id).map(|_| ())
            }
            tfg::force::ForceWrite::Place {
                unit_id, lat, lon, ..
            } => master
                .set_placement(token, game_id, *unit_id, *lat, *lon)
                .map(|_| ()),
            tfg::force::ForceWrite::Lift { unit_id, .. } => {
                master.clear_placement(token, game_id, *unit_id).map(|_| ())
            }
            tfg::force::ForceWrite::Embark { host, child, .. } => master
                .game_attach_units(token, game_id, *host, &[*child])
                .map(|_| ()),
            tfg::force::ForceWrite::Disembark { host, child, .. } => master
                .game_detach_units(token, game_id, *host, &[*child])
                .map(|_| ()),
        };
        if let Err(e) = step {
            return Err(format!("{} refused: {e}", w.name()));
        }
    }
    Ok(())
}

/// Write a roster plan through, one write per endpoint, in `roster::plan`'s
/// order.
///
/// Same shape and same stopping rule as [`flush_force`].
fn flush_roster(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    writes: &[tfg::roster::RosterWrite],
) -> Result<(), String> {
    for w in writes {
        let step = match w {
            tfg::roster::RosterWrite::Seat {
                user_id,
                role_id,
                call_sign,
                ..
            } => master
                .add_participant(token, game_id, *user_id, *role_id, call_sign)
                .map(|_| ()),
            tfg::roster::RosterWrite::Role {
                user_id, role_id, ..
            } => master
                .set_participant_role(token, game_id, *user_id, *role_id)
                .map(|_| ()),
            tfg::roster::RosterWrite::Unseat { user_id, .. } => master
                .remove_participant(token, game_id, *user_id)
                .map(|_| ()),
        };
        if let Err(e) = step {
            return Err(format!("{} refused: {e}", w.label()));
        }
    }
    Ok(())
}

/// Write a task-organisation plan through, one write per endpoint, in
/// `groups::plan`'s order.
///
/// A node the draft is about to CREATE is created here, and the answer —
/// the whole forest — says which id the server gave it. That answer is the
/// map the later writes resolve through: a write that names a node created
/// a moment earlier is written against the server's id, not the draft's
/// negative local one.
///
/// The order is the plan's, and it is load-bearing: creates land parents
/// first (a child's create names a parent the server has to hold), then a
/// created node's commander, then the edits to held nodes, then the units
/// leaving a node, then the units placed, then the removals. A placement
/// may therefore name a node the same flush just made.
///
/// The map is returned because the BOOK's flush needs it: a step's related
/// list names task-organisation nodes, and one of them may be a node this
/// flush is creating.
fn flush_groups(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    writes: &[tfg::groups::GroupWrite],
) -> Result<std::collections::BTreeMap<i64, i64>, String> {
    // The draft's negative local id to the server's id, for the nodes this
    // flush creates.
    let mut created: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for w in writes {
        let step: Result<(), tfg::backend::BackendError> = match w {
            tfg::groups::GroupWrite::CreateNode {
                local_id,
                name,
                echelon_id,
                parent,
            } => {
                let parent_id = match parent {
                    Some(p) => Some(resolve_group(&created, p)?),
                    None => None,
                };
                match master.hierarchy_create_node(
                    token,
                    game_id,
                    *echelon_id,
                    name,
                    parent_id,
                    "",
                    "",
                ) {
                    Ok(forest) => match find_created(&forest, name, parent_id) {
                        Some(id) => {
                            created.insert(*local_id, id);
                            Ok(())
                        }
                        None => Err(tfg::backend::BackendError::Other(format!(
                            "the server created node {name:?} but did not say which one it was"
                        ))),
                    },
                    Err(e) => Err(e),
                }
            }
            tfg::groups::GroupWrite::Rename { group_id, name } => {
                master.hierarchy_update_node(token, game_id, *group_id, Some(name.as_str()))
            }
            tfg::groups::GroupWrite::DeleteNode { group_id } => {
                master.hierarchy_delete_node(token, game_id, *group_id)
            }
            tfg::groups::GroupWrite::MoveNode { group_id, parent } => {
                let parent = resolve_group(&created, parent)?;
                master.hierarchy_move_node(token, game_id, *group_id, parent)
            }
            tfg::groups::GroupWrite::ClearParent { group_id } => {
                master.hierarchy_detach_node(token, game_id, *group_id)
            }
            tfg::groups::GroupWrite::SetCommander { group, user_id } => {
                let group = resolve_group(&created, group)?;
                master.hierarchy_set_commander(token, game_id, group, *user_id)
            }
            tfg::groups::GroupWrite::ClearCommander { group_id } => {
                master.hierarchy_clear_commander(token, game_id, *group_id)
            }
            tfg::groups::GroupWrite::PlaceUnit { group, unit_id } => {
                let group = resolve_group(&created, group)?;
                master.game_unit_place(token, game_id, *unit_id, group)
            }
            tfg::groups::GroupWrite::UnplaceUnit { unit_id } => {
                master.game_unit_unplace(token, game_id, *unit_id)
            }
        };
        if let Err(e) = step {
            return Err(format!("{} refused: {e}", w.label()));
        }
    }
    Ok(created)
}

/// The server's id for a node a write names: one the server holds, or one
/// this same flush created.
fn resolve_group(
    created: &std::collections::BTreeMap<i64, i64>,
    group: &tfg::groups::GroupRef,
) -> Result<i64, String> {
    match group {
        tfg::groups::GroupRef::Live(id) => Ok(*id),
        tfg::groups::GroupRef::New(id) => created.get(id).copied().ok_or_else(|| {
            format!("node staged as {id} names a node this flush did not create")
        }),
    }
}

/// The id of the node a create just made, out of the forest the server
/// answered with.
///
/// The answer is the WHOLE forest, so the created node is found by what it
/// was asked for: its name and its parent. Two nodes that answer to that
/// are ambiguous, and an ambiguous answer is refused rather than guessed
/// at — a placement written against the wrong node is a placement nobody
/// can undo by looking at the map.
fn find_created(
    forest: &[tfg::backend::HierarchyNode],
    name: &str,
    parent: Option<i64>,
) -> Option<i64> {
    let mut found: Option<i64> = None;
    fn walk(
        nodes: &[tfg::backend::HierarchyNode],
        name: &str,
        parent: Option<i64>,
        found: &mut Option<i64>,
    ) {
        for n in nodes {
            if n.name == name && n.parent_id == parent {
                if found.is_some() {
                    // Ambiguous: a second node answers to the same question.
                    *found = None;
                    return;
                }
                *found = Some(n.id);
            }
            walk(&n.children, name, parent, found);
        }
    }
    walk(forest, name, parent, &mut found);
    found
}

/// Write a book plan through, one write per endpoint, in `book::plan`'s
/// order.
///
/// THE ORDER IS THE SERVER'S. A scenario is created before its steps,
/// because a step belongs to a scenario the server has to hold first; a
/// removed step comes after the scenario it is removed from exists; and the
/// related lists come LAST, because every row they name — the scenario, the
/// step, the hulls, the task-organisation nodes — has to exist by then.
///
/// A scenario or a step this flush creates is resolved from the answer, not
/// guessed at: the create answers with the server's id, and a step-add
/// answers with the whole scenario, so the new step is the one the answer
/// holds that was not there before. An answer that does not single one out
/// is refused rather than guessed at.
///
/// The task-organisation nodes a related list may name are resolved through
/// the map the groups flush returned, because the flush that creates a node
/// runs BEFORE this one.
fn flush_book(
    master: &tfg::backend::MinosMaster,
    token: &str,
    game_id: i64,
    writes: &[tfg::book::BookWrite],
    live: &[tfg::book::LiveScenario],
    nodes: &std::collections::BTreeMap<i64, i64>,
) -> Result<(), String> {
    // The draft's scenario local id to the server's id.
    let mut scenarios: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    // The draft's step local id to the server's id.
    let mut steps: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    // The step ids each scenario is known to hold, so a step-add's answer can
    // be diffed against them.
    let mut held: std::collections::BTreeMap<i64, Vec<i64>> = live
        .iter()
        .map(|l| (l.id, l.steps.clone()))
        .collect();
    for w in writes {
        let step: Result<(), String> = match w {
            tfg::book::BookWrite::Create { local_id, title } => {
                match master.create_game_scenario(token, game_id, title, "") {
                    Ok(sc) => {
                        scenarios.insert(*local_id, sc.id);
                        held.insert(sc.id, Vec::new());
                        Ok(())
                    }
                    Err(e) => Err(format!("create scenario {title:?} refused: {e}")),
                }
            }
            tfg::book::BookWrite::AddStep {
                scenario,
                step: draft_step,
                content,
                window,
            } => {
                let sid = resolve_scenario(&scenarios, scenario)?;
                let before = held.get(&sid).cloned().unwrap_or_default();
                let w = window.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
                match master.add_scenario_step(token, game_id, sid, content, w) {
                    Ok(sc) => {
                        let fresh: Vec<i64> = sc
                            .steps
                            .iter()
                            .map(|s| s.id)
                            .filter(|id| !before.contains(id))
                            .collect();
                        let [id] = fresh.as_slice() else {
                            return Err(format!(
                                "the server added a step to scenario {sid} but did not say which one it was"
                            ));
                        };
                        steps.insert(draft_step_local(draft_step), *id);
                        held.entry(sid).or_default().push(*id);
                        Ok(())
                    }
                    Err(e) => Err(format!("add step refused: {e}")),
                }
            }
            tfg::book::BookWrite::RemoveStep {
                scenario_id,
                step_id,
            } => master
                .delete_scenario_step(token, game_id, *scenario_id, *step_id)
                .map(|_| ())
                .map_err(|e| format!("remove step {step_id} refused: {e}")),
            tfg::book::BookWrite::SetRelated {
                scenario,
                step,
                refs,
            } => {
                let sid = resolve_scenario(&scenarios, scenario)?;
                let stid = resolve_step(&steps, step)?;
                // A ref with a NEGATIVE id names a node this plan is about to
                // create; the groups flush is what creates it, and it hands
                // the map over.
                let mut resolved: Vec<(String, i64)> = Vec::with_capacity(refs.len());
                for (kind, id) in refs {
                    let id = if *id < 0 {
                        nodes.get(id).copied().ok_or_else(|| {
                            format!("related list names node {id}, which this flush did not create")
                        })?
                    } else {
                        *id
                    };
                    resolved.push((kind.clone(), id));
                }
                master
                    .set_step_related(token, game_id, sid, stid, &resolved)
                    .map(|_| ())
                    .map_err(|e| format!("related list refused: {e}"))
            }
        };
        if let Err(why) = step {
            return Err(why);
        }
    }
    Ok(())
}

/// The server's id for a scenario a write names: one the server holds, or
/// one this same flush created.
fn resolve_scenario(
    scenarios: &std::collections::BTreeMap<i64, i64>,
    scenario: &tfg::book::ScenarioRef,
) -> Result<i64, String> {
    match scenario {
        tfg::book::ScenarioRef::Live(id) => Ok(*id),
        tfg::book::ScenarioRef::New(id) => scenarios.get(id).copied().ok_or_else(|| {
            format!("scenario staged as {id} names a scenario this flush did not create")
        }),
    }
}

/// The server's id for a step a write names: one the server holds, or one
/// this same flush added.
fn resolve_step(
    steps: &std::collections::BTreeMap<i64, i64>,
    step: &tfg::book::StepRef,
) -> Result<i64, String> {
    match step {
        tfg::book::StepRef::Live(id) => Ok(*id),
        tfg::book::StepRef::New(id) => steps.get(id).copied().ok_or_else(|| {
            format!("step staged as {id} names a step this flush did not add")
        }),
    }
}

/// The draft's own id for a step, whichever way the write names it.
fn draft_step_local(step: &tfg::book::StepRef) -> i64 {
    match step {
        tfg::book::StepRef::Live(id) => *id,
        tfg::book::StepRef::New(id) => *id,
    }
}

/// How many extra frames to ask for after the first, so MapLibre's tiles have
/// time to arrive before a capture. A stand-in for the camera.
const WARMUP_FRAMES: u64 = 24;

#[cfg(test)]
mod advance_tests {
    use super::*;

    fn participant(user_id: i64, name: &str, role_id: i64) -> tfg::backend::Participant {
        tfg::backend::Participant {
            user_id,
            user_name: name.into(),
            role_id,
            role_name: format!("role-{role_id}"),
            judge: false,
            ready: false,
            joined_at: None,
            call_sign: name.to_uppercase(),
        }
    }

    fn unit(unit_id: i64, name: &str, commander: Option<i64>) -> tfg::backend::GameUnit {
        tfg::backend::GameUnit {
            unit_id,
            unit_name: name.into(),
            hull_number: format!("h{unit_id}"),
            commander_id: commander,
            commander_name: String::new(),
            hierarchy_node: None,
        }
    }

    /// A FAILED ADVANCE LEAVES THE SERVER HOLDING PART OF THE DRAFT, and a
    /// retry that diffs against a stale frame re-issues the write the
    /// server already took — `POST /games/{id}/participants` answers 409 to
    /// a second seat, so the advance would die on the first seat it had
    /// already taken. The re-read is what makes the retry honest, and this
    /// is the claim it makes: against the server's own frame, the retry
    /// writes NOTHING for what the server already holds.
    #[test]
    fn a_re_read_after_a_failed_advance_plans_nothing_for_what_the_server_took() {
        let mut draft = tfg::roster::RosterDraft::new();
        draft.stage(tfg::roster::StagedSeat {
            user_id: 6,
            user_name: "Budi".into(),
            role_id: 1,
            role_name: "role-1".into(),
            call_sign: "BUDI".into(),
            judge: false,
        });
        // The server took the seat before it refused the rest.
        let taken = vec![participant(6, "Budi", 1)];
        let live = live_seats_from(&taken);
        assert_eq!(
            tfg::roster::plan(&draft, &live),
            Vec::new(),
            "the retry writes nothing for the seat the server already took"
        );
    }

    /// The same claim about the force: the hull the server already holds,
    /// with the commander it already has, is not assigned again — a second
    /// `POST /games/{id}/units` answers 409, and a second hull for the same
    /// commander answers "One person commands one thing".
    #[test]
    fn a_re_read_after_a_failed_advance_plans_nothing_for_the_hull_it_took() {
        let mut draft = tfg::force::ForceDraft::new();
        let mut hull = tfg::force::DraftHull::new(7, "KRI Nanggala".into(), "h7".into(), "c".into());
        hull.commander_id = Some(3);
        draft.upsert(hull);
        let taken = vec![unit(7, "KRI Nanggala", Some(3))];
        let live = live_hulls_from(&taken, &[], &[]);
        assert_eq!(
            tfg::force::plan(&draft, &live),
            Ok(Vec::new()),
            "the retry writes nothing for the hull the server already took"
        );
    }

    /// The mirror the re-read builds carries the JUDGE side, because a
    /// judge-side seat may not command and the advance has to see that
    /// before it writes the force rather than after the server refuses it.
    #[test]
    fn the_re_read_carries_the_judge_side() {
        let mut judge = participant(6, "Budi", 2);
        judge.judge = true;
        let live = live_seats_from(&[judge]);
        assert!(live[0].judge, "the judge side is the mirror's to carry");
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;

    /// A minimal EXIF block declaring one orientation: a little-endian TIFF
    /// header with a single IFD entry (`0x0112`, SHORT, count 1, the value) and
    /// no next IFD. That is all `Orientation::from_exif_chunk` reads.
    fn exif_orientation(value: u16) -> Vec<u8> {
        let mut exif = Vec::new();
        exif.extend_from_slice(b"II\x2A\x00"); // little-endian TIFF magic
        exif.extend_from_slice(&8u32.to_le_bytes()); // IFD0 at offset 8
        exif.extend_from_slice(&1u16.to_le_bytes()); // one entry
        exif.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        exif.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        exif.extend_from_slice(&1u32.to_le_bytes()); // count
        exif.extend_from_slice(&u32::from(value).to_le_bytes()); // value
        exif.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        exif
    }

    fn png_with_exif(exif: Option<Vec<u8>>) -> Vec<u8> {
        use image::ImageEncoder as _;
        // 4x2, with one bright column so a rotation is visible in the pixels.
        let img = image::RgbaImage::from_fn(4, 2, |x, _| {
            image::Rgba([if x == 3 { 255 } else { 0 }, 0, 0, 255])
        });
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        if let Some(exif) = exif {
            encoder.set_exif_metadata(exif).expect("exif chunk");
        }
        encoder
            .write_image(img.as_raw(), 4, 2, image::ExtendedColorType::Rgba8)
            .expect("encode");
        bytes
    }

    /// The picture's OWN orientation is applied on decode, and a picture that
    /// declares none is untouched.
    ///
    /// This is the difference between a hull drawn on its side and a hull
    /// drawn along its axis: `image::load_from_memory` ignores the EXIF tag by
    /// design, so a photograph stored as "rotate 90" arrives sideways and every
    /// consumer — map, row, ghost — inherits the error.
    #[test]
    fn a_pictures_own_orientation_is_applied_on_decode() {
        let plain = decode_picture(&png_with_exif(None)).expect("a PNG with no tag decodes");
        assert_eq!((plain.width(), plain.height()), (4, 2), "no tag, no turn");

        let turned =
            decode_picture(&png_with_exif(Some(exif_orientation(6)))).expect("a tagged PNG decodes");
        assert_eq!(
            (turned.width(), turned.height()),
            (2, 4),
            "orientation 6 is one quarter turn"
        );

        // A tag that says nothing is also a no-op, so the default path is the
        // one every PNG in the register already takes.
        let untagged = decode_picture(&png_with_exif(Some(exif_orientation(1))))
            .expect("orientation 1 decodes");
        assert_eq!((untagged.width(), untagged.height()), (4, 2));
    }

    /// Bytes that are not an image decode to nothing rather than panicking: a
    /// truncated download paints no hull, never a torn one.
    #[test]
    fn a_corrupt_picture_decodes_to_nothing() {
        assert!(decode_picture(b"not an image").is_none());
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    /// A jump puts the hull on the VISIBLE centre, not merely on the frame's.
    ///
    /// The shell's shift says how far the content moves to clear the floating
    /// zone (positive = right for a left dock), so this is the one assertion
    /// that can catch the sign: project the hull back through the goal and it
    /// must land `shift` pixels to the right of the frame centre — the half of
    /// the frame the zone is not covering.
    #[test]
    fn a_jump_puts_the_hull_on_the_visible_centre() {
        let (lat, lon) = (-6.108, 106.9237);
        let (w, h) = (800.0, 600.0);
        let zoom = 14.0;
        for shift in [180.0f32, -180.0, 0.0] {
            let goal = jump_goal(lat, lon, shift, zoom, w, h);
            let (x, y) = tfg::map_render::project_mercator(lat, lon, goal, zoom, w, h);
            assert!(
                (x - (w / 2.0 + f64::from(shift))).abs() < 1e-6,
                "shift {shift}: the hull projects to {x}, wanted {}",
                w / 2.0 + f64::from(shift)
            );
            assert!((y - h / 2.0).abs() < 1e-6, "shift {shift}: vertical centre");
        }
    }
}

#[cfg(test)]
mod fleet_taxonomy_tests {
    use super::*;

    fn seed_tree(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO helpers (table_name, id, name, id_name) VALUES
              ('asset_groups', 2, 'Sea', 'Aset Laut'),
              ('asset_groups', 4, 'Other', 'Aset Lainnya')",
            [],
        )
        .expect("groups");
        conn.execute(
            "INSERT INTO unit_categories (id, name, id_name, type_count, id_asset_group)
              VALUES (2, 'Frigate', 'Frigat', 1, 2)",
            [],
        )
        .expect("category");
        conn.execute(
            "INSERT INTO unit_types (id, name, id_name, category_id, class_count)
              VALUES (3, 'FFG', 'Frigat', 2, 1)",
            [],
        )
        .expect("type");
        conn.execute(
            "INSERT INTO unit_classes (id, name, id_name, type_id, hull_count)
              VALUES (5, 'Sigma', 'Sigma', 3, 2)",
            [],
        )
        .expect("class");
        conn.execute(
            "INSERT INTO units (id, name, hull_number, class_id, branch_id)
              VALUES (13, 'KRI Ahmad Yani', '351', 5, 1),
                     (14, 'KRI Yos Sudarso', '353', 5, 1)",
            [],
        )
        .expect("hulls");
    }

    #[test]
    fn builder_walks_group_to_hull() {
        let conn =
            tfg::store::open(&std::path::PathBuf::from(":memory:")).expect("open");
        seed_tree(&conn);
        let tax =
            build_fleet_taxonomy(&conn, |_| tfg::view::MapSymbolShape::Frigate);
        assert_eq!(tax.groups.len(), 2, "both groups list, even empty ones");
        let sea = tax.groups.iter().find(|g| g.id == 2).expect("sea tab");
        assert_eq!(sea.cats.len(), 1);
        assert_eq!(sea.cats[0].types.len(), 1);
        assert_eq!(sea.cats[0].types[0].classes.len(), 1);
        assert_eq!(sea.cats[0].types[0].classes[0].units.len(), 2);
        let leaf = &sea.cats[0].types[0].classes[0].units[0];
        assert_eq!(leaf.name, "KRI Ahmad Yani");
        assert_eq!(leaf.hull, "351");
        assert_eq!(leaf.symbol, tfg::view::MapSymbolShape::Frigate);
        let other = tax.groups.iter().find(|g| g.id == 4).expect("other tab");
        assert!(other.cats.is_empty(), "nothing filed under Other");
    }

    #[test]
    fn builder_empty_store_is_empty_tree() {
        let conn =
            tfg::store::open(&std::path::PathBuf::from(":memory:")).expect("open");
        let tax =
            build_fleet_taxonomy(&conn, |_| tfg::view::MapSymbolShape::Dot);
        assert!(tax.groups.is_empty(), "the picker falls back to flat");
    }
}
