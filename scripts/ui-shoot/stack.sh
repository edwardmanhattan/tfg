#!/bin/bash
# The Minos stack, running out of /tmp with no root and no docker.
#
# Postgres and Valkey are not installed on this machine and there is no root,
# so the Arch packages were extracted into a local prefix. Postgres then needs
# LD_LIBRARY_PATH for libnuma (from numactl, also extracted).
set -u
export LD_LIBRARY_PATH=/tmp/opencode/stack/usr/lib
export PATH=/tmp/opencode/stack/usr/bin:$PATH
ROOT=/tmp/opencode
# Minos validates REDIS_PASSWORD as required, so an unauthenticated valkey is not
# an option: the API refuses to boot without one. The password is local-only.
VALKEY_PASSWORD=local-redis-pw
VALKEY_CLI="redis-cli -p 6380 -a $VALKEY_PASSWORD --no-auth-warning"

case "${1:-up}" in
up)
  pg_isready -h 127.0.0.1 -p 5433 >/dev/null 2>&1 || {
    pg_ctl -D $ROOT/pgdata -l $ROOT/pg.log \
      -o "-p 5433 -c listen_addresses=127.0.0.1 -c unix_socket_directories=$ROOT" \
      start >/dev/null 2>&1
  }
  for _ in $(seq 1 30); do pg_isready -h 127.0.0.1 -p 5433 >/dev/null 2>&1 && break; sleep 1; done
  pg_isready -h 127.0.0.1 -p 5433

  # Compare the reply, do not test the exit code: valkey answers PING after a
  # failed AUTH when the default user has no password, so a wrong password still
  # exits 0 and reads as "up".
  [ "$($VALKEY_CLI ping 2>/dev/null)" = PONG ] || \
    ( valkey-server --port 6380 --requirepass "$VALKEY_PASSWORD" --save '' --dir $ROOT \
      > $ROOT/valkey.log 2>&1 & )
  for _ in $(seq 1 20); do [ "$($VALKEY_CLI ping 2>/dev/null)" = PONG ] && break; sleep 1; done
  echo -n "valkey: "; $VALKEY_CLI ping 2>/dev/null || echo "no answer"
  ;;

down)
  pg_ctl -D $ROOT/pgdata stop -m fast >/dev/null 2>&1
  pkill -f 'valkey-server.*6380' 2>/dev/null
  echo stopped
  ;;
esac