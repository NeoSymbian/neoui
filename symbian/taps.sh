# SPDX-License-Identifier: GPL-3.0-only
# Build a QEMU monitor script ($M) for NeoSymbian/build's boot.sh MONITOR, in screen pixels of SCREEN (WIDTHxHEIGHT,
# default 768x640: QEMU's absolute pointer spans 0..0x7FFF per axis). Source it, call tap/drag/expect/wait_s, then
# run boot.sh with MON_GAP=0.1 MONITOR="$M" (and the same SCREEN). Each tap waits until neoui logs that it handled it
# ("neoui: tap from Some((X, Y))"); `expect TEXT` waits for a later serial line, such as an app's reaction.
M=""
SW=${SCREEN:-768x640}; SH=${SW#*x}; SW=${SW%x*}
add() { M="${M:+$M;}$*"; }
# +1/2 pixel, so the pointer driver's truncation lands on the pixel asked for
at() { add "mouse_move $(((2 * $1 + 1) * 16384 / SW)) $(((2 * $2 + 1) * 16384 / SH))"; }
tap() { at "$1" "$2"; add "mouse_button 1"; add "mouse_button 0"; add "expect neoui: tap from Some(($1, $2))"; }
# expect TEXT: wait until TEXT appears in the serial log after the previous expect's match (run-syborg EXPECT_MAX)
expect() { add "expect $*"; }
# drag X Y1 Y2: press at (X, Y1), move to (X, Y2) in 8 steps, release
drag() {
	at "$1" "$2"; add "mouse_button 1"
	for k in 1 2 3 4 5 6 7 8; do at "$1" $(($2 + ($3 - $2) * k / 8)); done
	add "mouse_button 0"
}
# wait_s N: N seconds, for what logs nothing (a drag settling)
wait_s() { add "wait $1"; }
# type WORD: one key per character (letters and digits, as QEMU names them), then Enter
type_text() {
	w=$1
	while [ -n "$w" ]; do
		c=${w%"${w#?}"}; w=${w#?}
		add "sendkey $c"
	done
	add "sendkey ret"
}
# kb_type WORD: tap WORD (lowercase letters and spaces) on the system keyboard, then its Enter key. The keyboard is
# open over an app screen (toolbar 61 dp); key geometry mirrors ui/components/keyboard.slint (rows 48 dp apart from
# 6 dp below its 198 dp top, 10 units per row), as crates/neoui-render/tests/keyboard.rs does.
kb_key() {
	awk -v c="$1" -v W="$SW" -v H="$SH" 'BEGIN {
		d = (W < H ? W : H) / 360; top = H - 61 * d - 198 * d; u = W / 10; w = 1
		if ((i = index("qwertyuiop", c))) { row = 0; at = i - 1 }
		else if ((i = index("asdfghjkl", c))) { row = 1; at = i - 0.5 }
		else if ((i = index("zxcvbnm", c))) { row = 2; at = i + 0.5 }
		else if (c == " ") { row = 3; at = 2.5; w = 5 }
		else if (c == "ENTER") { row = 3; at = 8.5; w = 1.5 }
		else exit 1
		printf "%d %d\n", (at + w / 2) * u, top + (6 + row * 48 + 21) * d
	}'
}
kb_type() {
	w=$1
	while [ -n "$w" ]; do
		c=${w%"${w#?}"}; w=${w#?}
		# shellcheck disable=SC2046 # "x y"
		tap $(kb_key "$c")
	done
	tap $(kb_key ENTER)
}
