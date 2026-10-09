#!/bin/sh
# Run one of these analysis scripts with the refs venv, isolated (-I).
#   sh tools/birds/py.sh transcribe.py args...
HERE=$(cd "$(dirname "$0")" && pwd)
S=$1; shift
exec /Users/joakimeriksson/work/gamesynth/target/refs/.venv/bin/python -I "$HERE/$S" "$@"
