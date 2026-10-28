//! The frames' deterministic states — `TFG_FLOW`, without a write.
//!
//! A frame is a picture of a state, and the hosted API cannot be made to hold
//! "9 placed, 3 unplaced, 4 groups, 5 in them". So each frame needs a FIXTURE:
//! the draft, the history and the reads staged from this module, and nothing
//! sent to the server. A fixture capture is a picture of the fixture — the right
//! evidence for layout and for a gesture's intermediate state, and not evidence
//! that a write works (`docs/creation-flow-handoff.md` §16).
//!
//! ONE FAMILY, `TFG_FLOW=<state>`, and every value is a frame's own label
//! (`f1a` … `f8`). [`Frame::of`] parses it; the shell asks for the frame and
//! this module builds its state.
//!
//! WHAT IS BUILT TODAY: the frames of the slices that exist — the landing
//! (`f1a`, `f1b`) and stage 01 (`f2a`, `f2b`). A frame whose slice has not
//! landed answers `None` rather than half a state, because a fixture that
//! invented the rest of a frame would be a picture of this module rather than
//! of the flow.
//!
//! THE NUMBERS ARE THE FRAMES'. `GAME SESSIONS · 08` with four state tabs whose
//! counts sum to eight, nine steps on a Saved Scenario, five fields filled on
//! F2a — the examples the sheet draws, made deterministic. A fixture that
//! quietly rendered 08 as 4 would make the capture disagree with the frame for
//! no reason anybody could find later.

use crate::backend::{GameRow, MissionStep, ScenarioMission};
use crate::draft::{Draft, Terms, TimeBase};
use crate::gamestate::GameMode;
use crate::landing::{Landing, StartFrom};

/// Every frame on the board, in the sheet's order.
///
/// The enum is the FAMILY, not a promise: [`fixture`] answers for the frames
/// whose slice has landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    F1a,
    F1b,
    F2a,
    F2b,
    F3a,
    F3b,
    F3c,
    F3d,
    F3e,
    F4a,
    F4b,
    F4c,
    F5a,
    F5b,
    F6a,
    F6b,
    F7,
    F8,
}

impl Frame {
    /// The `TFG_FLOW` value for this frame.
    pub fn label(self) -> &'static str {
        match self {
            Frame::F1a => "f1a",
            Frame::F1b => "f1b",
            Frame::F2a => "f2a",
            Frame::F2b => "f2b",
            Frame::F3a => "f3a",
            Frame::F3b => "f3b",
            Frame::F3c => "f3c",
            Frame::F3d => "f3d",
            Frame::F3e => "f3e",
            Frame::F4a => "f4a",
            Frame::F4b => "f4b",
            Frame::F4c => "f4c",
            Frame::F5a => "f5a",
            Frame::F5b => "f5b",
            Frame::F6a => "f6a",
            Frame::F6b => "f6b",
            Frame::F7 => "f7",
            Frame::F8 => "f8",
        }
    }

    /// Parse a `TFG_FLOW` value. Case-insensitive, trimmed — an environment
    /// variable is typed by a person.
    pub fn of(name: &str) -> Option<Frame> {
        let wanted = name.trim().to_lowercase();
        [
            Frame::F1a,
            Frame::F1b,
            Frame::F2a,
            Frame::F2b,
            Frame::F3a,
            Frame::F3b,
            Frame::F3c,
            Frame::F3d,
            Frame::F3e,
            Frame::F4a,
            Frame::F4b,
            Frame::F4c,
            Frame::F5a,
            Frame::F5b,
            Frame::F6a,
            Frame::F6b,
            Frame::F7,
            Frame::F8,
        ]
        .into_iter()
        .find(|f| f.label() == wanted)
    }

    /// Whether this frame's slice has landed, so a capture of it is a picture
    /// of something that exists.
    pub fn built(self) -> bool {
        matches!(
            self,
            Frame::F1a
                | Frame::F1b
                | Frame::F2a
                | Frame::F2b
                | Frame::F3a
                | Frame::F3b
                | Frame::F3c
                | Frame::F3d
                | Frame::F3e
                | Frame::F4a
                | Frame::F4b
                | Frame::F4c
                | Frame::F5a
                | Frame::F5b
                | Frame::F6a
                | Frame::F6b
                | Frame::F7
                | Frame::F8
        )
    }
}

/// The history's rows: eight games, as `GAME SESSIONS · 08` states.
///
/// The four named ones are the sheet's own rows; the other four exist so the
/// counts sum to the total, which is the point erratum 14 makes about the
/// frame — four tabs could not count eight games that include a `preparation`.
pub fn games() -> Vec<GameRow> {
    let row = |id: i64, name: &str, mode: &str, state: &str, at: &str| GameRow {
        id,
        name: name.to_string(),
        mode: mode.to_string(),
        state: state.to_string(),
        created_at: at.to_string(),
        updated_at: at.to_string(),
    };
    vec![
        row(
            1,
            "Baltic Shield 26",
            "maneuver",
            "planning",
            "2026-10-23T09:41:00+07:00",
        ),
        row(
            2,
            "North Sea Patrol",
            "scenario",
            "preparation",
            "2026-10-22T08:10:00+07:00",
        ),
        row(
            3,
            "Harbour Defence",
            "static",
            "execution",
            "2026-10-21T07:30:00+07:00",
        ),
        row(
            4,
            "Harbour Sweep",
            "scenario",
            "closure",
            "2026-10-20T15:00:00+07:00",
        ),
        row(
            5,
            "Java Sea Watch",
            "maneuver",
            "planning",
            "2026-10-19T11:20:00+07:00",
        ),
        row(
            6,
            "Sunda Strait",
            "scenario",
            "preparation",
            "2026-10-18T09:00:00+07:00",
        ),
        row(
            7,
            "Malacca Run",
            "maneuver",
            "execution",
            "2026-10-17T13:45:00+07:00",
        ),
        row(
            8,
            "Lombok Passage",
            "static",
            "closure",
            "2026-10-16T10:05:00+07:00",
        ),
    ]
}

/// The Saved Scenarios the START FROM list draws: three, with the sheet's step
/// counts and dates.
pub fn missions() -> Vec<ScenarioMission> {
    let mission = |id: i64, title: &str, code: &str, steps: usize, at: &str| ScenarioMission {
        id,
        code: code.to_string(),
        title: title.to_string(),
        description: String::new(),
        play_count: 0,
        updated_at: Some(at.to_string()),
        steps: (0..steps)
            .map(|at| MissionStep {
                id: (id * 100) + at as i64,
                position: at as i64,
                kind: "orders".to_string(),
                content: format!("step {}", at + 1),
                start_hour: None,
                end_hour: None,
                updated_at: None,
                substeps: Vec::new(),
            })
            .collect(),
    };
    vec![
        mission(
            11,
            "Baltic Shield 25",
            "Surface",
            9,
            "2026-10-18T09:00:00+07:00",
        ),
        mission(
            12,
            "North Sea Patrol",
            "Escort",
            14,
            "2026-10-15T09:00:00+07:00",
        ),
        mission(
            13,
            "Harbour Sweep",
            "Littoral",
            6,
            "2026-10-11T09:00:00+07:00",
        ),
    ]
}

/// F1a: the history read, nothing chosen.
///
/// `PROCEED` is disabled and says so, which is the frame's own note.
pub fn landing_nothing_chosen() -> Landing {
    let mut landing = Landing::new();
    landing.set_games(games());
    landing.set_missions(missions());
    landing
}

/// F1b: `SCENARIO` chosen, so `START FROM` is revealed and nothing in it is
/// chosen yet — `BLANK SCENARIO` is a row, not a default.
pub fn landing_scenario_revealed() -> Landing {
    let mut landing = landing_nothing_chosen();
    landing.choose_mode(GameMode::Scenario);
    debug_assert_eq!(landing.start_from(), StartFrom::Unchosen);
    landing
}

/// A stage-01 draft, filled as the frame draws it.
///
/// F2a is the TOP of the form: five fields filled, the time base below the fold
/// and empty. F2b is scrolled to the time base: the same five fields, plus a
/// complete base from which both ends are derived.
///
/// The scroll counter (`3 OF 10`, `10 OF 10`) is a SCROLL POSITION and not a
/// fixture fact — it belongs to the viewport, so the shell sets it and this
/// module supplies the field values it counts past.
pub fn stage_one(frame: Frame) -> Option<Draft> {
    stage_one_in(frame, GameMode::Scenario)
}

/// The same form in another mode. A **scenario** draft is the only one that may
/// hold a book, so F7's five steps need this door.
fn stage_one_in(frame: Frame, mode: GameMode) -> Option<Draft> {
    let mut draft = Draft::blank(mode)?;
    draft.origin = crate::draft::Origin::Blank { mode };
    draft.pace = crate::backend::GamePace::Standard;
    draft.terms = Terms {
        name: "Baltic Shield 26".to_string(),
        description: "Two-day manoeuvre exercise in the Java Sea.".to_string(),
        purpose: "Train the staff in a contested transit.".to_string(),
        target: "The exercise area west of Surabaya.".to_string(),
        area: "Java Sea, west of Surabaya".to_string(),
        map_tag: String::new(),
        time: match frame {
            // The base the operator has not reached yet: the form may hold a
            // half-set base, and an empty one is legal while planning.
            Frame::F2a => TimeBase::default(),
            _ => TimeBase {
                actual_date: "2026-11-01".to_string(),
                actual_start: "0800".to_string(),
                assumed_start: "0600".to_string(),
                factor: "2".to_string(),
                length: "1.5".to_string(),
            },
        },
    };
    Some(draft)
}

/// F7's draft: one that can be written.
///
/// THE FORCE DECIDES THE SEATS, and that is the server's rule rather than this
/// fixture's convenience: `POST /games/{id}/units` needs a commander, that
/// commander must be a participant, and one person commands ONE thing
/// (`crate::force::gap`). So nine hulls need nine commandos, and the referee
/// sits on the judge side where a command is refused by design.
///
/// Five steps in one book and one task group holding three of the hulls, so the
/// ladder has positions with more than one write behind them and stage 03's
/// counts are not zero.
pub fn review_draft() -> Option<Draft> {
    // A SCENARIO DRAFT, because the five steps below need a book and only a
    // scenario exercise may hold one (`crate::commit::gaps`).
    let mut draft = stage_one_in(Frame::F2b, GameMode::Scenario)?;
    let seats = [
        (1, "Cdr. Anwar", "Game Master", false),
        (2, "Lt. Fitri", "Commando", false),
        (3, "Rina Wijaya", "Commando", false),
        (4, "Lt. Bayu", "Commando", false),
        (5, "Lt. Sari", "Commando", false),
        (6, "Lt. Dedi", "Commando", false),
        (7, "Lt. Wulan", "Commando", false),
        (8, "Lt. Joko", "Commando", false),
        (9, "Lt. Nia", "Commando", false),
        (10, "Lt. Adi", "Commando", false),
        // The referee is on the JUDGE SIDE, where a command is refused by
        // design — so the id is not one of the nine the hulls name.
        (11, "Kpt. Sari", "Referee", true),
    ];
    for (user_id, name, role, judge) in seats {
        draft.roster.stage(crate::roster::StagedSeat {
            user_id,
            user_name: name.to_string(),
            role_id: 1,
            role_name: role.to_string(),
            call_sign: name.to_uppercase().replace(' ', "-"),
            judge,
        });
    }
    // Nine hulls, each placed and each commanded by a different commando.
    let hulls = [
        (351, "KRI Ahmad Yani", -3.21, 108.91),
        (352, "KRI Slamet Riyadi", -3.24, 108.95),
        (353, "KRI Yos Sudarso", -3.28, 108.99),
        (354, "KRI Oswald Siahaan", -3.31, 109.04),
        (367, "KRI Iskandar Muda", -3.35, 109.08),
        (371, "KRI Hang Tuah", -3.38, 109.12),
        (365, "KRI Diponegoro", -3.42, 109.16),
        (361, "KRI Sigmas 01", -3.45, 109.20),
        (524, "KRI Bintuni", -3.49, 109.24),
    ];
    for (at, (unit_id, name, lat, lon)) in hulls.iter().enumerate() {
        let mut hull = crate::force::DraftHull::new(
            *unit_id,
            name.to_string(),
            format!("{unit_id}"),
            "frigate".to_string(),
        );
        hull.start = Some(crate::force::Start {
            lat: *lat,
            lon: *lon,
        });
        hull.commander_id = Some(at as i64 + 2);
        draft.force.upsert(hull);
    }
    draft.ceiling = Some(40);
    draft
        .groups
        .stage_group(
            0,
            "GUS-1",
            crate::groups::GroupKind::SatuanTugas,
            None,
            Some(2),
            vec![351, 352, 353],
        )
        .ok()?;
    let book = draft
        .book
        .stage_scenario_named("Harbour sweep", "the sweep north");
    for (at, (content, start, end)) in [
        ("ORDERS brief", "0800", "0820"),
        ("TRANSIT north", "0820", "0910"),
        ("SWEEP the strait", "0930", "1015"),
        ("RECOVER", "1100", "1130"),
        ("DEBRIEF", "1130", "1200"),
    ]
    .iter()
    .enumerate()
    {
        let _ = at;
        draft
            .book
            .stage_step_new(book, content, Some((start.to_string(), end.to_string())));
    }
    Some(draft)
}

/// THE REGISTER, with the sheet's own shape: four groups (one empty, because
/// the endpoint returns it), ten categories, twenty-six types, forty-one
/// classes and a hundred and twenty-five hulls.
///
/// GENERATED, not typed out: a hundred and twenty-five hand-written rows would
/// be a hundred and twenty-five chances to disagree with the counts in the
/// band. The generator is arithmetic over three short tables, so the numbers
/// the frame draws and the rows the columns hold are the same numbers.
pub fn register() -> crate::register::Register {
    use crate::register::{Category, Group, Hull, HullClass, Kind, Register};
    let groups = [(1, "LAUT"), (2, "UDARA"), (3, "DARAT"), (4, "LAINNYA")];
    // Ten categories over the three groups that hold anything.
    let categories = [
        (10, 1, "KAPAL PERANG"),
        (11, 1, "KAPAL PATROLI"),
        (12, 1, "KAPAL BANTU"),
        (13, 2, "PESAWAT SAYAP TETAP"),
        (14, 2, "HELIKOPTER"),
        (15, 2, "PESAWAT TANPA AWAK"),
        (16, 3, "KENDARAAN LAPIS BAJA"),
        (17, 3, "ARTILERI"),
        (18, 3, "INFANTERI"),
        (19, 3, "PUSAT KOMANDO"),
    ];
    // Twenty-six types: the sheet's eight names, then numbered ones so the
    // column is as long as the frame's count says.
    let kind_names = [
        "FRIGATE",
        "CORVETTE",
        "FAST ATTACK",
        "PATROL",
        "SUBMARINE",
        "AMPHIBIOUS",
        "MINE WARFARE",
        "AUXILIARY",
        "PESAWAT ANGKUT",
        "PESAWAT LATIH",
        "HELIKOPTER SERBU",
        "HELIKOPTER ANGKUT",
        "UAV PENGAWAS",
        "UAV SERANG",
        "TANK",
        "PENGANGKUT",
        "ARTILERI MEDAN",
        "HOWITZER",
        "INFANTERI MEKANIS",
        "INFANTERI RINGAN",
        "KOMANDO",
        "PUSAT OPERASI",
        "PUSAT LATIHAN",
        "RUMAH SAKIT",
        "BENGKEL",
        "DEPOT",
    ];
    let mut kinds = Vec::new();
    for (at, name) in kind_names.iter().enumerate() {
        // Spread the types over the categories that own them, in order.
        let category = categories[at % categories.len()].0;
        kinds.push(Kind {
            id: 100 + at as i64,
            category,
            label: (*name).to_string(),
        });
    }
    // Forty-one classes: the sheet's own names first, then numbered ones.
    let class_names = [
        "FRIGAT SERBAGUNA",
        "FRIGAT RUDAL",
        "SIGMAS",
        "DIPONEGORO",
        "FAST ATTACK BOAT",
        "PATROL BOAT",
        "SUBMARINE DIESEL",
        "LANDING SHIP",
        "MINE COUNTERMEASURE",
        "TANKER",
        "CN-235",
        "C-130",
        "KT-1B",
        "SUPER PUMA",
        "NAS-332",
        "SCAN EAGLE",
        "LEOPARD 2",
        "AMX-13",
        "TRUCK ANOA",
        "ASTROS",
        "M109",
        "M113",
        "KOMANDO PASUKAN",
    ];
    let mut classes = Vec::new();
    for at in 0..41usize {
        let label = match class_names.get(at) {
            Some(name) => (*name).to_string(),
            None => format!("KELAS {}", at + 1),
        };
        classes.push(HullClass {
            id: 300 + at as i64,
            kind: kinds[at % kinds.len()].id,
            label,
        });
    }
    // A hundred and twenty-five hulls, round-robin over the classes so every
    // class holds at least three and the counts are spread.
    let mut hulls = Vec::new();
    for at in 0..125usize {
        let class = &classes[at % classes.len()];
        let number = format!("{}", 351 + at);
        hulls.push(Hull {
            id: 1000 + at as i64,
            class: class.id,
            name: format!("KRI {number}"),
            class_label: class.label.clone(),
            number,
        });
    }
    Register {
        groups: groups
            .iter()
            .map(|(id, label)| Group {
                id: *id,
                label: (*label).to_string(),
            })
            .collect(),
        categories: categories
            .iter()
            .map(|(id, group, label)| Category {
                id: *id,
                group: *group,
                label: (*label).to_string(),
            })
            .collect(),
        kinds,
        classes,
        hulls,
    }
}

/// F3a's path: LAUT ▸ FRIGATE ▸ FRIGAT SERBAGUNA ▸ AHMAD YANI. The sheet's
/// own breadcrumb, walked through this register's ids.
pub fn register_path() -> crate::register::Path {
    let register = register();
    let domain = register
        .groups
        .iter()
        .find(|g| g.label == "LAUT")
        .map(|g| g.id)
        .unwrap_or(1);
    let category = register
        .categories
        .iter()
        .find(|c| c.group == domain && c.label == "KAPAL PERANG")
        .map(|c| c.id)
        .unwrap_or(10);
    let kind = register
        .kinds
        .iter()
        .find(|k| k.category == category && k.label == "FRIGATE")
        .map(|k| k.id)
        .unwrap_or(100);
    let class = register
        .classes
        .iter()
        .find(|c| c.kind == kind && c.label == "FRIGAT SERBAGUNA")
        .map(|c| c.id)
        .unwrap_or(300);
    crate::register::Path::default()
        .pick(0, domain)
        .pick(1, category)
        .pick(2, kind)
        .pick(3, class)
}

/// F3d's draft: a reopened one, `12 UNITS · 9 PLACED · 3 UNPLACED` (`§5.5`).
///
/// Twelve hulls from the register, nine at a start position and three in the
/// game with none — the state whose two card marks the basket explains: a
/// FILLED PIN is on the map, a HOLLOW RING is in the game with no position.
pub fn reopened_twelve() -> Option<Draft> {
    use crate::force::{DraftHull, Start};
    let mut draft = stage_one(Frame::F2b)?;
    let register = register();
    let hulls: Vec<_> = register
        .hulls
        .iter()
        .filter(|h| draft.force.get(h.id).is_none())
        .take(12)
        .collect();
    for (at, hull) in hulls.iter().enumerate() {
        let placed = at < 9;
        draft.force.upsert(DraftHull {
            unit_id: hull.id,
            name: hull.name.clone(),
            class_id: hull.class_label.clone(),
            hull_number: hull.number.clone(),
            commander_id: None,
            // ON THE CHART THE CONSOLE OPENS ON, not merely valid ground: a
            // fixture hull at -3.2/108.9 is a legal position two hundred
            // kilometres off the right edge, and a capture of map mode with no
            // pins in it is a capture of the wrong thing.
            start: placed.then(|| Start {
                // Spread across the chart the console opens on, not clustered
                // under the middle of it: nine pins in one spot are one pin.
                lat: -6.00 - (at as f64 % 3.0) * 0.18,
                lon: 106.66 + (at as f64 % 4.0) * 0.20,
            }),
            course_deg: None,
            host: None,
        });
    }
    Some(draft)
}

/// F4a's draft: four groups, five units in them, four free (`§6.1`).
///
/// `OPGAB JAYA` (Operasi Gabungan) holds `GUS-1` (Gugus), which holds two
/// Satuan Tugas: `TG-1` with three hulls and `TG-2` with two. Three ranks
/// nested, which is the board's whole shape.
pub fn groups_draft() -> Option<Draft> {
    let mut draft = review_draft()?;
    draft.groups.clear();
    let opgab = draft
        .groups
        .stage_group(
            0,
            "OPGAB JAYA",
            crate::groups::GroupKind::OperasiGabungan,
            None,
            Some(2),
            vec![],
        )
        .ok()?;
    let gus = draft
        .groups
        .stage_group(
            0,
            "GUS-1",
            crate::groups::GroupKind::Gugus,
            Some(opgab),
            Some(3),
            vec![],
        )
        .ok()?;
    draft
        .groups
        .stage_group(
            0,
            "TG-1",
            crate::groups::GroupKind::SatuanTugas,
            Some(gus),
            Some(4),
            vec![351, 352, 353],
        )
        .ok()?;
    draft
        .groups
        .stage_group(
            0,
            "TG-2",
            crate::groups::GroupKind::SatuanTugas,
            Some(gus),
            Some(5),
            vec![365, 371],
        )
        .ok()?;
    Some(draft)
}

/// F5a's draft: five steps, one of them a beat, and three sub-steps in step
/// 03 — with four hulls left idle (`§7.1`).
pub fn book_draft() -> Option<Draft> {
    use crate::book::StepKind;
    let mut draft = groups_draft()?;
    draft.book.clear();
    let book = draft
        .book
        .stage_scenario_named("Harbour sweep", "the sweep north");
    let plan = [
        ("BRIEFING", StepKind::Orders, "0800", "0820"),
        ("TRANSIT", StepKind::Orders, "0820", "0910"),
        ("SWEEP", StepKind::Orders, "0930", "1015"),
        ("CONTACT", StepKind::Beat, "1015", "1040"),
        ("RECOVER", StepKind::Orders, "1100", "1130"),
    ];
    let mut sweep = None;
    for (content, kind, start, end) in plan {
        let local =
            draft
                .book
                .stage_step_new(book, content, Some((start.to_string(), end.to_string())))?;
        draft.book.set_kind(local, kind);
        if content == "SWEEP" {
            sweep = Some(local);
        }
    }
    let sweep = sweep?;
    for (content, units) in [
        ("TAKE STATION XYZ", vec![351, 352, 353]),
        ("TAKE STATION XYA", vec![354, 367]),
        ("HOLD RESERVE", vec![371]),
    ] {
        let sub = draft.book.stage_substep(sweep, content).ok()?;
        draft.book.set_substep_units(sweep, sub, units);
    }
    Some(draft)
}

/// F6a's draft: four seats and five people explicitly not playing (`§8.1`).
///
/// `GAME MASTER`, two `COMMANDO`s — one commanding a group and one a hull — and
/// a `REFEREE` on the judge side with no scope. The scopes come from the
/// groups' own commanders, so the two panes cannot disagree about who commands
/// what.
pub fn seats_draft() -> Option<Draft> {
    use crate::roster::StagedSeat;
    let mut draft = groups_draft()?;
    draft.roster.clear();
    // The groups' commanders are already 2 and 3 in `groups_draft`; the seats
    // name them, and a hull's commander (5) is the other commando.
    for (user_id, name, role, judge) in [
        (2, "Cdr. Anwar", "GAME MASTER", false),
        (3, "Lt. Fitri", "COMMANDO", false),
        (5, "Rina Wijaya", "COMMANDO", false),
        (11, "Kpt. Sari", "REFEREE", true),
    ] {
        draft.roster.stage(StagedSeat {
            user_id,
            user_name: name.to_string(),
            role_id: 0,
            role_name: role.to_string(),
            call_sign: name.to_string(),
            judge,
        });
    }
    // Five people considered and declined — NOT PLAYING writes nothing, and
    // still counts as a decision (`§8.2`).
    for user_id in [20, 21, 22, 23, 24] {
        draft.roster.remove(user_id, false);
    }
    Some(draft)
}

/// F8: the arrival, with the gate the sheet draws (`§10`).
///
/// PLAY DIRECTLY is the arrival that has a gate, so it is the one the frame
/// shows: four seated, two ready, and the server refusing with its own
/// sentence. The strip's ticks are the client's reading, and the verdict is
/// `can_execute`.
pub fn arrival() -> Option<crate::arrival::Arrival> {
    let draft = seats_draft()?;
    let (placed, unplaced) = draft.placement_counts();
    Some(crate::arrival::Arrival::of(
        crate::draft::Verb::PlayDirectly,
        Some(128),
        (placed, unplaced),
        draft.unit_count(),
        draft.book.staged_step_count(),
        draft.book.all_substeps().count(),
        draft.considered(),
    ))
}

/// F8's gate strip.
pub fn arrival_gate() -> Option<crate::arrival::Gate> {
    let draft = seats_draft()?;
    let (placed, unplaced) = draft.placement_counts();
    Some(crate::arrival::Gate::of(
        &crate::backend::GameReadinessView {
            can_execute: false,
            blockers: vec!["2 participants have not declared themselves ready".to_string()],
            fast: false,
        },
        true,
        draft.unit_count(),
        (placed, unplaced),
        draft.force.uncommanded().len(),
        // The EXERCISE SIDE: the referee is not counted (erratum 10).
        (2, 3),
    ))
}

/// The state a `TFG_FLOW` value names, if its slice has landed./// The state a `TFG_FLOW` value names, if its slice has landed./// The state a `TFG_FLOW` value names, if its slice has landed./// The state a `TFG_FLOW` value names, if its slice has landed./// The state a `TFG_FLOW` value names, if its slice has landed./// The state a `TFG_FLOW` value names, if its slice has landed./// The state a `TFG_FLOW` value names, if its slice has landed.
#[derive(Debug, Clone, PartialEq)]
pub enum Fixture {
    /// F1a and F1b: the landing, with its history and its one question.
    Landing(Box<Landing>),
    /// F2a and F2b: a draft at stage 01.
    StageOne(Box<Draft>),
    /// F7: a draft at stage 06, writable.
    Review(Box<Draft>),
    /// F3a–F3e: a draft at stage 02, with the register behind it.
    StageTwo(Box<Draft>),
    /// F4a–F4c: a draft at stage 03, with its task organisation staged.
    StageThree(Box<Draft>),
    /// F5a, F5b: a draft at stage 04, with its book staged.
    StageFour(Box<Draft>),
    /// F6a, F6b: a draft at stage 05, with its seats staged.
    StageFive(Box<Draft>),
    /// F8: the arrival panel, with its gate.
    Arrival,
}

/// Build the frame's state, or `None` when that slice has not landed.
pub fn fixture(frame: Frame) -> Option<Fixture> {
    match frame {
        Frame::F1a => Some(Fixture::Landing(Box::new(landing_nothing_chosen()))),
        Frame::F1b => Some(Fixture::Landing(Box::new(landing_scenario_revealed()))),
        Frame::F2a | Frame::F2b => stage_one(frame).map(|d| Fixture::StageOne(Box::new(d))),
        Frame::F7 => review_draft().map(|d| Fixture::Review(Box::new(d))),
        // F3d is the basket open on a REOPENED draft: twelve hulls, nine with
        // a start position.
        Frame::F3d => reopened_twelve().map(|d| Fixture::StageTwo(Box::new(d))),
        Frame::F3a | Frame::F3b | Frame::F3c | Frame::F3e => {
            stage_one(Frame::F2b).map(|d| Fixture::StageTwo(Box::new(d)))
        }
        Frame::F4a | Frame::F4b | Frame::F4c => {
            groups_draft().map(|d| Fixture::StageThree(Box::new(d)))
        }
        Frame::F5a | Frame::F5b => book_draft().map(|d| Fixture::StageFour(Box::new(d))),
        Frame::F6a | Frame::F6b => seats_draft().map(|d| Fixture::StageFive(Box::new(d))),
        Frame::F8 => Some(Fixture::Arrival),
        _ => None,
    }
}

/// The `TFG_FLOW` value in this process's environment, if it names a frame.
pub fn flow_frame() -> Option<Frame> {
    std::env::var("TFG_FLOW").ok().and_then(|v| Frame::of(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{BaseState, Verb};
    use crate::landing::{PROCEED_DISABLED, Proceed};

    /// The family parses its own labels, and nothing else.
    #[test]
    fn every_frame_parses_its_own_label() {
        for frame in [
            Frame::F1a,
            Frame::F1b,
            Frame::F2a,
            Frame::F2b,
            Frame::F3a,
            Frame::F8,
        ] {
            assert_eq!(Frame::of(frame.label()), Some(frame));
        }
        assert_eq!(Frame::of(" F1A "), Some(Frame::F1a), "typed by a person");
        assert_eq!(Frame::of("f9"), None);
        assert_eq!(Frame::of(""), None);
    }

    /// The history is EIGHT GAMES and the four states partition them: that is
    /// `GAME SESSIONS · 08`, and it is the number the sheet's own example gets
    /// wrong.
    #[test]
    fn the_history_is_eight_games_in_four_states() {
        let landing = landing_nothing_chosen();
        assert_eq!(landing.games().len(), 8);
        let counts = landing.counts();
        assert_eq!(counts[0].1, 8, "ALL");
        assert_eq!(
            counts[1..].iter().map(|(_, n)| n).sum::<usize>(),
            8,
            "planning + preparation + execution + closure"
        );
        assert_eq!(counts[2].1, 2, "preparation is a state and a tab");
        assert_eq!(
            landing.proceed(),
            Proceed::Disabled(PROCEED_DISABLED),
            "F1a has chosen nothing"
        );
    }

    /// F1b: `SCENARIO` chosen reveals START FROM, no row chosen, so PROCEED is
    /// still disabled — `BLANK SCENARIO` is a row.
    #[test]
    fn the_scenario_frame_reveals_start_from_with_no_row_chosen() {
        let landing = landing_scenario_revealed();
        assert_eq!(landing.mode(), Some(GameMode::Scenario));
        let rows = landing.start_rows();
        assert_eq!(rows.len(), 4, "blank plus the three Saved Scenarios");
        assert_eq!(rows[1].title, "Baltic Shield 25");
        assert_eq!(rows[1].steps, "9 STEPS");
        assert_eq!(rows[1].when, "18 OCT");
        assert!(matches!(
            landing.proceed(),
            Proceed::Disabled(crate::landing::START_FROM_UNCHOSEN)
        ));
    }

    /// F2a is the top of the form: five fields filled, the time base empty and
    /// LEGAL — an empty base is an ordinary planning state, and only the verbs
    /// are gated.
    #[test]
    fn the_top_of_the_form_has_five_fields_and_no_time_base() {
        let draft = stage_one(Frame::F2a).expect("a draft");
        assert_eq!(draft.terms.name, "Baltic Shield 26");
        let filled = [
            &draft.terms.name,
            &draft.terms.description,
            &draft.terms.purpose,
            &draft.terms.target,
            &draft.terms.area,
        ]
        .iter()
        .filter(|v| !v.is_empty())
        .count();
        assert_eq!(filled, 5, "the frame's five filled fields");
        assert_eq!(draft.terms.time.state(), BaseState::Empty);
        assert_eq!(draft.terms.time.problem(), None, "empty is not broken");
        assert!(draft.terms.time.window("+07:00").is_none());
    }

    /// F2b is scrolled to a COMPLETE time base, and both ends derive from it —
    /// the same derivation the flow's stage 01 uses, never a typed end.
    #[test]
    fn the_time_base_frame_derives_both_windows() {
        let draft = stage_one(Frame::F2b).expect("a draft");
        let time = &draft.terms.time;
        assert_eq!(time.state(), BaseState::Complete);
        let window = time.window("+07:00").expect("a window");
        assert_eq!(
            window.actual_start.as_deref(),
            Some("2026-11-01T08:00:00+07:00")
        );
        assert_eq!(
            window.actual_end.as_deref(),
            Some("2026-11-01T09:30:00+07:00")
        );
        assert_eq!(
            window.assumed_end.as_deref(),
            Some("2000-01-01T09:00:00+07:00")
        );
        assert_eq!(time.elapsed_hours(), Some(3.0));
        // A DRAFT WITH A NAME AND A WINDOW AND NOTHING ELSE IS NOT WRITABLE.
        // The force is empty and so is the book, and the console refuses to
        // save a record that opens into nothing; the blockers name the two
        // stages, in rail order.
        let blocking = draft.blockers(Verb::PlayDirectly);
        assert_eq!(blocking.len(), 3, "{blocking:?}");
        assert_eq!(blocking[0].stage, crate::draft::Stage::Units);
        assert_eq!(blocking[1].stage, crate::draft::Stage::Scenario);
        // Play Directly also transitions, and a game with nobody on the
        // exercise side cannot move to preparation.
        assert_eq!(blocking[2].stage, crate::draft::Stage::Players);
        assert_eq!(
            draft.stage_count(),
            6,
            "the sheet's F2 is a scenario draft: six stages"
        );
        assert_eq!(
            draft.origin,
            crate::draft::Origin::Blank {
                mode: GameMode::Scenario
            }
        );
    }

    /// F7 IS A DRAFT THAT CAN BE WRITTEN: every verb it earns has no blocker.
    /// A capture of a review frame over a draft the press would refuse is a
    /// picture of a disabled button, not of the commit rail.
    /// THE REGISTER HAS THE SHEET'S SHAPE, and its counts agree with its rows:
    /// four groups (one empty), and 125 hulls reachable.
    #[test]
    fn the_register_has_the_sheets_shape() {
        let register = super::register();
        assert_eq!(register.groups.len(), 4, "the endpoint returns four");
        assert_eq!(register.hull_total(), 125, "ASET 125");
        let cascade = register.cascade(&crate::register::Path::default());
        let empty = cascade.columns[0]
            .iter()
            .find(|r| r.label == "LAINNYA")
            .expect("the empty group is drawn");
        assert_eq!(empty.count, 0, "a zero is a number");
        let laut = cascade.columns[0]
            .iter()
            .find(|r| r.label == "LAUT")
            .expect("LAUT");
        assert!(laut.count > 0, "LAUT holds hulls");
        assert!(
            cascade.columns[0].iter().map(|r| r.count).sum::<usize>() == 125,
            "the four groups partition the register"
        );
        // F3a's path walks to a class with cards in it.
        let path = super::register_path();
        let at = register.cascade(&path);
        assert_eq!(
            at.breadcrumb.len(),
            4,
            "LAUT ▸ KAPAL PERANG ▸ FRIGATE ▸ CLASS"
        );
        assert!(!at.hulls.is_empty(), "the ASET column is not empty");
    }

    /// F4a IS FOUR GROUPS WITH FIVE UNITS IN THEM, in three nested ranks.
    #[test]
    fn the_groups_board_is_four_groups_five_units() {
        let draft = super::groups_draft().expect("a groups draft");
        let groups: Vec<_> = draft.groups.staged_groups().cloned().collect();
        assert_eq!(groups.len(), 4, "OPGAB, GUS-1, TG-1, TG-2");
        let in_groups: usize = groups.iter().map(|g| g.units.len()).sum();
        assert_eq!(in_groups, 5, "three in TG-1 and two in TG-2");
        let tree = crate::groups::tree(&groups);
        assert_eq!(
            tree.iter().map(|n| n.depth).collect::<Vec<_>>(),
            vec![0, 1, 2, 2],
            "a root, a child, and two grandchildren"
        );
        assert_eq!(tree[0].group.name, "OPGAB JAYA");
    }

    /// F3d IS TWELVE HULLS, NINE PLACED — the basket's own counts, and the
    /// state its two card marks explain.
    #[test]
    fn the_basket_draft_is_twelve_nine_three() {
        let draft = super::reopened_twelve().expect("a reopened draft");
        assert_eq!(draft.unit_count(), 12);
        assert_eq!(draft.placement_counts(), (9, 3));
        assert!(
            draft.force.get(0).is_none(),
            "no hull with a made-up id is staged"
        );
    }

    #[test]
    fn the_review_draft_is_writable() {
        use crate::gamestate::GameMode as Mode;
        let draft = review_draft().expect("a review draft");
        for verb in draft.verbs().iter().copied() {
            assert!(
                draft.blockers(verb).is_empty(),
                "{} is blocked: {:?}",
                verb.label(),
                draft.blockers(verb)
            );
        }
        // Nine hulls, each commanded by its own commando, and the referee on
        // the judge side commands nothing.
        assert_eq!(draft.unit_count(), 9);
        assert_eq!(draft.placement_counts(), (9, 0));
        assert_eq!(draft.considered().0, 11, "ten commandoes and a referee");
        assert_eq!(draft.group_count(), 1);
        assert_eq!(draft.staged_step_count(), 5);
        assert_eq!(draft.mode, Mode::Scenario, "a book needs a scenario");
        assert_eq!(draft.stage_count(), 6);
    }

    /// EVERY BUILT FRAME ANSWERS A FIXTURE. A frame that says it is built and
    /// then hands back nothing is a capture of the landing pretending to be
    /// that frame — which is exactly what happened once.
    #[test]
    fn every_built_frame_has_a_fixture() {
        for frame in [
            Frame::F1a,
            Frame::F1b,
            Frame::F2a,
            Frame::F2b,
            Frame::F3a,
            Frame::F3b,
            Frame::F3c,
            Frame::F3d,
            Frame::F3e,
            Frame::F4a,
            Frame::F4b,
            Frame::F4c,
            Frame::F5a,
            Frame::F5b,
            Frame::F6a,
            Frame::F6b,
            Frame::F7,
            Frame::F8,
        ] {
            assert!(frame.built(), "{} is marked built", frame.label());
            assert!(
                fixture(frame).is_some(),
                "{} is built and answers a fixture",
                frame.label()
            );
        }
    }

    /// A frame whose slice has not landed builds NOTHING rather than half a
    /// state: a picture of a fixture would not be a picture of the flow.
    #[test]
    fn an_unbuilt_frame_has_no_fixture() {
        assert!(Frame::F8.built(), "the arrival panel has landed");
        assert_eq!(fixture(Frame::F8), Some(Fixture::Arrival));
        assert!(fixture(Frame::F3a).is_some(), "stage 02 has landed");
        assert!(Frame::F2b.built());
        assert!(fixture(Frame::F2b).is_some());
    }
}

#[cfg(test)]
mod book_fixture_tests {
    /// F5a IS FIVE STEPS, ONE BEAT, THREE SUB-STEPS AND FOUR IDLE HULLS.
    #[test]
    fn the_book_fixture_is_the_sheets_state() {
        let draft = super::book_draft().expect("a book draft");
        let scenario = draft.book.staged_news().next().expect("a scenario");
        assert_eq!(scenario.steps.len(), 5, "five steps");
        assert_eq!(
            scenario
                .steps
                .iter()
                .filter(|s| s.kind == crate::book::StepKind::Beat)
                .count(),
            1,
            "one beat"
        );
        let sweep = scenario
            .steps
            .iter()
            .find(|s| s.content == "SWEEP")
            .expect("SWEEP");
        let subs: Vec<_> = draft.book.substeps_of(sweep.local_id).to_vec();
        assert_eq!(subs.len(), 3, "three sub-steps");
        assert_eq!(subs[0].units, vec![351, 352, 353]);
        assert_eq!(subs[1].units, vec![354, 367]);
        assert_eq!(subs[2].units, vec![371]);
        let force: Vec<i64> = draft.force.hulls().map(|h| h.unit_id).collect();
        let refs: Vec<&crate::book::StagedSubStep> = subs.iter().collect();
        assert_eq!(
            crate::book::idle_in_step(&force, &refs).len(),
            3,
            "nine hulls, six commanded, three idle"
        );
    }
}
