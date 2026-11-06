# SPDX-License-Identifier: GPL-3.0-only
# Build a QEMU monitor script ($M) for symbian-2010's boot.sh MONITOR, in screen pixels of SCREEN (WIDTHxHEIGHT,
# default 768x640: QEMU's absolute pointer spans 0..0x7FFF per axis). Source it, call tap/drag/wait_s, then run
# boot.sh with MON_GAP=0.3 MONITOR="$M" (and the same SCREEN).
M=""
SW=${SCREEN:-768x640}; SH=${SW#*x}; SW=${SW%x*}
add() { M="${M:+$M;}$*"; }
# +1/2 pixel, so the pointer driver's truncation lands on the pixel asked for
at() { add "mouse_move $(((2 * $1 + 1) * 16384 / SW)) $(((2 * $2 + 1) * 16384 / SH))"; }
tap() { at "$1" "$2"; add "mouse_button 1"; add "mouse_button 0"; }
# drag X Y1 Y2: press at (X, Y1), move to (X, Y2) in 8 steps, release
drag() {
	at "$1" "$2"; add "mouse_button 1"
	for k in 1 2 3 4 5 6 7 8; do at "$1" $(($2 + ($3 - $2) * k / 8)); done
	add "mouse_button 0"
}
# wait_s N: about N seconds at 0.3 s per command
wait_s() { i=0; while [ $i -lt $(($1 * 10 / 3)) ]; do add "info status"; i=$((i + 1)); done; }
