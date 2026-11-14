#!/bin/sh
# Set the UI password on first start (or when UFOP_SERVER_PASSWORD changes
# is not detected: run `ufop-server --set-password` to change it later).
set -e
if [ -n "$UFOP_SERVER_PASSWORD" ] && [ ! -f "$UFOP_SERVER_DATADIR/.password-set" ]; then
  ufop-server --set-password
  touch "$UFOP_SERVER_DATADIR/.password-set"
fi
exec ufop-server "$@"
