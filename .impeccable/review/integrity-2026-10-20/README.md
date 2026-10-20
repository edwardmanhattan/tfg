# Visual integrity review — 2026-10-20

The capture set behind `docs/visual-integrity-audit.md`.

## Three folders, and why there are three

| folder | server | when | what it shows |
|---|---|---|---|
| `before/` | the LOCAL Minos stack | the audit's first pass | the defects as they were found |
| `after-local/` | the LOCAL Minos stack | after the fixes | the fixes, on the same server as `before/` |
| `after-hosted/` | the **hosted dev API** | after the fixes | the console as it actually runs |

`after-local/` and `after-hosted/` are the same code. The difference
between them is the DATA, and that is the point: the local stack is a
throwaway database with a fixture in it, and the fixture's hull names run
to 20 characters where the real register's run to 31. Two defects — the
rail's picture channel overrunning its edge, and the Command island's HULLS
row cutting a hull name in half — are invisible in `after-local/` and
plain in `after-hosted/`.

The audit's first pass ran against the local stack. That was a mistake, and
the folder is kept rather than deleted so the mistake is visible rather
than tidied away.

## The games

`after-hosted/` was captured against `api.tfg.development.crossnet.co.id`:

| state | game | what it holds |
|---|---|---|
| planning | `9` "Game 8" | a Commando seat, one hull, a room of three |
| preparation | `41` "GPUI helm proof" | a Commando seat and a commanded hull — the HULLS row with a real name in it |
| execution | `44` "GPUI step proof" | a Commando seat, one hull, one scenario step, a live picture |
| closure | `11` "Game 9" | a judge seat, one piece, a scenario with two steps |
| no session | — (`TFG_SESSIONS=1`) | the session picker over the real games list |

`fleet-leaf-long-names.png` is the fleet picker drilled to a class, where
the real hull names and their photographs are drawn. The drill is four
levels deep and a capture cannot click, so the state is pinned by id:

    TFG_GAME=41 TFG_FLEET=1 TFG_DRILL_GROUP=2 TFG_DRILL_CATEGORY=2 \
      TFG_DRILL_TYPE=2 TFG_DRILL_CLASS=1

which is Sea → Frigate → FF → Ahmad Yani / Van Speijk, the class that
holds `KRI Abdul Halim Perdanakusuma` and `KRI Abdul Halim Perdanakusuma
(KRI-355)`. The ids come from the console's own store
(`~/.local/share/tfg/tfg-local.db`, tables `helpers`, `unit_categories`,
`unit_types`, `unit_classes`, `units`) — the taxonomy the hosted server
publishes, cached locally by the app.

The whole `after-hosted/` set was re-shot from the current build after the
sign-in rewrite, so every picture in it is a picture of the code as it
stands and of the hosted deployment's data. One of the earlier captures,
`inspector.png`, had been taken during a fixture fallback — it was a
picture of the fixture and not of the deployment, and it was filed here as
one. That is what the harness now reports out loud.

## The sidebar

The `after-hosted/` set was taken with the column DOCKED: flush to the
window's left edge, running to its bottom edge, and its fields joined so
they share one 1px rule instead of standing apart with a strip of map
between them. The `before/` set is the same console with the column
floating 24pt in and the fields 2pt apart — which is the difference the
comparison is about.

`after-hosted/prep-command-crew.png` is the clearest one: three fields
(OPERATOR, COMMAND, CREW) in one continuous column, each with its own
band, and one rule between them.

`after-hosted/column-owned-edge.png` is the same console with the pointer
pinned over the ESSENTIALS field, so the field that owns the input carries
the cyan perimeter: up one side, across the top, down the other, and along
the bottom, closing over the rule the field above owns.

Those two facts are asserted, not photographed. `scripts/gpui-sweep.sh`
runs `scripts/shot-assert.sh` after each capture and exits non-zero when a
pixel is wrong:

    scripts/gpui-sweep.sh after \
      "plan:TFG_GAME=9|at:0,300=cut-grey;at:399,300=cut-grey;runs:250=cut-grey=3"

`runs:250=cut-grey=3` is the whole claim of the connected column in one
number: three rules in that column — the first field's top, the rule
between the first and second fields, and the second field's bottom — and
not four, which is what a stack of separate cards would have.

## The capture harness

`scripts/gpui-capture.sh`, on the Wayland compositor — the console is a
GPUI/Vulkan app and does not render on Xvfb. The script exports the hosted
superuser's credentials, so a capture with no host override talks to the
hosted API.

It also reports what it caught: how many wheels and clicks reached the
console while the picture was being taken, and where they landed. A
capture that caught the operator's own wheel is a photograph of a state
nobody asked for, and that is exactly what one of these was — a "scrolled
column" that sent me hunting for a layout bug that did not exist. And when
the hosted API does not answer, the console falls back to its fixture
roster and the picture is of the FIXTURE; the harness says so, because a
picture of the fixture filed as a picture of the deployment is worse than
no picture at all.
