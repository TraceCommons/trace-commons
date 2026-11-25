#!/bin/sh
# Draws the merged chart from TLC's full state dump. Needs FLOW_PROMISES (the skill folder),
# TLA2TOOLS (tla2tools.jar 1.7.4) and JAVA (Java 21+); the promises are checked by the skill's
# scripts/check, as README.md says.
set -eu
cd "$(dirname "$0")"
SKILL=${FLOW_PROMISES:?Set FLOW_PROMISES to the flow-promises skill folder}
: "${TLA2TOOLS:?Set TLA2TOOLS to tla2tools.jar}" "${JAVA:?Set JAVA to a Java 21+ binary}"
PATH=$(dirname "$JAVA"):$PATH
export PATH


work=$(mktemp -d "${TMPDIR:-/tmp}/merge.XXXXXX")
quint compile --target tlaplus --main enterprise_seat enterprise_seat.qnt >"$work/enterprise_seat.tla" 2>/dev/null
jar=$(ls -d "$HOME"/.quint/apalache-dist-*/apalache/lib/apalache.jar | tail -n 1)
for m in Apalache Variants; do unzip -p "$jar" "tla2sany/StandardModules/$m.tla" >"$work/$m.tla"; done
printf "INIT q_init\nNEXT q_step\n" >"$work/enterprise_seat.cfg"
(cd "$work" && "$JAVA" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC -workers auto -deadlock \
  -dump dot,actionlabels graph -config enterprise_seat.cfg enterprise_seat.tla | grep -E "distinct states|Finished")
node --max-old-space-size=24000 merge.mts "$work/graph.dot" render.mts enterprise_seat.mmd
node "$SKILL/scripts/render.mjs" enterprise_seat.mmd enterprise_seat.svg
rm -rf "$work"
echo "Rendered enterprise_seat.svg"
