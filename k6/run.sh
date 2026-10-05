#!/bin/sh
# Run from any working directory. Extra flags go to k6, e.g. -e PROFILE=smoke
set -e
cd "$(dirname "$0")"
exec k6 run traffic.js "$@"
