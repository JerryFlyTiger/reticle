# dev/vim-scroll/

Ground truth for M139's evil viewport family (`C-f`/`C-b`/`C-e`/`C-y`/`C-d`/`C-u`,
`zz`/`zt`/`zb`/`z RET`/`z .`/`z -`) — every numeric rule in
`crates/core/lisp/evil.el`'s "M139: viewport family" section is quoted from the
`.out` files here, produced by real vim (`/usr/bin/vim` here, vim 9.x), not from
memory or guesswork.

## Fixtures

- `hundred.txt` — 100 lines, `line 1` through `line 100`, each on its own line
  with a trailing newline.
- `hundred-indent.txt` — the same 100 lines, but ODD lines (1, 3, 5, ...) are
  indented by four spaces. Used wherever a rule needed to distinguish "cursor
  keeps its column" from "cursor moves to the first non-blank column" (e.g.
  `C-e`/`C-y` vs. `C-d`/`z .`).

## How the `.out` files were produced

```
OUT=vimprobe.out  script -q /dev/null vim -u NONE -N -S vimprobe.vim hundred.txt
OUT=vimprobe2.out script -q /dev/null vim -u NONE -N -S vimprobe2.vim hundred-indent.txt
```

`-u NONE -N` disables any user vimrc and plugins, so the numbers reflect vim's
own built-in defaults only (`'scroll'` starts at half the window height,
`&scrolloff` is 0, etc.) — not this machine's personal vim configuration.
`script -q /dev/null` is needed because vim's window-scrolling commands only
take effect under a real terminal (`lines`/`columns` are set inside the script
itself to pin the frame size regardless of the actual terminal window).

Each `.vim` script sets `lines=24 columns=80` (`set nomore` suppresses the
`--More--` prompt), runs a sequence of normal-mode keystrokes via `:normal`,
and after each one calls `redraw` then records `line('w0')` (window-start
line), `line('.')` (cursor line), `col('.')` (cursor column, `vimprobe2.vim`
only), and `line('w$')` (last visible line) to a `g:out` list, which
`writefile` dumps to `$OUT` at the end.

## Known caveat: two tags in `vimprobe2.vim`/`vimprobe2.out` are mislabeled

The `z<CR> indented` / `z. unindented` tags in `vimprobe2.vim` are swapped
against the actual `normal NG` calls that precede them: the script moves to
line 50 (even, UNINDENTED in `hundred-indent.txt`) before running `z<CR>`, and
to line 51 (odd, INDENTED) before running `z.` — the opposite of what the tags
say. `crates/core/tests/evil_tests.rs`'s `z_ret_z_dot_z_dash_...` test asserts
against the real per-line indentation (verified by re-reading this script, not
by trusting the tag text) rather than the mislabeled names. The raw `col`
values in `vimprobe2.out` are unaffected — only the English tag string next to
each row is wrong.

## 23 text rows

Every number in `crates/core/lisp/evil.el`'s doc comments assumes `h = 23`
(`window-text-height`), matching `lines=24` here: one row for vim's own
command/status line, 23 for the buffer text — the same accounting this
editor's own frame uses (`text_rows = rows - 2`: one row for the echo area,
one for the single window's own mode line — see `viewport_tests.rs`'s file
header). `crates/core/tests/evil_tests.rs`'s `setup_scroll` uses frame
`(80, 25)` to land on the same `h = 23`.

`vimprobe3.vim` / `.out` (2026-09-14): the two boundary rules re-measured so
the M139 record's claim has an artifact — `C-f` from w0 60/70/78/79/80 (81, 91,
100, 100, 100: the jump to the last line happens once the old bottom row already
shows line 100) and `C-d` from cursor 70/80/85/90 with the default `'scroll'`
(w0 70, 78, 78, 78 — the start never passes `100 − 22 = 78` while the cursor
keeps moving: 81, 91, 96, 100).
