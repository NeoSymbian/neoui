# SPDX-License-Identifier: GPL-3.0-only
# Build a QEMU monitor script ($M) for NeoSymbian/build's boot.sh MONITOR, in screen pixels of SCREEN (WIDTHxHEIGHT,
# default 768x640: QEMU's absolute pointer spans 0..0x7FFF per axis). Source it, call tap/drag/swipe/expect/wait_s, then
# run boot.sh with MON_GAP=0.1 MONITOR="$M" (and the same SCREEN). Each tap waits until neoui logs that it handled it
# ("neoui: tap from Some((X, Y))"); `expect TEXT` waits for a later serial line, such as an app's reaction.
M=""
SW=${SCREEN:-768x640}; SH=${SW#*x}; SW=${SW%x*}
# the navigation mode the taps assume (Navigation v2): the keyboard sits above Buttons' 56 dp key row or the 16 dp
# gesture band; export NAV=gestures after switching
NAV=${NAV:-buttons}
kb_bottom_dp() { if [ "$NAV" = gestures ]; then echo 16; else echo 56; fi; }
add() { M="${M:+$M;}$*"; }
# +1/2 pixel, so the pointer driver's truncation lands on the pixel asked for
at() { add "mouse_move $(((2 * $1 + 1) * 16384 / SW)) $(((2 * $2 + 1) * 16384 / SH))"; }
tap() { at "$1" "$2"; add "mouse_button 1"; add "mouse_button 0"; add "expect neoui: tap from Some(($1, $2))"; }
# expect TEXT: wait until TEXT appears in the serial log after the previous expect's match (run-syborg EXPECT_MAX)
expect() { add "expect $*"; }
# swipe X1 Y1 X2 Y2: press at (X1, Y1), move to (X2, Y2) in 8 steps, release; an edge gesture (docs neoui-shell/navigation.md)
# starts within 16 dp of the edge
swipe() {
	at "$1" "$2"; add "mouse_button 1"
	for k in 1 2 3 4 5 6 7 8; do at $(($1 + ($3 - $1) * k / 8)) $(($2 + ($4 - $2) * k / 8)); done
	add "mouse_button 0"
}
# swipe_hold X1 Y1 X2 Y2: as swipe, held still for a second before the release (the Thumb map's up-and-hold)
swipe_hold() {
	at "$1" "$2"; add "mouse_button 1"
	for k in 1 2 3 4 5 6 7 8; do at $(($1 + ($3 - $1) * k / 8)) $(($2 + ($4 - $2) * k / 8)); done
	add "wait 1"; add "mouse_button 0"
}
# hold X Y: a long press (a second), for the kit's long-press (selection mode, menus)
hold() { at "$1" "$2"; add "mouse_button 1"; add "wait 1"; add "mouse_button 0"; add "expect neoui: tap from Some(($1, $2))"; }
# drag X Y1 Y2: a vertical swipe
drag() { swipe "$1" "$2" "$1" "$3"; }
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
# kb_type WORD: tap WORD (lowercase letters and spaces) on the system keyboard in English, then its Enter key. The
# keyboard is open above the key row or the band (NAV); key geometry mirrors ui/components/keyboard/keyboard.slint,
# counted up from the keyboard's bottom (row k centred 27 + 48 k dp above it, whatever rows are above), as
# crates/neoui-render/tests/keyboard.rs does. The bottom row is 10 units: space spans 3.5-7.5 with the globe key.
kb_key() {
	awk -v c="$1" -v W="$SW" -v H="$SH" -v B="$(kb_bottom_dp)" 'BEGIN {
		d = (W < H ? W : H) / 360; bottom = H - B * d; u = W / 10; w = 1
		if ((i = index("qwertyuiop", c))) { row = 3; at = i - 1 }
		else if ((i = index("asdfghjkl", c))) { row = 2; at = i - 0.5 }
		else if ((i = index("zxcvbnm", c))) { row = 1; at = i + 0.5 }
		else if (c == " ") { row = 0; at = 3.5; w = 4 }
		else if (c == "GLOBE") { row = 0; at = 1.5 }
		else if (c == "ENTER") { row = 0; at = 8.5; w = 1.5 }
		else exit 1
		printf "%d %d\n", (at + w / 2) * u, bottom - (27 + row * 48) * d
	}'
}
# kb_tap K AT COLS: the one-unit key at AT (in key widths, COLS to a row) of row K counted up from the bottom row (0);
# for the other languages, whose letters awk can't index (Russian is 12 wide: row 2 ф..э from 0.5, row 1 shift,
# then ё я ч с м ... from 1)
kb_tap() {
	# shellcheck disable=SC2046 # "x y"
	tap $(awk -v k="$1" -v at="$2" -v cols="$3" -v W="$SW" -v H="$SH" -v B="$(kb_bottom_dp)" 'BEGIN {
		d = (W < H ? W : H) / 360
		printf "%d %d\n", (at + 0.5) * W / cols, H - (B + 27 + k * 48) * d
	}')
}
# kb_hide [term]: the keyboard's hide key, the right end of its 30 dp top band, for the English letters with the
# digit row (all languages on, as tall as the tallest: 5 rows of 48 dp + 6 dp), and the terminal's extra row
kb_hide() {
	# shellcheck disable=SC2046 # "x y"
	tap $(awk -v t="$1" -v W="$SW" -v H="$SH" -v B="$(kb_bottom_dp)" 'BEGIN {
		d = (W < H ? W : H) / 360; h = 30 + 6 + 5 * 48 + (t == "term" ? 48 : 0)
		printf "%d %d\n", W - 32 * d, H - (B + h - 15) * d
	}')
}
# kb_globe: the next language on (English -> Armenian -> Russian -> English by default)
# shellcheck disable=SC2046 # "x y"
kb_globe() { tap $(kb_key GLOBE); }
# kb_enter: the Enter key
# shellcheck disable=SC2046 # "x y"
kb_enter() { tap $(kb_key ENTER); }
kb_type() {
	w=$1
	while [ -n "$w" ]; do
		c=${w%"${w#?}"}; w=${w#?}
		# shellcheck disable=SC2046 # "x y"
		tap $(kb_key "$c")
	done
	tap $(kb_key ENTER)
}
