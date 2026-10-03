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

case "${1:-up}" in
up)
  pg_isready -h 127.0.0.1 -p 5433 >/dev/null 2>&1 || {
    pg_ctl -D $ROOT/pgdata -l $ROOT/pg.log \
      -o "-p 5433 -c listen_addresses=127.0.0.1 -c unix_socket_directories=$ROOT" \
      start >/dev/null 2>&1
  }
  for _ in $(seq 1 30); do pg_isready -h 127.0.0.1 -p 5433 >/dev/null 2>&1 && break; sleep 1; done
  pg_isready -h 127.0.0.1 -p 5433

  redis-cli -p 6380 ping >/dev/null 2>&1 || \
    ( valkey-server --port 6380 --save '' --dir $ROOT > $ROOT/valkey.log 2>&1 & )
  for _ in $(seq 1 20); do redis-cli -p 6380 ping >/dev/null 2>&1 && break; sleep 1; done
  echo -n "valkey: "; redis-cli -p 6380 ping
  ;;

down)
  pg_ctl -D $ROOT/pgdata stop -m fast >/dev/null 2>&1
  pkill -f 'valkey-server.*6380' 2>/dev/null
  echo stopped
  ;;
esac