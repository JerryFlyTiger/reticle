# GNU Emacs reference: viewport / paging commands (M138)

These are the probe scripts used to derive M138's spec numbers, and the raw
output GNU Emacs 30.2 produced for each. Same purpose as `dev/gnu-auto/`:
reproduce the numbers here, don't re-derive them from this editor's own
code.

Run with the exact command each `.el` file was captured with:

```
OUT=/path/out.txt emacs -Q -l gnu-scroll-gui.el
```

`/opt/homebrew/bin/emacs` is GNU Emacs 30.2. Each `-gui*.el` script opens a
real frame sized 80x24 (`(set-frame-size (selected-frame) 80 24)`), drives
`scroll-up-command`/`scroll-down-command`/`recenter`/`recenter-top-bottom`/
`set-window-start`/`pos-visible-in-window-p` against a 100-line buffer, and
kills itself when done. `$OUT` is where it appends its trace lines; the
`.out` files here are that trace, captured verbatim.

- `gnu-scroll-gui.el` / `.out` — the main `C-v`/`M-v` sweep: repeated
  scrolls, explicit and negative counts, boundary behavior at both ends of
  the buffer, and the `C-v` loop from the top.
- `gnu-scroll-gui2.el` / `.out` — `recenter`/`recenter-top-bottom` table
  (middle/top/bottom cycling, explicit ARG, clamping near the top and
  bottom of the buffer).
- `gnu-scroll-gui3.el` / `.out` — `window-start`/`set-window-start`/
  `pos-visible-in-window-p`, plus the block-row and wrapped-line row
  counting cases.

## `gnu-scroll.el` / `.out` (batch mode)

Kept for reference, but **batch mode has no redisplay** — `(redisplay t)`
is a no-op with no frame to paint, so `window-start`/`point`/`window-end`
never move the way they do under real display code. **Its numbers are NOT
the reference for this milestone; only the three `-gui` runs above are.**
It's useful for exactly one thing: reading off the values of variables
that don't depend on redisplay at all (`next-screen-context-lines`,
`recenter-positions`, `scroll-error-top-bottom`,
`scroll-preserve-screen-position`, `scroll-margin`).

`gnu-scroll-signal.el` / `.out` (2026-09-14, batch mode on purpose): after
`(scroll-up-command 50)` signals `end-of-buffer` and after `M-v` at the top
signals `beginning-of-buffer`, window-start and point are unchanged (70/71 and
1/6). Only the `-> after signal` lines of that file are evidence; its other
lines are batch numbers.
