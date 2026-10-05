#!/bin/sh
# Run from any working directory. Extra flags go to k6, e.g. -e PROFILE=smoke
# Pass a script name to run that file: ./k6/run.sh breakpoint.js
set -e
cd "$(dirname "$0")"
script=traffic.js
if [ -n "$1" ] && [ -f "$1" ]; then
  script=$1
  shift
fi
exec k6 run "$script" "$@"
