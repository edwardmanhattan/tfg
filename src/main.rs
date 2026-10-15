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

use std::sync::Arc;
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
    // the shipped crate carries no secret, and the console's real path is
    // the login card plus the OS keyring, which is where this goes next.
    let host = std::env::var("TFG_MINOS_HOST")
        .unwrap_or_else(|_| "https://api.tfg.development.crossnet.co.id/api/v1".to_string());
    let (roster, roster_total, token) = match (
        std::env::var("TFG_API_USER"),
        std::env::var("TFG_API_PASS"),
    ) {
        (Ok(user), Ok(pass)) => match fetch_roster(&host, &user, &pass) {
            Some((rows, token)) => {
                let total = rows.len();
                eprintln!("roster: {total} hulls from {host}");
                (rows, total, token)
            }
            None => {
                eprintln!("roster: fetch failed, falling back to the fixture");
                (tfg::shell_gpui::fixture_roster(), 125, String::new())
            }
        },
        _ => {
            eprintln!("roster: no credentials (TFG_API_USER/TFG_API_PASS), fixture rows");
            (tfg::shell_gpui::fixture_roster(), 125, String::new())
        }
    };

    // `TFG_SIGNED_OUT` forces the sign-in card for a headless capture. The
    // shipped build starts here anyway — the card is otherwise only reachable
    // by having no credentials at all — and this is the seed that makes the
    // first panel photographable without unsetting the environment.
    let (roster, roster_total, token) = if std::env::var("TFG_SIGNED_OUT").as_deref() == Ok("1") {
        (tfg::shell_gpui::fixture_roster(), 125, String::new())
    } else {
        (roster, roster_total, token)
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
    // Swap the boot wire for the live one immediately: the same `WireKind`
    // the egui console sends, over the same socket URL the kernel derives.
    // No token (no credentials) means no live wire — the empty boot wire
    // stands, and staleness stays seeded, honestly.
    if !token.is_empty() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let _ = wire_ctl_tx.send(tfg::shell_threads::WireKind::Live {
            ws_url: tfg::backend::websocket_url(&host),
            rest_base: host.clone(),
            token: token.clone(),
            cmd_tx,
            cmd_rx,
            event_tx,
            wake_tx: wire_wake_tx.clone(),
        });
        eprintln!("wire: live {}", tfg::backend::websocket_url(&host));
    }

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
    let (create_tx, create_rx) = mpsc::channel::<Result<tfg::shell_gpui::CreatedSession, String>>();
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
    {
        let (host2, token2, game_id) = (host.clone(), token.clone(), session.game_id);
        let camera2 = camera.clone();
        let req_tx2 = req_tx.clone();
        std::thread::spawn(move || {
            // Mutated by the sign-in/sign-out answers below: a card sign-in
            // replaces the startup token (empty when the shell opened on the
            // card), and a sign-out clears it so nothing else posts on it.
            let mut token2 = token2;
            let mut game_id = game_id;
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
                            Ok(fix) => tfg::shell_gpui::HelmStatus {
                                unit_id: order.unit_id.clone(),
                                ok: true,
                                text: format!(
                                    "applied {:.0}\u{00b0} / {:.1} kn{}",
                                    fix.heading,
                                    fix.speed,
                                    if fix.clamped { " (clamped)" } else { "" }
                                ),
                            },
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
                            tfg::map_render::JUMP_PUMP,
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
                        let goal = tfg::map_render::unproject_mercator(
                            mw / 2.0 + f64::from(shift_px),
                            mh / 2.0,
                            (lat, lon),
                            zoom,
                            mw,
                            mh,
                        );
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
                            tfg::map_render::JUMP_PUMP,
                        ));
                    }
                    tfg::shell_gpui::ToBinary::SignIn { identifier, password } => {
                        // The whole authenticated read, TOGETHER: the register
                        // and the session come from one sign-in, so the shell
                        // never sees half a session. The refusal is a sentence
                        // the card can show — never a swallowed error.
                        let outcome = match fetch_roster(&host2, &identifier, &password) {
                            Some((rows, token)) => {
                                let total = rows.len();
                                let pinned = std::env::var("TFG_GAME")
                                    .ok()
                                    .and_then(|v| v.parse::<i64>().ok());
                                match fetch_session(&host2, &token, pinned) {
                                    Some(mut s) => {
                                        s.signed_in = true;
                                        s.zoom = tfg::map_render::MAP_ZOOM;
                                        // The shell gets the session; THIS
                                        // side keeps the token and the game,
                                        // so a helm order after a card
                                        // sign-in posts as the new session.
                                        // Before this, the startup token
                                        // (empty when the shell opened on the
                                        // card) stayed, and the first order
                                        // after signing in was refused.
                                        token2 = token;
                                        game_id = s.game_id;
                                        Ok(tfg::shell_gpui::SignedIn {
                                            roster: rows,
                                            roster_total: total,
                                            session: s,
                                        })
                                    }
                                    None => Err(
                                        "signed in, but the session could not be read".to_string(),
                                    ),
                                }
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
                        // this side never held the refresh token — and the
                        // live wire keeps its socket, honestly idle.
                        token2.clear();
                        eprintln!("signed out — token cleared");
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
                                            Ok(tfg::shell_gpui::CreatedSession { session: s })
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "added, but the book could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("add failed: {e}")),
                            }
                        };
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
                                                Ok(tfg::shell_gpui::CreatedSession { session: s })
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
                                            Ok(tfg::shell_gpui::CreatedSession { session: s })
                                        }
                                        None => Err(
                                            "removed, but the book could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("remove failed: {e}")),
                            }
                        };
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "declared, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("readiness failed: {e}")),
                            }
                        };
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
                            let mut refusal: Option<String> = None;
                            for w in &writes {
                                let step = match w {
                                    tfg::roster::RosterWrite::Seat {
                                        user_id, role_id, call_sign, ..
                                    } => master
                                        .add_participant(
                                            &token2,
                                            game_id,
                                            *user_id,
                                            *role_id,
                                            call_sign,
                                        )
                                        .map(|_| ()),
                                    tfg::roster::RosterWrite::Role { user_id, role_id, .. } => {
                                        master
                                            .set_participant_role(&token2, game_id, *user_id, *role_id)
                                            .map(|_| ())
                                    }
                                    tfg::roster::RosterWrite::Unseat { user_id, .. } => master
                                        .remove_participant(&token2, game_id, *user_id)
                                        .map(|_| ()),
                                };
                                if let Err(e) = step {
                                    refusal = Some(format!("{} refused: {e}", w.label()));
                                    break;
                                }
                            }
                            match refusal {
                                None => {
                                    eprintln!("roster synced: {} write(s)", writes.len());
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "the room was written, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Some(why) => Err(why),
                            }
                        };
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
                        // A `Place` or `Lift` here is a bug, not a refusal —
                        // neither side of this shell's draft carries a starting
                        // position (there is no map drop), so `plan` cannot
                        // emit one. It is reported loudly rather than swallowed,
                        // because a draft that grew a start without the panel
                        // that sets it would otherwise write a coordinate
                        // nobody chose.
                        let outcome = if token2.is_empty() {
                            Err("signed out \u{2014} sign in to pick the fleet".to_string())
                        } else {
                            let Ok(master) = tfg::backend::MinosMaster::new(&host2) else {
                                return;
                            };
                            let mut refusal: Option<String> = None;
                            for w in &writes {
                                let step = match w {
                                    tfg::force::ForceWrite::Assign {
                                        unit_id, commander_id, ..
                                    } => master
                                        .assign_unit(&token2, game_id, *unit_id, *commander_id)
                                        .map(|_| ()),
                                    tfg::force::ForceWrite::HandOver {
                                        unit_id, commander_id, ..
                                    } => master
                                        .set_unit_commander(&token2, game_id, *unit_id, *commander_id)
                                        .map(|_| ()),
                                    tfg::force::ForceWrite::Remove { unit_id, .. } => {
                                        master.remove_unit(&token2, game_id, *unit_id).map(|_| ())
                                    }
                                    tfg::force::ForceWrite::Place { name, .. }
                                    | tfg::force::ForceWrite::Lift { name, .. } => Err(
                                        tfg::backend::BackendError::Other(format!(
                                            "{name} has a position this shell cannot write"
                                        )),
                                    ),
                                };
                                if let Err(e) = step {
                                    refusal = Some(format!("{} refused: {e}", w.name()));
                                    break;
                                }
                            }
                            match refusal {
                                None => {
                                    eprintln!("force synced: {} write(s)", writes.len());
                                    match fetch_session(&host2, &token2, Some(game_id)) {
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "the force was written, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Some(why) => Err(why),
                            }
                        };
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "advanced, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("advance refused: {e}")),
                            }
                        };
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "ended, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("end refused: {e}")),
                            }
                        };
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "saved, but the book could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("save failed: {e}")),
                            }
                        };
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "declared, but the session could not be read"
                                                .to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("step declaration failed: {e}")),
                            }
                        };
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
                                        Some(s) => Ok(tfg::shell_gpui::CreatedSession { session: s }),
                                        None => Err(
                                            "sent, but the thread could not be read".to_string(),
                                        ),
                                    }
                                }
                                Err(e) => Err(format!("send failed: {e}")),
                            }
                        };
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
        order_tx,
        roster,
        roster_total,
        session,
    );
}

/// The hull register over the dev API, as the console's own client reads it.
///
/// Returns the rows and the access token together: the token opens the live
/// wire too, so logging in once serves both the register and the feed.
///
/// `MinosMaster::units_named` rather than `units`: the sheet wants a class
/// NAME, and `units` flattens the taxonomy to ids by design (the console
/// resolves them from the catalog after a sync).
fn fetch_roster(host: &str, user: &str, pass: &str) -> Option<(Vec<tfg::shell_gpui::RosterRow>, String)> {
    let auth = tfg::backend::MinosAuth::new(host).ok()?;
    let pair = auth.login(user, pass).ok()?;
    let master = tfg::backend::MinosMaster::new(host).ok()?;
    let units = master.units_named(&pair.access_token).ok()?;
    let rows = units
        .into_iter()
        .map(|u| tfg::shell_gpui::RosterRow {
            unit_id: u.id.to_string(),
            name: u.name,
            hull: u.hull_number,
            class: format!("{} · {}", u.class_name, u.category_name),
            stale: false,
        })
        .collect();
    Some((rows, pair.access_token))
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

/// The held session: the game this account is SEATED in, and its room.
///
/// Read-only on purpose. The console JOINS (a room key, a write) and reads the
/// seat from the answer; the prototype reads `/users/me` and looks for its own
/// row in each game's roster instead — no write, and the same seat. A game
/// whose participants are unreadable (403) is skipped, because a room this
/// account cannot read is not a room it holds.
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
    // Which of them sits on the map. A separate read because it is a
    // separate fact: a hull is in the exercise whether or not it has been
    // placed, and the console's draft carries the same two fields side by
    // side. Best-effort — an unreadable placements list leaves every hull
    // unplaced rather than failing the whole session read.
    let placed_ids: Vec<i64> = master
        .placements_list(token, game_id)
        .map(|p| p.placements.iter().map(|g| g.unit_id).collect())
        .unwrap_or_default();
    let pieces = units
        .iter()
        .map(|u| tfg::shell_gpui::PieceView {
            id: u.unit_id,
            label: format!("{} ({})", u.unit_name, u.hull_number),
            commander_id: u.commander_id,
            placed: placed_ids.contains(&u.unit_id),
        })
        .collect();
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
        related,
        facts,
        readiness,
        directory,
        roles,
    })
}

/// How many extra frames to ask for after the first, so MapLibre's tiles have
/// time to arrive before a capture. A stand-in for the camera.
const WARMUP_FRAMES: u64 = 24;
