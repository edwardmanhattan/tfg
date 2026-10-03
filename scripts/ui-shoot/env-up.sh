#!/usr/bin/env bash
# Bring up everything the console needs to be photographed, in one command.
#
#   bash scripts/ui-shoot/env-up.sh
#
# Nothing here survives a reboot: Postgres and valkey run out of /tmp with no
# root, the display is an Xvfb, and Minos is a locally built binary against a
# throwaway database. Re-running is the normal case — every step checks whether
# it is already done.
#
# WHAT THIS REPLACES. `docs/ui-verify-handoff.md` spelled the same bring-up out
# as prose, and the prose was not enough: it assumed /tmp/opencode/stack and
# /tmp/opencode/minos.env already existed, which is true only within one
# session. Those two are the whole cold-start cost, and they are made here.
#
# TWO THINGS THAT ARE NOT OBVIOUS AND COST THE LAST TWO SESSIONS:
#
#   * `migrator` parses flags BEFORE the subcommand. `migrator up --env F` puts
#     `--env F` into `up`'s argument list, the env file is never read, and the
#     migrator silently connects to the DEFAULTS — user postgres, database
#     janus, port 5432 — which fails as "connection refused" and reads as a
#     broken database rather than a mis-ordered flag.
#   * Minos validates REDIS_PASSWORD as required, so a valkey with no
#     requirepass is not an option: the API refuses to boot. stack.sh owns that
#     password.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MINOS_REPO="${MINOS_REPO:-$HOME/Work/Crossnet/minos}"
ROOT=/tmp/opencode
STACK=$ROOT/stack
PKGS=$ROOT/pkgs
SHOTS=$ROOT/shots

mkdir -p "$ROOT" "$PKGS" "$STACK" "$SHOTS"

say() { printf '\n== %s\n' "$1"; }

say "unpack the stack"
# Postgres, valkey and numactl are not installed here and there is no root, so
# the Arch packages are extracted into a prefix. Postgres needs libnuma at
# runtime, which is why numactl is in the list.
cd "$PKGS"
for pkg in postgresql valkey numactl; do
  url=$(pacman -Sp "$pkg" | tail -1)
  file=$(basename "$url")
  [ -f "$file" ] || curl -sSLo "$file" "$url"
  tar --zstd -xf "$file" -C "$STACK"
done
export PATH="$STACK/usr/bin:$PATH"
export LD_LIBRARY_PATH="$STACK/usr/lib"

say "postgres data directory"
# initdb only ever runs once. It cannot run as root, which is why this whole
# arrangement exists.
[ -d "$ROOT/pgdata" ] || initdb -D "$ROOT/pgdata" -U minos \
  --auth-local=trust --auth-host=trust --encoding=UTF8 --locale=C >/dev/null

say "start postgres and valkey"
bash "$REPO/scripts/ui-shoot/stack.sh" up

say "the minos database"
psql -h 127.0.0.1 -p 5433 -U minos -d postgres -tAc \
  "select 1 from pg_database where datname='minos'" | grep -q 1 || \
  createdb -h 127.0.0.1 -p 5433 -U minos minos

say "minos env"
# SUPERUSER_USERNAME is `supersuser`, which looks like a typo and is not: both
# committed helpers (seedgame.py, seat.py) and the handoff log in with it, so
# the account name is the repo's contract, not a local choice. SUPERUSER_NAME
# is quoted because it contains a space, which is why run-minos.sh sources this
# file instead of handing it to `env`.
[ -f "$ROOT/minos.env" ] || cat > "$ROOT/minos.env" <<'ENV'
ENV=development
TZ=Asia/Jakarta
SERVICE_NAME=minos
APP_HOST=127.0.0.1:8099
HTTP_ROOT=/api
HTTP_PORT=8099
GRPC_PORT=9099

PG_HOST=127.0.0.1
PG_PORT=5433
PG_USER=minos
PG_PASSWORD=local-owner-pw
PG_DB_NAME=minos
PG_SSL_MODE=disable
PG_MAX_CONS=10

MIGRATE_PG_HOST=127.0.0.1
MIGRATE_PG_PORT=5433
MIGRATE_PG_USER=minos
MIGRATE_PG_PASSWORD=local-owner-pw
MIGRATE_PG_DB_NAME=minos
MIGRATE_PG_SSL_MODE=disable

REDIS_HOST=127.0.0.1
REDIS_PORT=6380
REDIS_PASSWORD=local-redis-pw
REDIS_DB_NUM=0

JWT_SECRET_KEY=local-jwt-secret-key-not-a-secret
REFRESH_TOKEN_SECRET=local-refresh-secret-key-not-a-secret
JWT_DURATION_SECONDS=3600
REFRESH_TOKEN_DURATION_SECONDS=604800
COOKIE_REFRESH_PATH=/auth
COOKIE_SECURE=false
COOKIE_SAME_SITE=lax

AWS_ENDPOINT=http://127.0.0.1:3900
AWS_SECURE=false
AWS_REGION=garage
AWS_BUCKET=minos
AWS_ACCESS_KEY_ID=local-access-key
AWS_SECRET_ACCESS_KEY=local-secret-key
AWS_PRESIGNED_URL_LIFETIME_SECONDS=3600

CENTRIFUGO_API_URL=http://127.0.0.1:8000
CENTRIFUGO_API_KEY=local-centrifugo-key

CASBIN_MODEL_PATH=envs/casbin_models.conf

SUPERUSER_EMAIL=supersuser@tfg.local
SUPERUSER_USERNAME=supersuser
SUPERUSER_NAME="Super User"
SUPERUSER_PASSWORD=tfg-dev-password

DEFAULT_DEV_USER_PASSWORD=password
ENV

say "build minos"
cd "$MINOS_REPO"
go build -o "$ROOT/minos" ./cmd/minos
go build -o "$ROOT/migrator" ./cmd/migrator

say "cluster roles"
# Roles are cluster objects and a migration will not create them, so this runs
# before `migrator up` every time — it is idempotent and cheap.
set -a; . "$ROOT/minos.env"; set +a
MINOS_APP_PASSWORD=local-app-pw MINOS_RO_PASSWORD=local-ro-pw \
  PGHOST=127.0.0.1 PGPORT=5433 PGUSER=minos PGPASSWORD="$PG_PASSWORD" PGDATABASE=minos \
  bash scripts/init_db_roles.sh | tail -1

say "migrate and seed"
# Flags BEFORE the subcommand. See the note at the top.
"$ROOT/migrator" --env "$ROOT/minos.env" up | tail -1
"$ROOT/migrator" --env "$ROOT/minos.env" --allow-superuser-seed seed-superuser | tail -1

say "start minos"
cat > "$ROOT/run-minos.sh" <<'RUNNER'
#!/usr/bin/env bash
# Source the env rather than assembling it with `env $(cat ...)`: SUPERUSER_NAME
# is "Super User" with embedded quotes, and word splitting hands the shell two
# arguments where one is expected.
set -euo pipefail
cd "${MINOS_REPO:-$HOME/Work/Crossnet/minos}"
set -a
. /tmp/opencode/minos.env
set +a
exec /tmp/opencode/minos "$@"
RUNNER
chmod +x "$ROOT/run-minos.sh"

if [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8099/api/api/v1/games)" != "000" ]; then
  # 401 is the healthy answer: /games needs a bearer token. Testing for 2xx
  # here reads a working API as a dead one and starts a second minos onto an
  # occupied port.
  echo "already listening on 8099"
  # Casbin loads its policy set at startup, so an account seeded after the
  # process started keeps being denied. If the seed above created the
  # superuser, this start is what makes it usable.
  setsid "$ROOT/run-minos.sh" > "$ROOT/minos.log" 2>&1 < /dev/null &
  for _ in $(seq 1 30); do
    curl -sf -o /dev/null http://127.0.0.1:8099/api/api/v1/games && break
    sleep 1
  done
fi
curl -s -o /dev/null -w 'minos /games -> %{http_code}\n' \
  http://127.0.0.1:8099/api/api/v1/games

say "private display"
bash "$REPO/scripts/ui-shoot/xvfb-up.sh" | tail -1

say "seed a game to photograph"
# A game in planning with a scenario book and a Game Master seat, which is what
# the three modals need before they have anything to show.
cd "$REPO"
# Reuse the newest SHOOT-01 rather than making another. Re-running this script
# should not fill the list with duplicates, and a duplicate is exactly the
# state the console's own session picker makes expensive to unpick.
GID=$(TOK=$(curl -s -X POST http://127.0.0.1:8099/api/api/v1/auth/login \
        -H 'Content-Type: application/json' \
        -d '{"identifier":"supersuser","password":"tfg-dev-password"}' \
        | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["access_token"])'); \
      curl -s -H "Authorization: Bearer $TOK" \
        'http://127.0.0.1:8099/api/api/v1/games?page_size=100&page_number=1' \
      | python3 -c '
import sys, json
rows = json.load(sys.stdin)["data"]
rows = rows if isinstance(rows, list) else rows.get("games", [])
named = [g for g in rows if g.get("name") == "SHOOT-01"]
print(max(named, key=lambda g: g["id"])["id"] if named else "")
')
if [ -z "$GID" ]; then
  python3 scripts/ui-shoot/seedgame.py SHOOT-01 | tail -2
  GID=$(curl -s -X POST http://127.0.0.1:8099/api/api/v1/auth/login \
          -H 'Content-Type: application/json' \
          -d '{"identifier":"supersuser","password":"tfg-dev-password"}' \
        | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["access_token"])' \
        | xargs -I{} curl -s -H "Authorization: Bearer {}" \
          'http://127.0.0.1:8099/api/api/v1/games?page_size=100&page_number=1' \
        | python3 -c '
import sys, json
rows = json.load(sys.stdin)["data"]
rows = rows if isinstance(rows, list) else rows.get("games", [])
named = [g for g in rows if g.get("name") == "SHOOT-01"]
print(max(named, key=lambda g: g["id"])["id"] if named else "")
')
fi
# The Game Master seat is what makes the scenario book reachable: authoring is
# gated on a game role, not an application one.
python3 scripts/ui-shoot/seat.py "$GID" | tail -4

say "done"
cat <<EOF
  app:      cd $REPO && setsid env -u WAYLAND_DISPLAY DISPLAY=:99 \\
              TFG_MINOS_HOST=http://127.0.0.1:8099/api/api/v1 \\
              TFG_ZONE_DEBUG=1 LIBGL_ALWAYS_SOFTWARE=1 \\
              ./target/debug/tfg > $ROOT/tfg.log 2>&1 < /dev/null &
  sign in:  identifier supersuser / password tfg-dev-password
  wait 30s after launch before driving it. Software rendering plus map tiles
  means the first seconds drop clicks and keystrokes, which reads as a dead
  control rather than a busy one.
  drive:    export DISPLAY=:99 XDRV_DISPLAY=:99 X=/tmp/opencode/xdrv
  shoot:    /tmp/opencode/shot.sh $SHOTS/name.png
EOF