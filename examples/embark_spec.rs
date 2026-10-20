//! The embark spec, from the LIVE server: what a host declares it carries,
//! and who is in it right now — and what the console's rule would say about
//! a child dropped aboard it.
//!
//! The console's embark drop is decided against exactly this — the rule in
//! `tfg::embark` reads the complements, the passengers and the register's
//! read of each hull's kind — so the read is worth proving against the real
//! thing rather than a fixture. The probe logs in with the same credentials
//! the shell uses, reads one host's spec, applies the rule to a child, and
//! prints both, so a wrong route or a changed shape shows up as words on
//! the terminal.
//!
//! Run:
//!
//! ```text
//! TFG_API_USER=superuser TFG_API_PASS=… \
//!   cargo run --example embark_spec -- <game> <host> [child]
//! ```
//!
//! The child is optional: it names the hull the operator is about to drop
//! aboard, and the probe answers what the rule would say about it. The rule
//! needs the register's read of each hull's KIND, which lives in the LOCAL
//! mirror the shell syncs at startup — so the probe opens the same mirror.

use std::collections::BTreeMap;

use tfg::backend::{MinosAuth, MinosMaster};
use tfg::embark::HullTaxonomy;

fn main() {
    let host = std::env::var("TFG_MINOS_HOST")
        .unwrap_or_else(|_| "https://api.tfg.development.crossnet.co.id/api/v1".to_string());
    let user = std::env::var("TFG_API_USER").unwrap_or_default();
    let pass = std::env::var("TFG_API_PASS").unwrap_or_default();
    if user.is_empty() || pass.is_empty() {
        eprintln!("TFG_API_USER and TFG_API_PASS are required");
        std::process::exit(2);
    }
    let mut args = std::env::args().skip(1);
    let game: i64 = args
        .next()
        .and_then(|a| a.parse().ok())
        .unwrap_or_else(|| {
            eprintln!("usage: embark_spec <game> <host> [child]");
            std::process::exit(2);
        });
    let unit: i64 = match args.next().and_then(|a| a.parse().ok()) {
        Some(u) => u,
        None => {
            eprintln!("usage: embark_spec <game> <host> [child]");
            std::process::exit(2);
        }
    };
    let child: Option<i64> = args.next().and_then(|a| a.parse().ok());

    let Ok(auth) = MinosAuth::new(&host) else {
        eprintln!("cannot reach {host}");
        std::process::exit(1);
    };
    let Ok(pair) = auth.login(&user, &pass) else {
        eprintln!("login refused");
        std::process::exit(1);
    };
    let Ok(master) = MinosMaster::new(&host) else {
        eprintln!("cannot reach {host}");
        std::process::exit(1);
    };
    let token = pair.access_token;

    // The exercise's own pieces, so the probe can name a host that exists
    // in it — and show what the shell would drop onto.
    if let Ok(pieces) = master.game_units_list(&token, game) {
        println!("game {game} holds {} piece(s):", pieces.len());
        for p in &pieces {
            println!("    {} — {}", p.unit_id, p.unit_name);
        }
    }

    // The two reads the embark rule needs, and only those two.
    let caps = match master.unit_capacities(&token, unit) {
        Ok(caps) => caps,
        Err(e) => {
            eprintln!("capacities failed: {e}");
            std::process::exit(1);
        }
    };
    let aboard = match master.game_embarkations(&token, game, unit) {
        Ok(aboard) => aboard,
        Err(e) => {
            // A host the exercise does not hold answers 404 here, and that
            // is the truth: it holds nobody.
            eprintln!("embarkations failed: {e}");
            Vec::new()
        }
    };

    println!("host {unit} of game {game}");
    println!("  complements: {}", caps.len());
    for c in &caps {
        println!(
            "    {} {} — holds at most {}",
            c.level, c.id_level, c.max_aboard
        );
    }
    println!("  aboard: {}", aboard.len());
    for u in &aboard {
        println!(
            "    {} (hull {}, class {}){}",
            u.name,
            u.hull_number,
            u.class_name,
            if u.is_cleared { " — cleared" } else { "" }
        );
    }
    if caps.is_empty() {
        println!("  RULE: this host carries nothing, so nothing may go aboard it");
    }
    // The rule's own answer about a child, against the same spec: the
    // register's read of each hull's KIND comes from the LOCAL mirror the
    // shell syncs at startup.
    if let Some(child) = child {
        let taxonomy = local_taxonomy();
        match tfg::embark::check(&caps, &aboard, &taxonomy, unit, child) {
            Ok(()) => println!("  RULE: {child} may embark aboard {unit}"),
            Err(why) => println!("  RULE: {child} may NOT embark aboard {unit} — {}", why.reason()),
        }
    }
    // The REGISTER's own answer to "what could go inside this hull" — the
    // server's own version of the same question the rule asks.
    match master.embarkation_candidates(&token, unit) {
        Ok(cands) => {
            println!("  register candidates: {}", cands.len());
            for c in cands.iter().take(8) {
                println!("    {} ({})", c.name, c.class_name);
            }
        }
        Err(e) => println!("  register candidates failed: {e}"),
    }
    // The hull's PICTURE — the embarkation boxes on it. These are the server's
    // hard ceiling for an attach into an EXERCISE, where the complements
    // above are only a warning, so the two are worth reading side by side.
    match reqwest::blocking::Client::new()
        .get(format!("{host}/games/{game}/units/{unit}/slots"))
        .bearer_auth(&token)
        .send()
    {
        Ok(resp) => {
            let status = resp.status();
            let body: serde_json::Value = resp.json().unwrap_or(serde_json::Value::Null);
            let slots = body
                .get("slots")
                .and_then(|s| s.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!("  picture boxes: {slots} (HTTP {status})");
            if let Some(list) = body.get("slots").and_then(|s| s.as_array()) {
                for s in list {
                    println!(
                        "    box {} — {} — asset group {:?}",
                        s.get("id").and_then(|v| v.as_u64()).unwrap_or(0),
                        s.get("kind").and_then(|v| v.as_str()).unwrap_or("?"),
                        s.get("id_asset_group").and_then(|v| v.as_u64())
                    );
                }
            }
        }
        Err(e) => println!("  picture boxes failed: {e}"),
    }
}

/// The register's read of every hull's KIND, from the local mirror the shell
/// syncs at startup. Empty when the mirror is missing or empty, and the rule
/// then leaves the hull to the server — which is the same reading the shell
/// makes.
fn local_taxonomy() -> BTreeMap<i64, HullTaxonomy> {
    let paths = tfg::paths::AppPaths::discover().ok();
    let Some(paths) = paths else {
        return BTreeMap::new();
    };
    let Ok(conn) = tfg::store::open(&paths.local_db) else {
        return BTreeMap::new();
    };
    tfg::store::fleet_units(&conn)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|u| {
            u.id.parse::<i64>().ok().map(|id| {
                (
                    id,
                    HullTaxonomy {
                        class_id: u.class_id,
                        type_id: u.type_id,
                        category_id: u.category_id,
                    },
                )
            })
        })
        .collect()
}
