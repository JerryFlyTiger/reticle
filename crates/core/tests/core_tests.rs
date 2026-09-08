use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn grid_row(interp: &Interp, ed: &Rc<RefCell<Editor>>, row: usize) -> String {
    let grid = core::redisplay::render(interp, ed);
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// A scratch directory that deletes itself on drop.
///
/// The old shape put `std::fs::remove_dir_all` as the LAST line of each
/// test body -- exactly the line a panicking test never reaches, so
/// cleanup ran on success and leaked on failure, backwards from what you
/// want. By 2026-08-14 that had left 298 stale directories under
/// $TMPDIR, the oldest three days old. `Drop` runs during unwind too.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn editing_primitives() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(progn (insert \"hello world\") (buffer-string))"),
        "\"hello world\""
    );
    assert_eq!(run(&mut i, "(point)"), "12");
    assert_eq!(run(&mut i, "(point-max)"), "12");
    assert_eq!(run(&mut i, "(progn (goto-char 1) (point))"), "1");
    assert_eq!(run(&mut i, "(progn (forward-char 5) (point))"), "6");
    assert_eq!(run(&mut i, "(char-after)"), "32"); // space
    assert_eq!(
        run(&mut i, "(progn (delete-region 6 12) (buffer-string))"),
        "\"hello\""
    );
    assert_eq!(run(&mut i, "(progn (end-of-line) (point))"), "6");
    assert_eq!(run(&mut i, "(bolp)"), "nil");
    assert_eq!(run(&mut i, "(eobp)"), "t");
}

#[test]
fn motion_and_lines() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"one\\ntwo\\nthree\")");
    assert_eq!(
        run(&mut i, "(progn (goto-char 1) (forward-line 1) (point))"),
        "5"
    );
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");
    assert_eq!(run(&mut i, "(progn (end-of-line) (point))"), "8");
    assert_eq!(run(&mut i, "(progn (next-line) (point))"), "12");
    assert_eq!(
        run(&mut i, "(progn (previous-line) (previous-line) (point))"),
        "4"
    );
    assert_eq!(run(&mut i, "(current-column)"), "3");
    assert_eq!(run(&mut i, "(line-beginning-position)"), "1");
    assert_eq!(run(&mut i, "(line-end-position)"), "4");
}

#[test]
fn insert_chinese() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(
            &mut i,
            "(progn (insert \"中文\" ?字 \"abc\") (buffer-string))"
        ),
        "\"中文字abc\""
    );
    assert_eq!(run(&mut i, "(point-max)"), "7");
}

#[test]
fn typing_through_keymap() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "h e l l o SPC t h e r e").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello there\"");
    feed_keys(&mut i, &ed, "RET w o w").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello there\\nwow\"");
    // DEL deletes backward.
    feed_keys(&mut i, &ed, "DEL DEL").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello there\\nw\"");
    // C-a then C-k kills the line text.
    feed_keys(&mut i, &ed, "C-a C-k").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello there\\n\"");
    // C-y yanks it back.
    feed_keys(&mut i, &ed, "C-y").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello there\\nw\"");
}

// `kill-whole-line` (M110) is not bound to any key (see the M110 report:
// neither frontend's key layer can express `C-S-backspace`, the GNU
// binding, so there is nothing to bind it to), so most of these tests
// call it directly via `(kill-whole-line)`. The two tests that need
// `last-command` to reflect a real command cycle (append-on-repeat, and
// an intervening command breaking that) bind it to an unused key
// (`<f24>`, not bound anywhere in `simple.el`) for the duration of the
// test and drive it through `feed_keys`, so `last_command` updates the
// same way it would for any other command.
#[test]
fn kill_whole_line_middle_of_buffer() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nccc\\n\"");
    assert_eq!(run(&mut i, "(point)"), "5"); // now the start of "ccc"
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb\n");
    run(&mut i, "(yank)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nbbb\\nccc\\n\"");
}

#[test]
fn kill_whole_line_point_mid_line() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 7)"); // middle of "bbb", not its start
    run(&mut i, "(kill-whole-line)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb\n");
}

#[test]
fn kill_whole_line_last_line_no_trailing_newline() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\")"); // no trailing newline
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(kill-whole-line)");
    // Verified against real GNU Emacs 30.2: with no trailing newline of
    // its own to take, "bbb" is killed alone and the newline before it is
    // left in place. The well-known consequence (real GNU behavior, not a
    // bug) is that a file lacking a final newline gains one this way.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb");
    run(&mut i, "(yank)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nbbb\"");
}

#[test]
fn kill_whole_line_point_max_with_trailing_newline_is_noop() {
    // GNU Emacs 30.2 signals `end-of-buffer` and changes nothing when
    // point sits on the buffer's implicit trailing empty line. This
    // editor's `kill-line` handles its own equivalent boundary case by
    // silently doing nothing rather than signaling (see the `start ==
    // end` early return above); `kill-whole-line` follows that local
    // convention instead of introducing a new error signal.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\n\")");
    run(&mut i, "(goto-char (point-max))");
    let before = ed.borrow().kill_ring.len();
    assert_eq!(run(&mut i, "(kill-whole-line)"), "nil");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\n\"");
    assert_eq!(
        ed.borrow().kill_ring.len(),
        before,
        "must not push an entry onto the kill ring"
    );
}

#[test]
fn kill_whole_line_single_newline_buffer_at_point_max_is_noop() {
    // Same boundary case as above, but the buffer is nothing but the
    // newline itself -- the defect-1 repro that lost the whole buffer.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"\\n\")");
    run(&mut i, "(goto-char (point-max))");
    let before = ed.borrow().kill_ring.len();
    assert_eq!(run(&mut i, "(kill-whole-line)"), "nil");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\\n\"");
    assert_eq!(
        ed.borrow().kill_ring.len(),
        before,
        "must not push an entry onto the kill ring"
    );
}

#[test]
fn kill_whole_line_count_overshoots_remaining_lines_from_non_first_line() {
    // Verified against real GNU Emacs 30.2: from "bbb" with only one real
    // line left, `(kill-whole-line 2)` kills just that line, clamped at
    // the buffer's end rather than erroring or reaching past it.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line 2)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb\n");
}

#[test]
fn kill_whole_line_count_zero_excludes_trailing_newline() {
    // Verified against real GNU Emacs 30.2: `(kill-whole-line 0)` kills
    // the current line's content but leaves its trailing newline in
    // place, unlike a positive count.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line 0)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\n\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb");
}

#[test]
fn kill_whole_line_negative_count_kills_backward_with_preceding_newline() {
    // Verified against real GNU Emacs 30.2: `(kill-whole-line -1)` from
    // line 2 of "aaa\nbbb\nccc\n" kills "bbb" plus the newline that
    // precedes it (but not "bbb"'s own trailing newline), giving
    // "aaa\nccc\n" with "\nbbb" on the kill ring.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line -1)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "\nbbb");
}

#[test]
fn kill_whole_line_negative_count_overshoots_past_first_line() {
    // Verified against real GNU Emacs 30.2: from line 2 of
    // "aaa\nbbb\nccc\n", `(kill-whole-line -5)` clamps at the buffer's
    // start rather than erroring, killing "aaa\nbbb" and leaving
    // "\nccc\n" ("bbb"'s own trailing newline survives).
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line -5)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "aaa\nbbb");
}

#[test]
fn kill_whole_line_negative_count_large_valid_overshoots_past_first_line() {
    // A large-but-valid negative count (safely negatable) should behave
    // exactly like the smaller overshoot case above: clamp at the
    // buffer's start, no error.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line -9223372036854775807)"); // i64::MIN + 1
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "aaa\nbbb");
}

#[test]
fn kill_whole_line_negative_count_i64_min_does_not_panic() {
    // Defect: the negative branch computed `(-n) as usize`, which
    // overflows when n == i64::MIN (there is no positive i64 for
    // -i64::MIN). Debug builds have overflow-checks on, so this used to
    // panic ("attempt to negate with overflow") instead of clamping like
    // any other large backward overshoot. `i64::MIN` reaches here
    // directly from user elisp: `(kill-whole-line -9223372036854775808)`.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line -9223372036854775808)"); // i64::MIN
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "aaa\nbbb");
}

#[test]
fn kill_whole_line_refuses_on_read_only_buffer() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\n\")");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(set-buffer-read-only t)");
    let before_buf = run(&mut i, "(buffer-string)");
    let before_kill_len = ed.borrow().kill_ring.len();
    let out = run(&mut i, "(kill-whole-line)");
    assert!(
        out.starts_with("ERROR"),
        "kill-whole-line on a read-only buffer must be refused: {}",
        out
    );
    assert!(
        out.contains("read-only") || out.contains("Read-only"),
        "error must mention read-only: {}",
        out
    );
    run(&mut i, "(set-buffer-read-only nil)"); // buffer-string itself never needs write access, but be tidy
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        before_buf,
        "a refused kill-whole-line must leave the buffer untouched"
    );
    assert_eq!(
        ed.borrow().kill_ring.len(),
        before_kill_len,
        "a refused kill-whole-line must leave the kill ring untouched"
    );
}

#[test]
fn kill_whole_line_last_line_with_trailing_newline() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\n\")");
    run(&mut i, "(goto-char 6)"); // inside "bbb", before its newline
    run(&mut i, "(kill-whole-line)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb\n");
    run(&mut i, "(yank)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nbbb\\n\"");
}

#[test]
fn kill_whole_line_empty_line_in_middle() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\n\\nccc\\n\")");
    run(&mut i, "(goto-char 5)"); // the empty line between "aaa" and "ccc"
    run(&mut i, "(kill-whole-line)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nccc\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "\n");
}

#[test]
fn kill_whole_line_nothing_to_kill_does_not_error() {
    let (mut i, ed) = setup();
    // A wholly empty buffer: no text, no newline, nothing before it --
    // there is truly nothing to take.
    let before = ed.borrow().kill_ring.len();
    assert_eq!(run(&mut i, "(kill-whole-line)"), "nil");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
    assert_eq!(
        ed.borrow().kill_ring.len(),
        before,
        "nothing to kill must not push an entry onto the kill ring"
    );
}

#[test]
fn kill_whole_line_consecutive_calls_append_one_kill_ring_entry() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(global-set-key \"<f24>\" 'kill-whole-line)");
    feed_keys(&mut i, &ed, "<f24>").unwrap(); // kills "aaa\n"
    feed_keys(&mut i, &ed, "<f24>").unwrap(); // kills "bbb\n", should append
    assert_eq!(run(&mut i, "(buffer-string)"), "\"ccc\\n\"");
    assert_eq!(
        ed.borrow().kill_ring.len(),
        1,
        "two consecutive kill-whole-lines must land as one kill-ring entry"
    );
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "aaa\nbbb\n");
    run(&mut i, "(yank)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nbbb\\nccc\\n\"");
}

#[test]
fn kill_whole_line_intervening_command_prevents_append() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\n\")");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(global-set-key \"<f24>\" 'kill-whole-line)");
    feed_keys(&mut i, &ed, "<f24>").unwrap(); // kills "aaa\n"
    feed_keys(&mut i, &ed, "C-f").unwrap(); // an unrelated command
    feed_keys(&mut i, &ed, "<f24>").unwrap(); // kills "bbb\n", must NOT append
    assert_eq!(run(&mut i, "(buffer-string)"), "\"ccc\\n\"");
    assert_eq!(
        ed.borrow().kill_ring.len(),
        2,
        "an intervening non-kill command must break the append chain"
    );
    // Contents too, in order -- a mutation that got the right count but
    // the wrong order or wrong text in either entry would survive the
    // length-only assertion above.
    assert_eq!(ed.borrow().kill_ring[0], "aaa\n");
    assert_eq!(ed.borrow().kill_ring[1], "bbb\n");
}

#[test]
fn kill_whole_line_positive_count_multiline_uses_running_position() {
    // Coverage gap: every existing n>=1 multi-count test runs out of real
    // lines on the second iteration, so the clamp masks whether the loop
    // threads `pos` across iterations or reuses a fixed `bol`. Here there
    // are enough lines that it can't be masked. Verified against real GNU
    // Emacs 30.2 (`emacs -Q --batch --eval '(progn (insert
    // "aaa\nbbb\nccc\nddd\n") (goto-char (point-min)) (forward-line 1)
    // (kill-whole-line 2) (princ (format "buf=%S kills=%S\n"
    // (buffer-string) kill-ring)))'`) -> buf="aaa\nddd\n"
    // kills=("bbb\nccc\n"). If the loop used a fixed `bol` instead of the
    // running `pos`, this would kill only "bbb\n".
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\nddd\\n\")");
    run(&mut i, "(goto-char 5)"); // start of "bbb"
    run(&mut i, "(kill-whole-line 2)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\nddd\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "bbb\nccc\n");
}

#[test]
fn kill_whole_line_negative_count_multiline_uses_running_position() {
    // Same coverage gap as above, for the n<0 loop. Verified against real
    // GNU Emacs 30.2 (`emacs -Q --batch --eval '(progn (insert
    // "aaa\nbbb\nccc\nddd\n") (goto-char (point-min)) (forward-line 3)
    // (kill-whole-line -3) (princ (format "buf=%S kills=%S\n"
    // (buffer-string) kill-ring)))'`) -> buf="aaa\n"
    // kills=("\nbbb\nccc\nddd").
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\\nddd\\n\")");
    run(&mut i, "(goto-char 13)"); // start of "ddd"
    run(&mut i, "(kill-whole-line -3)");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"aaa\\n\"");
    assert_eq!(ed.borrow().kill_ring.last().unwrap(), "\nbbb\nccc\nddd");
}

#[test]
fn kill_and_yank_region() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"abcdefgh\")");
    // Mark at 3, point at 6, C-w kills "cde".
    run(
        &mut i,
        "(progn (goto-char 3) (set-mark (point)) (goto-char 6))",
    );
    feed_keys(&mut i, &ed, "C-w").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abfgh\"");
    run(&mut i, "(goto-char 6)");
    feed_keys(&mut i, &ed, "C-y").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abfghcde\"");
}

// `feed_keys` goes straight through `keymap::parse_kbd`, so it never
// exercises the crossterm-event-to-`Key` conversion a real TUI session
// uses (`crates/frontend-tui/src/lib.rs`'s `convert_key`) — that layer
// is why `C-/` was unreachable from an actual terminal for a while
// (M64) despite this test passing the whole time. Coverage for
// `convert_key` itself lives in that file's `convert_key_tests`.
#[test]
fn undo_redo() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "a b c").unwrap();
    run(&mut i, "(undo-boundary)");
    feed_keys(&mut i, &ed, "d e f").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abcdef\"");
    feed_keys(&mut i, &ed, "C-/").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abc\"");
    feed_keys(&mut i, &ed, "C-/").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
}

#[test]
fn undo_redo_via_c_underscore() {
    // Same as `undo_redo`, but via `"C-_"` — the binding that actually
    // fires in a terminal, since both physical `Ctrl+/` and `Ctrl+_`
    // arrive as byte 0x1F there (see `simple.el`'s undo binding comment
    // and `convert_key` in `crates/frontend-tui/src/lib.rs`).
    //
    // NOTE: this only proves the `"C-_"` binding triggers undo — it is
    // NOT end-to-end coverage of the M64 decode fix. `feed_keys` goes
    // through `keymap::parse_kbd`, and `"C-_"` was already bound before
    // M64, so this test passes even with `convert_key`'s C0 remap
    // reverted. The actual coverage for that fix is
    // `frontend_tui::convert_key_tests` in `crates/frontend-tui/src/lib.rs`.
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "a b c").unwrap();
    run(&mut i, "(undo-boundary)");
    feed_keys(&mut i, &ed, "d e f").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abcdef\"");
    feed_keys(&mut i, &ed, "C-_").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abc\"");
    feed_keys(&mut i, &ed, "C-_").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
}

#[test]
fn buffers_and_locals() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");
    run(&mut i, "(defvar my-setting 'global-val)");
    run(&mut i, "(get-buffer-create \"other\")");
    run(&mut i, "(set-buffer \"other\")");
    run(&mut i, "(setq-local my-setting 'local-val)");
    assert_eq!(run(&mut i, "my-setting"), "local-val");
    run(&mut i, "(set-buffer \"*scratch*\")");
    assert_eq!(run(&mut i, "my-setting"), "global-val");
    run(&mut i, "(set-buffer \"other\")");
    assert_eq!(run(&mut i, "my-setting"), "local-val");
    assert_eq!(
        run(
            &mut i,
            "(buffer-local-value 'my-setting (get-buffer \"*scratch*\"))"
        ),
        "global-val"
    );
}

#[test]
fn excursions() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"hello\")");
    assert_eq!(
        run(
            &mut i,
            "(progn (save-excursion (goto-char 1) (insert \">> \")) (point))"
        ),
        "9"
    );
    assert_eq!(run(&mut i, "(buffer-string)"), "\">> hello\"");
    run(&mut i, "(get-buffer-create \"tmp\")");
    assert_eq!(
        run(
            &mut i,
            "(progn (with-current-buffer \"tmp\" (insert \"in-tmp\")) (buffer-name))"
        ),
        "\"*scratch*\""
    );
    assert_eq!(
        run(&mut i, "(with-current-buffer \"tmp\" (buffer-string))"),
        "\"in-tmp\""
    );
}

#[test]
fn markers_adjust() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(setq m (copy-marker 7))");
    run(&mut i, "(progn (goto-char 1) (insert \"XX\"))");
    assert_eq!(run(&mut i, "(marker-position m)"), "9");
    run(&mut i, "(delete-region 1 4)");
    assert_eq!(run(&mut i, "(marker-position m)"), "6");
}

#[test]
fn overlays_and_invisible_rendering() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (30, 6);
    run(
        &mut i,
        "(insert \"* Heading\\nbody line one\\nbody two\\n* Next\\n\")",
    );
    run(&mut i, "(goto-char (point-min))");
    assert_eq!(grid_row(&i, &ed, 0), "* Heading");
    assert_eq!(grid_row(&i, &ed, 1), "body line one");
    // Hide the body like org folding would.
    run(&mut i, "(setq ov (make-overlay 10 33))");
    run(&mut i, "(overlay-put ov 'invisible t)");
    assert_eq!(grid_row(&i, &ed, 0), "* Heading...");
    assert_eq!(grid_row(&i, &ed, 1), "* Next");
    run(&mut i, "(delete-overlay ov)");
    assert_eq!(grid_row(&i, &ed, 1), "body line one");
}

#[test]
fn chinese_width_rendering() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (20, 5);
    run(&mut i, "(insert \"中文abc\")");
    let grid = core::redisplay::render(&i, &ed);
    assert_eq!(grid.lines[0][0].ch, '中');
    assert!(grid.lines[0][1].continuation);
    assert_eq!(grid.lines[0][2].ch, '文');
    assert_eq!(grid.lines[0][4].ch, 'a');
    // Cursor after "中文abc" = col 7 (2+2+3).
    assert_eq!(grid.cursor, (0, 7));
}

#[test]
fn modeline_and_echo() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 6);
    run(&mut i, "(insert \"x\")");
    let mode = grid_row(&i, &ed, 4);
    assert!(mode.contains("*scratch*"), "modeline: {}", mode);
    assert!(mode.contains("L1"), "modeline: {}", mode);
    run(&mut i, "(message \"hi there\")");
    assert_eq!(grid_row(&i, &ed, 5), "hi there");
}

/// P1.2 #4: `ensure_point_visible` recenters instead of walking every
/// intervening row when point lands far from `window_start` (M-> / M-<
/// / a large `goto-char`). These tests build a buffer big enough that
/// the old row-by-row scan would be O(distance) but small enough to run
/// fast in the default test suite; `line_number_perf_tests.rs` covers
/// the 10MB/200k-line perf bar separately.
fn make_numbered_lines(n: usize) -> String {
    let mut s = String::with_capacity(n * 10);
    for i in 0..n {
        s.push_str(&format!("line {:04}\n", i));
    }
    s
}

// NOTE: `render_window`'s own cursor placement (redisplay.rs) clamps the
// hardware cursor into the last visible row whenever point turns out to
// be genuinely off-screen (so the terminal cursor has *somewhere* sane
// to sit). That means a bare `grid.cursor.0 < text_rows` assertion would
// pass even if `ensure_point_visible` were completely broken — the
// clamp would quietly paper over it. Every test below therefore also
// checks the *content* actually rendered at the cursor position against
// what we independently know point's true target is.

#[test]
fn large_downward_jump_recenters_point_into_view() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (20, 12);
    run(&mut i, &format!("(insert {:?})", make_numbered_lines(2000)));
    run(&mut i, "(goto-char (point-min))");
    let _ = core::redisplay::render(&i, &ed); // settle window_start at the top
                                              // A big downward jump (like M->), landing exactly on a known line.
    run(
        &mut i,
        "(progn (goto-char (point-min)) (forward-line 1500))",
    );
    let grid = core::redisplay::render(&i, &ed);
    let text_rows = 12 - 2; // frame rows, minus the window's modeline and the echo row
    assert!(
        grid.cursor.0 < text_rows,
        "point off-screen: cursor row {}",
        grid.cursor.0
    );
    // Recentered (point lands mid-window), not just scrolled until point
    // sits on the top row.
    assert!(
        grid.cursor.0 > 0,
        "expected point recentered mid-window, got row {}",
        grid.cursor.0
    );
    // The row under the cursor really is line 1500's text, not whatever
    // the fallback clamp happened to land on.
    assert_eq!(grid_row(&i, &ed, grid.cursor.0), "line 1500");
}

#[test]
fn large_upward_jump_recenters_point_into_view() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (20, 12);
    run(&mut i, &format!("(insert {:?})", make_numbered_lines(2000)));
    // Point is already at the end after the insert; settle window_start
    // there before the jump.
    let _ = core::redisplay::render(&i, &ed);
    // Jump far back up, but not all the way to point-min, so the
    // recenter path is exercised rather than the point-min edge case.
    run(
        &mut i,
        "(progn (goto-char (point-min)) (forward-line 1000))",
    );
    let grid = core::redisplay::render(&i, &ed);
    let text_rows = 12 - 2;
    assert!(
        grid.cursor.0 < text_rows,
        "point off-screen: cursor row {}",
        grid.cursor.0
    );
    assert!(
        grid.cursor.0 > 0,
        "expected point recentered mid-window, got row {}",
        grid.cursor.0
    );
    assert_eq!(grid_row(&i, &ed, grid.cursor.0), "line 1000");
}

#[test]
fn large_jump_into_folded_region_stays_visible_without_panic() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (20, 12);
    run(&mut i, &format!("(insert {:?})", make_numbered_lines(2000)));
    // Fold lines [100, 400) the way org's TAB-hide-subtree would.
    run(&mut i, "(progn (goto-char (point-min)) (forward-line 100))");
    run(&mut i, "(setq fold-start (point))");
    run(&mut i, "(progn (goto-char (point-min)) (forward-line 400))");
    run(&mut i, "(setq fold-end (point))");
    run(&mut i, "(setq ov (make-overlay fold-start fold-end))");
    run(&mut i, "(overlay-put ov 'invisible t)");
    run(&mut i, "(goto-char (point-min))");
    let _ = core::redisplay::render(&i, &ed); // settle window_start at the top
                                              // Jump point just past the fold start: the recenter target (roughly
                                              // half a window of logical lines back from point) lands inside the
                                              // invisible range itself, which must render (as "...") rather than
                                              // panic.
    run(&mut i, "(progn (goto-char (point-min)) (forward-line 105))");
    let grid = core::redisplay::render(&i, &ed); // must not panic
    let text_rows = 12 - 2;
    assert!(
        grid.cursor.0 < text_rows,
        "point off-screen: cursor row {}",
        grid.cursor.0
    );
    // Point sits inside the folded (invisible) range, so it's shown at
    // the "..." indicator — confirm that's actually what's under the
    // cursor, not just that some row was clamped to.
    let row_text = grid_row(&i, &ed, grid.cursor.0);
    assert!(
        row_text.contains("..."),
        "expected the fold indicator under the cursor, got: {row_text}"
    );
}

/// Regression test for a reviewer-caught bug in the first cut of this
/// fix: `recenter`'s correction pass used to give up after a small,
/// fixed step budget and fall back to plain `line_start(point)`. That
/// puts window_start at the right *line*, but if point sits deep inside
/// one enormous wrapped logical line (minified JS/CSS, a giant log
/// line, ...), point itself can still be hundreds of visual rows further
/// down and completely off-screen — `line_start(point)` alone doesn't
/// fix that. `exact_rows_before_point` now replaces that unsafe
/// fallback with an exact (if no-longer-O(1)) computation.
#[test]
fn large_jump_into_a_single_giant_wrapped_line_stays_visible() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (20, 12);
    let mut s = make_numbered_lines(50);
    // One ~20000-char line with a unique marker 15000 characters in —
    // deep enough that it wraps across hundreds of rows at cols=20.
    s.push_str(&"a".repeat(15_000));
    s.push('Z');
    s.push_str(&"b".repeat(5_000));
    s.push('\n');
    run(&mut i, &format!("(insert {:?})", s));

    run(&mut i, "(goto-char (point-min))");
    let _ = core::redisplay::render(&i, &ed); // settle window_start at the top
                                              // Land point exactly on the marker, deep inside the giant line.
    run(
        &mut i,
        "(progn (goto-char (point-min)) (search-forward \"Z\") (backward-char 1))",
    );
    let grid = core::redisplay::render(&i, &ed);

    let (row, col) = grid.cursor;
    assert!(row < grid.rows, "cursor row {row} out of the grid entirely");
    assert_eq!(
        grid.lines[row][col].ch, 'Z',
        "point's marker character isn't under the cursor at ({row}, {col}) — point is off-screen"
    );
}

#[test]
fn minibuffer_find_file() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.txt");
    std::fs::write(&path, "file contents here").unwrap();

    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "minibuffer should be active"
    );
    for c in path.to_str().unwrap().chars() {
        core::commands::handle_key(&mut i, &ed, core::commands::Key::Char(c as i64));
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_none());
    assert_eq!(run(&mut i, "(buffer-string)"), "\"file contents here\"");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"test.txt\"");

    // Edit and save with C-x C-s.
    feed_keys(&mut i, &ed, "M-> X").unwrap();
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "file contents hereX"
    );
}

#[test]
fn meta_x_command() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"abc\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    for c in "beginning-of-buffer".chars() {
        core::commands::handle_key(&mut i, &ed, core::commands::Key::Char(c as i64));
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(point)"), "1");
}

#[test]
fn user_defined_command_and_binding() {
    let (mut i, ed) = setup();
    // A user config defining a new command and binding it — the M4 core path.
    run(
        &mut i,
        "(progn
           (defun insert-date ()
             (interactive)
             (insert \"2026-07-20\"))
           (global-set-key \"C-c d\" 'insert-date))",
    );
    feed_keys(&mut i, &ed, "C-c d").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"2026-07-20\"");
}

#[test]
fn search() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"foo bar baz bar\")");
    run(&mut i, "(goto-char 1)");
    assert_eq!(run(&mut i, "(search-forward \"bar\")"), "8");
    assert_eq!(run(&mut i, "(search-forward \"bar\")"), "16");
    assert_eq!(run(&mut i, "(search-forward \"bar\" nil t)"), "nil");
    assert_eq!(run(&mut i, "(search-backward \"foo\")"), "1");
}

#[test]
fn init_file_loading() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("init");
    std::fs::create_dir_all(&dir).unwrap();
    // An extension library on load-path...
    std::fs::write(
        dir.join("my-ext.el"),
        ";;; -*- lexical-binding: t -*-\n\
         (defun my-ext-hello () (concat \"hello from ext\"))\n\
         (provide 'my-ext)\n",
    )
    .unwrap();
    // ...loaded from the init file, which also defines a command + binding.
    std::fs::write(
        dir.join("init.el"),
        ";;; -*- lexical-binding: t -*-\n\
         (require 'my-ext)\n\
         (setq my-init-ran t)\n\
         (defun my-cmd () (interactive) (insert (my-ext-hello)))\n\
         (global-set-key \"C-c h\" 'my-cmd)\n",
    )
    .unwrap();

    core::load_init_from(&mut i, &ed, dir.to_str().unwrap());
    assert_eq!(run(&mut i, "my-init-ran"), "t");
    assert_eq!(run(&mut i, "(featurep 'my-ext)"), "t");
    feed_keys(&mut i, &ed, "C-c h").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello from ext\"");
    assert!(run(&mut i, "user-init-file").contains("init.el"));

    // A broken init file must not crash startup — error goes to echo area.
    std::fs::write(dir.join("init.el"), "(this-function-does-not-exist)\n").unwrap();
    let (mut i2, ed2) = setup();
    core::load_init_from(&mut i2, &ed2, dir.to_str().unwrap());
    let echo = ed2.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Error loading init file"), "echo: {}", echo);
}

#[test]
fn window_split_and_navigate() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 12);
    run(&mut i, "(insert \"buffer-one-text\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");

    // Both windows show the same buffer right after splitting.
    let top = grid_row(&i, &ed, 0);
    assert!(top.contains("buffer-one-text"), "top: {}", top);

    // Switch to the other window and show a different buffer there.
    feed_keys(&mut i, &ed, "C-x o").unwrap();
    run(&mut i, "(switch-to-buffer \"second\")");
    run(&mut i, "(insert \"second-buffer-text\")");

    // The first window must still show buffer-one untouched.
    feed_keys(&mut i, &ed, "C-x o").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"buffer-one-text\"");

    feed_keys(&mut i, &ed, "C-x 1").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "1");
}

// =======================================================================
// M102: split ratios, resize commands, minimum-size protection
// =======================================================================

#[test]
fn window_split_default_sizes_match_pre_m102_halves() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40, splits evenly
    run(&mut i, "(split-window-below)");
    assert_eq!(run(&mut i, "(window-height 0)"), "20");
    assert_eq!(run(&mut i, "(window-height 1)"), "20");
}

#[test]
fn window_split_below_honors_size_argument() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below 10)");
    assert_eq!(run(&mut i, "(window-height 0)"), "10");
    assert_eq!(run(&mut i, "(window-height 1)"), "30");
}

#[test]
fn enlarge_window_grows_selected_and_shrinks_the_other() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below)"); // 20/20, id0 (top) selected
    run(&mut i, "(enlarge-window 3)");
    assert_eq!(run(&mut i, "(window-height 0)"), "23");
    assert_eq!(run(&mut i, "(window-height 1)"), "17");
}

#[test]
fn shrink_window_shrinks_selected_and_grows_the_other() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below)"); // 20/20, id0 (top) selected
    run(&mut i, "(shrink-window 2)");
    assert_eq!(run(&mut i, "(window-height 0)"), "18");
    assert_eq!(run(&mut i, "(window-height 1)"), "22");
}

#[test]
fn enlarge_and_shrink_window_horizontally_move_the_vertical_divider() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (91, 10); // windows_height irrelevant; width avail = 90 (1-col sep)
    run(&mut i, "(split-window-right)"); // 45/45, id0 (left) selected
    assert_eq!(run(&mut i, "(window-width 0)"), "45");
    assert_eq!(run(&mut i, "(window-width 1)"), "45");

    run(&mut i, "(enlarge-window-horizontally 10)");
    assert_eq!(run(&mut i, "(window-width 0)"), "55");
    assert_eq!(run(&mut i, "(window-width 1)"), "35");

    run(&mut i, "(shrink-window-horizontally 12)");
    assert_eq!(run(&mut i, "(window-width 0)"), "43");
    assert_eq!(run(&mut i, "(window-width 1)"), "47");
}

#[test]
fn enlarge_window_clamps_the_other_side_to_the_minimum_height() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below)"); // 20/20, id0 (top) selected
    run(&mut i, "(enlarge-window 1000)");
    // WINDOW_MIN_HEIGHT (redisplay.rs) is 2 -- the other window must
    // land exactly there, not 0, not negative, and the call must not
    // panic (a `run` that errors would show up as "ERROR: ..." here,
    // not as a mismatched number).
    assert_eq!(run(&mut i, "(window-height 1)"), "2");
    assert_eq!(run(&mut i, "(window-height 0)"), "38");
}

#[test]
fn split_window_below_refuses_when_too_small() {
    let (mut i, ed) = setup();
    // windows_height = rows - 1 = 3, below 2*WINDOW_MIN_HEIGHT (4).
    ed.borrow_mut().frame = (40, 4);
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(
        ed.borrow().echo.clone().unwrap_or_default(),
        "Window too small to split"
    );
}

#[test]
fn resizing_a_nested_split_does_not_disturb_the_outer_boundary() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (90, 41); // windows_height = 40, width avail = 89
    run(&mut i, "(split-window-below)"); // id0 (top, selected) / id1 (bottom): 20/20
    run(&mut i, "(split-window-right)"); // inside id0: id0 (left, selected) / id2 (right): 44/45
    assert_eq!(run(&mut i, "(window-width 0)"), "44");
    assert_eq!(run(&mut i, "(window-width 2)"), "45");

    run(&mut i, "(enlarge-window-horizontally 10)");
    // The outer (vertical) split's boundary must be untouched: the
    // bottom window's height stays exactly what it was before the
    // nested horizontal resize.
    assert_eq!(run(&mut i, "(window-height 1)"), "20");
    assert_eq!(run(&mut i, "(window-height 0)"), "20");
    // The nested split itself did move.
    assert_eq!(run(&mut i, "(window-width 0)"), "54");
    assert_eq!(run(&mut i, "(window-width 2)"), "35");
}

#[test]
fn balance_windows_resets_a_nested_asymmetric_layout() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (90, 41); // windows_height = 40, width avail = 89
    run(&mut i, "(split-window-below)"); // id0 (top) / id1 (bottom): 20/20
    run(&mut i, "(split-window-right)"); // inside id0: id0 (left) / id2 (right): 44/45
    run(&mut i, "(enlarge-window 5)"); // outer split: id0 grows to 25, id1 shrinks to 15
    run(&mut i, "(enlarge-window-horizontally 10)"); // nested split: id0 to 54/id2 35
    assert_eq!(run(&mut i, "(window-height 0)"), "25");
    assert_eq!(run(&mut i, "(window-height 1)"), "15");
    assert_eq!(run(&mut i, "(window-width 0)"), "54");
    assert_eq!(run(&mut i, "(window-width 2)"), "35");

    run(&mut i, "(balance-windows)");
    assert_eq!(run(&mut i, "(window-height 0)"), "20");
    assert_eq!(run(&mut i, "(window-height 1)"), "20");
    assert_eq!(run(&mut i, "(window-width 0)"), "44");
    assert_eq!(run(&mut i, "(window-width 2)"), "45");
}

#[test]
fn enlarge_window_on_a_single_window_is_a_no_op_with_a_message() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41);
    run(&mut i, "(enlarge-window)"); // no split exists yet
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(
        ed.borrow().echo.clone().unwrap_or_default(),
        "Cannot resize a single window"
    );
}

#[test]
fn resizing_then_changing_frame_size_keeps_the_ratio_approximately() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below)"); // 20/20
    run(&mut i, "(enlarge-window 3)"); // 23/17 (frac = 0.575)
    assert_eq!(run(&mut i, "(window-height 0)"), "23");
    assert_eq!(run(&mut i, "(window-height 1)"), "17");

    ed.borrow_mut().frame = (40, 80); // windows_height = 79
    assert_eq!(run(&mut i, "(window-height 0)"), "45");
    assert_eq!(run(&mut i, "(window-height 1)"), "34");
    // Both windows stay at or above the minimum, and the ratio 45/79 ~=
    // 0.57 stayed close to the original 0.575 (within the rounding
    // slack the M102 spec explicitly allows).
    assert!(run(&mut i, "(window-height 0)").parse::<i64>().unwrap() >= 2);
    assert!(run(&mut i, "(window-height 1)").parse::<i64>().unwrap() >= 2);
}

#[test]
fn window_resize_key_bindings_actually_resize() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (91, 41); // windows_height = 40, width avail = 90
    run(&mut i, "(split-window-below)"); // 20/20, id0 (top) selected
    feed_keys(&mut i, &ed, "C-x ^").unwrap();
    assert_eq!(run(&mut i, "(window-height 0)"), "21");
    assert_eq!(run(&mut i, "(window-height 1)"), "19");

    run(&mut i, "(split-window-right)"); // inside id0: id0 (left)/id2 (right): 45/45
    assert_eq!(run(&mut i, "(window-width 0)"), "45");
    assert_eq!(run(&mut i, "(window-width 2)"), "45");

    // Each of these key presses carries an implicit prefix count of 1
    // (this editor has no `C-u`/prefix-argument input yet -- `p` always
    // evaluates to 1 -- see commands.rs's `process_pending`), so a
    // single `C-x }` moves the divider by exactly one column.
    feed_keys(&mut i, &ed, "C-x }").unwrap();
    assert_eq!(run(&mut i, "(window-width 0)"), "46");
    assert_eq!(run(&mut i, "(window-width 2)"), "44");

    feed_keys(&mut i, &ed, "C-x {").unwrap();
    assert_eq!(run(&mut i, "(window-width 0)"), "45");
    assert_eq!(run(&mut i, "(window-width 2)"), "45");

    feed_keys(&mut i, &ed, "C-x +").unwrap();
    assert_eq!(run(&mut i, "(window-height 0)"), "20");
    assert_eq!(run(&mut i, "(window-height 1)"), "20");
    assert_eq!(run(&mut i, "(window-width 0)"), "45");
    assert_eq!(run(&mut i, "(window-width 2)"), "45");
}

// M102 fix round (cold-review defect 1): plain `f32` truncation in
// `split_lengths` made the read-count-add-delta-write-frac round trip
// in `resize_in_layout` lossy, so repeated `enlarge-window` presses
// silently stopped growing the window well short of
// `WINDOW_MIN_HEIGHT`, on some frame sizes after as few as 2 presses.
// `split_lengths` now nudges by a small epsilon before flooring (see
// its own doc comment for why that's exact without disturbing the
// existing default 50/50 splits) -- these tests pin the round trip
// itself, across several frame sizes chosen because the reported bug
// reproduced differently (or not at all) on different ones, so no
// single size can hide a regression here.
#[test]
fn enlarge_window_never_stalls_across_frame_sizes() {
    for &(cols, rows) in &[(80, 42), (80, 24), (120, 30), (41, 41)] {
        let (mut i, ed) = setup();
        ed.borrow_mut().frame = (cols, rows);
        run(&mut i, "(split-window-below)");
        let avail = rows - 1; // no minibuffer panel open
        let min = 2; // WINDOW_MIN_HEIGHT
        let max_a = avail - min;
        let mut prev_a: i64 = run(&mut i, "(window-height 0)").parse().unwrap();
        for step in 1..=25 {
            run(&mut i, "(enlarge-window 1)");
            let a: i64 = run(&mut i, "(window-height 0)").parse().unwrap();
            let b: i64 = run(&mut i, "(window-height 1)").parse().unwrap();
            if prev_a < max_a as i64 {
                assert_eq!(
                    a,
                    prev_a + 1,
                    "frame {}x{} step {}: expected +1 growth (a was {}), got {}",
                    cols,
                    rows,
                    step,
                    prev_a,
                    a
                );
            } else {
                assert_eq!(
                    a, max_a as i64,
                    "frame {}x{} step {}: once clamped, a must hold exactly at {}",
                    cols, rows, step, max_a
                );
            }
            assert!(
                b >= min as i64,
                "frame {}x{} step {}: other side {} must never drop below WINDOW_MIN_HEIGHT",
                cols,
                rows,
                step,
                b
            );
            prev_a = a;
        }
        assert_eq!(
            prev_a, max_a as i64,
            "frame {}x{} never actually reached the clamp within 25 presses",
            cols, rows
        );
    }
}

#[test]
fn enlarge_then_shrink_window_round_trips_to_the_original_height() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below)");
    let a0 = run(&mut i, "(window-height 0)");
    let b0 = run(&mut i, "(window-height 1)");
    run(&mut i, "(enlarge-window 1)");
    run(&mut i, "(shrink-window 1)");
    assert_eq!(run(&mut i, "(window-height 0)"), a0);
    assert_eq!(run(&mut i, "(window-height 1)"), b0);
}

#[test]
fn enlarge_then_shrink_window_horizontally_round_trips_to_the_original_width() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (91, 10); // width avail = 90 (1-col sep)
    run(&mut i, "(split-window-right)");
    let a0 = run(&mut i, "(window-width 0)");
    let b0 = run(&mut i, "(window-width 1)");
    run(&mut i, "(enlarge-window-horizontally 1)");
    run(&mut i, "(shrink-window-horizontally 1)");
    assert_eq!(run(&mut i, "(window-width 0)"), a0);
    assert_eq!(run(&mut i, "(window-width 1)"), b0);
}

// M102 fix round (cold-review defect 2): `delta` comes straight from
// `need_int` with no bound -- `i64::MAX`/`i64::MIN` used to overflow
// the plain `a_len as i64 + delta` / `- delta` arithmetic in
// `resize_in_layout` and panic in a debug build (this is one). Now
// `saturating_add`/`saturating_sub`.
#[test]
fn window_resize_selected_does_not_panic_on_extreme_delta() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40
    run(&mut i, "(split-window-below)");

    let r = run(
        &mut i,
        &format!("(window-resize-selected {} nil)", i64::MAX),
    );
    assert!(
        !r.starts_with("ERROR"),
        "i64::MAX must not panic/error: {}",
        r
    );
    let a: i64 = run(&mut i, "(window-height 0)").parse().unwrap();
    let b: i64 = run(&mut i, "(window-height 1)").parse().unwrap();
    assert_eq!(b, 2, "other side must clamp to WINDOW_MIN_HEIGHT");
    assert_eq!(
        a, 38,
        "selected side must clamp to avail - WINDOW_MIN_HEIGHT"
    );

    let r = run(
        &mut i,
        &format!("(window-resize-selected {} nil)", i64::MIN),
    );
    assert!(
        !r.starts_with("ERROR"),
        "i64::MIN must not panic/error: {}",
        r
    );
    let a: i64 = run(&mut i, "(window-height 0)").parse().unwrap();
    let b: i64 = run(&mut i, "(window-height 1)").parse().unwrap();
    assert_eq!(a, 2, "selected side must clamp to WINDOW_MIN_HEIGHT");
    assert_eq!(b, 38, "other side must clamp to avail - WINDOW_MIN_HEIGHT");
}

// M102 fix round (cold-review defect 3): a SIZE outside `[min, avail -
// min]` used to be silently clamped (`.max(0)`) instead of refused.
//
// M102 fix round 3 (cold-review, message misattribution): a SIZE that
// is merely OUT OF RANGE on a window that is otherwise plenty big must
// NOT report "Window too small to split" -- that message is reserved
// for `raw < 2 * min + sep` (the window itself has no room for any
// split at all). `split-window-internal` (builtins/ui.rs) now returns
// the distinct symbol `bad-size` for this case, and `split-window--
// report` (simple.el) turns THAT into "Invalid window size".
#[test]
fn split_window_below_refuses_an_out_of_range_size_with_the_right_message() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40, avail = 40, min = 2
                                      // this frame is NOT too small -- a size-only rejection.
    run(&mut i, "(split-window-below 0)");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(
        ed.borrow().echo.clone().unwrap_or_default(),
        "Invalid window size"
    );

    ed.borrow_mut().echo = None;
    run(&mut i, "(split-window-below -5)");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(
        ed.borrow().echo.clone().unwrap_or_default(),
        "Invalid window size"
    );

    // SIZE too large for the OTHER side to keep the minimum -- also a
    // size-only rejection, not "window too small".
    ed.borrow_mut().echo = None;
    run(&mut i, "(split-window-below 1000)");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(
        ed.borrow().echo.clone().unwrap_or_default(),
        "Invalid window size"
    );
}

// M102 fix round 3 (cold-review, message misattribution): confirm the
// TWO messages are actually distinguishable -- a genuinely tiny window
// (no SIZE given at all) must still say "Window too small to split",
// not "Invalid window size".
#[test]
fn split_window_below_still_says_window_too_small_with_no_size_given() {
    let (mut i, ed) = setup();
    // windows_height = 3, below 2*WINDOW_MIN_HEIGHT (4) -- see
    // `split_window_below_refuses_when_too_small` above for the same
    // frame size.
    ed.borrow_mut().frame = (40, 4);
    run(&mut i, "(split-window-below)");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(
        ed.borrow().echo.clone().unwrap_or_default(),
        "Window too small to split"
    );
}

// M102 fix round 3 (cold-review, off-by-one coverage gap): the range
// check in `split-window-internal` is the CLOSED interval `[min, avail
// - min]`; changing either boundary's `<`/`>` to `<=`/`>=` would not
// turn any pre-existing test red. These two pin both boundaries as
// exactly-successful, not "close to the edge but still rejected".
#[test]
fn split_window_below_size_at_the_minimum_boundary_succeeds() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40, avail = 40, min = 2
    run(&mut i, "(split-window-below 2)"); // SIZE == min, exactly
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert_eq!(run(&mut i, "(window-height 0)"), "2");
    assert_eq!(run(&mut i, "(window-height 1)"), "38");
}

#[test]
fn split_window_below_size_at_the_maximum_boundary_succeeds() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 41); // windows_height = 40, avail = 40, min = 2
    run(&mut i, "(split-window-below 38)"); // SIZE == avail - min, exactly
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert_eq!(run(&mut i, "(window-height 0)"), "38");
    assert_eq!(run(&mut i, "(window-height 1)"), "2");
}

// M102 fix round 2 (mutation report): `split_lengths`' own minimum-size
// clamp (`raw_a.clamp(min, avail - min)`) is NOT redundant with
// `resize_in_layout`'s `clamp_side_len` -- that one only clamps the
// value being WRITTEN into `frac` at resize time, against the frame
// size AT THAT MOMENT. It does nothing once the frame later shrinks:
// `frac` itself doesn't change, so `split_lengths` (called fresh every
// render, from `window_rects`) is the ONLY thing standing between an
// old, skewed `frac` and a re-rendered side dropping below the minimum
// on a smaller frame. `dev/mutations/m102.py`'s `let a = raw_a;`
// mutation (dropping just this clamp) left all pre-existing M102 tests
// green because none of them ever changed the frame size AFTER
// skewing `frac` -- these three do.
#[test]
fn window_min_height_survives_a_frame_shrink_after_a_skewed_resize() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (80, 42); // windows_height = 41
    run(&mut i, "(split-window-below)");
    // Skew hard toward the top window: enlarge-window clamps against
    // WINDOW_MIN_HEIGHT at THIS frame size, leaving frac ~= 39/41 ~=
    // 0.951 baked into the split.
    run(&mut i, "(enlarge-window 1000)");
    assert_eq!(run(&mut i, "(window-height 0)"), "39");
    assert_eq!(run(&mut i, "(window-height 1)"), "2");

    // Now shrink the frame drastically WITHOUT touching the split
    // again -- `frac` is unchanged, only the frame got smaller. Both
    // sizes chosen keep `avail = rows - 1 >= 2 * WINDOW_MIN_HEIGHT` (4),
    // i.e. squarely in `split_lengths`' clamped branch, not its
    // `avail < 2 * min` degenerate fallback (covered separately below).
    for &rows in &[6usize, 5] {
        ed.borrow_mut().frame = (80, rows);
        let avail = rows - 1;
        let a: i64 = run(&mut i, "(window-height 0)").parse().unwrap();
        let b: i64 = run(&mut i, "(window-height 1)").parse().unwrap();
        assert!(
            a >= 2,
            "rows={rows}: top window height {a} below WINDOW_MIN_HEIGHT"
        );
        assert!(
            b >= 2,
            "rows={rows}: bottom window height {b} below WINDOW_MIN_HEIGHT"
        );
        assert_eq!(
            a + b,
            avail as i64,
            "rows={rows}: heights must sum to the available area ({avail})"
        );
    }
}

#[test]
fn window_min_width_survives_a_frame_shrink_after_a_skewed_resize() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (91, 10); // width avail = 90 (1-col sep)
    run(&mut i, "(split-window-right)");
    // Skew hard toward the left window: frac ~= 86/90 ~= 0.9556.
    run(&mut i, "(enlarge-window-horizontally 1000)");
    assert_eq!(run(&mut i, "(window-width 0)"), "86");
    assert_eq!(run(&mut i, "(window-width 1)"), "4");

    // Shrink the frame width without touching the split again. All
    // three keep `avail = width - sep >= 2 * WINDOW_MIN_WIDTH` (8).
    for &width in &[12usize, 10, 9] {
        ed.borrow_mut().frame = (width, 10);
        let sep = if width > 2 { 1 } else { 0 };
        let avail = width - sep;
        let a: i64 = run(&mut i, "(window-width 0)").parse().unwrap();
        let b: i64 = run(&mut i, "(window-width 1)").parse().unwrap();
        assert!(
            a >= 4,
            "width={width}: left window width {a} below WINDOW_MIN_WIDTH"
        );
        assert!(
            b >= 4,
            "width={width}: right window width {b} below WINDOW_MIN_WIDTH"
        );
        assert_eq!(
            a + b,
            avail as i64,
            "width={width}: widths must sum to the available area ({avail})"
        );
    }
}

#[test]
fn degenerate_tiny_frame_after_skewed_resize_does_not_panic() {
    // `avail < 2 * min` is `split_lengths`' explicitly-documented
    // degenerate fallback -- it deliberately does NOT enforce the
    // minimum there (a frame this tiny can't honor it on both sides at
    // all), so this only asserts survival (no panic, window count
    // unchanged), never a minimum size.
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (80, 42);
    run(&mut i, "(split-window-below)");
    run(&mut i, "(enlarge-window 1000)");

    for &rows in &[4usize, 3] {
        ed.borrow_mut().frame = (80, rows);
        let r = run(&mut i, "(window-height 0)");
        assert!(!r.starts_with("ERROR"), "rows={rows}: {r}");
        assert_eq!(run(&mut i, "(window-count)"), "2");
    }

    let (mut i2, ed2) = setup();
    ed2.borrow_mut().frame = (91, 10);
    run(&mut i2, "(split-window-right)");
    run(&mut i2, "(enlarge-window-horizontally 1000)");

    for &width in &[8usize, 6] {
        ed2.borrow_mut().frame = (width, 10);
        let r = run(&mut i2, "(window-width 0)");
        assert!(!r.starts_with("ERROR"), "width={width}: {r}");
        assert_eq!(run(&mut i2, "(window-count)"), "2");
    }
}

#[test]
fn window_side_by_side_and_delete() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (50, 10);
    run(&mut i, "(insert \"left side content\")");
    feed_keys(&mut i, &ed, "C-x 3").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    // Side-by-side: row 0 should contain the buffer text near the left
    // and a separator before the right pane's own copy.
    let row0 = grid_row(&i, &ed, 0);
    assert!(row0.contains("left side content"), "row0: {}", row0);
    feed_keys(&mut i, &ed, "C-x 0").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "1");
}

#[test]
fn typing_stays_in_selected_window_buffer() {
    // Regression: find-file / switch-to-buffer must update the SELECTED
    // WINDOW's buffer, not just "current buffer", or the next keystroke's
    // buffer-sync would silently redirect typed text into the old buffer.
    let (mut i, ed) = setup();
    let dir = Scratch::new("w");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("w.txt");
    std::fs::write(&path, "").unwrap();
    run(
        &mut i,
        &format!("(find-file-internal \"{}\")", path.display()),
    );
    feed_keys(&mut i, &ed, "h i").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hi\"");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"w.txt\"");
}

#[test]
fn incremental_search() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(insert \"the quick brown fox jumps over the lazy dog\")",
    );
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-s b r o w n").unwrap();
    assert_eq!(run(&mut i, "(point)"), "16"); // just after "brown"
                                              // RET exits, keeping the match position.
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().isearch.is_none());
    assert_eq!(run(&mut i, "(point)"), "16");

    // Repeated C-s finds the next occurrence of a repeated word.
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-s t h e").unwrap();
    let first = run(&mut i, "(point)");
    feed_keys(&mut i, &ed, "C-s").unwrap();
    let second = run(&mut i, "(point)");
    assert_ne!(first, second);

    // C-g aborts back to the search origin.
    run(&mut i, "(goto-char 1)");
    feed_keys(&mut i, &ed, "C-s x y z n o m a t c h").unwrap();
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(run(&mut i, "(point)"), "1");
}

#[test]
fn isearch_exit_redispatches_key() {
    // A key not bound in isearch (like C-a) ends the search AND still
    // executes as a normal command afterward.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-s w o r l d").unwrap();
    feed_keys(&mut i, &ed, "C-a").unwrap();
    assert!(ed.borrow().isearch.is_none());
    assert_eq!(run(&mut i, "(point)"), "1");
}

#[test]
fn byte_compiled_interactive_commands_still_work() {
    // Regression: interactive_spec() originally only read the .interactive
    // field off Function::Lambda. Byte-compiling a command (e.g. via a
    // user's init.el calling byte-compile for speed) turns it into
    // Function::Compiled, which carries the same .interactive field —
    // but the dispatcher must actually check it there too, or a compiled
    // command that needs interactive args (like kill-region's "r" spec)
    // gets called with zero arguments and silently fails arity checking.
    let (mut i, ed) = setup();
    run(&mut i, "(byte-compile 'kill-region)");
    run(&mut i, "(byte-compile 'kill-ring-save)");
    run(&mut i, "(byte-compile 'set-mark-command)");
    run(&mut i, "(byte-compile 'yank)");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1)");
    feed_keys(&mut i, &ed, "C-SPC").unwrap();
    for _ in 0..5 {
        feed_keys(&mut i, &ed, "C-f").unwrap();
    }
    feed_keys(&mut i, &ed, "C-w").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\" world\"");
    feed_keys(&mut i, &ed, "C-y").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello world\"");
}

#[test]
fn gc_torture_editing_survives_collection_between_every_command() {
    // Root-set completeness check for the editor: run real editing
    // (typing, kill/yank, undo, org folding/TODO cycling, buffer
    // switching, keymap use) with a forced cycle collection between
    // every keystroke. If the editor's GC root provider missed any
    // persistent Value holder, some live keymap/mode/overlay data
    // would get cleared and these assertions would break.
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 12);

    let gc_every_step = |i: &mut Interp, ed: &Rc<RefCell<Editor>>, keys: &str| {
        feed_keys(i, ed, keys).unwrap();
        elisp::gc::collect(i, false).expect("collect refused at quiescent point");
    };

    // Build a cycle deliberately so collections have real work to do.
    run(
        &mut i,
        "(let ((junk (list 1 2))) (setcdr (cdr junk) junk) nil)",
    );

    gc_every_step(&mut i, &ed, "h e l l o SPC w o r l d");
    gc_every_step(&mut i, &ed, "C-a");
    gc_every_step(&mut i, &ed, "C-SPC");
    gc_every_step(&mut i, &ed, "C-f C-f C-f C-f C-f");
    gc_every_step(&mut i, &ed, "C-w");
    assert_eq!(run(&mut i, "(buffer-string)"), "\" world\"");
    gc_every_step(&mut i, &ed, "C-y");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello world\"");
    gc_every_step(&mut i, &ed, "C-/");
    assert_eq!(run(&mut i, "(buffer-string)"), "\" world\"");

    // Org mode: overlays (folding) + buffer-local keymap survive GC.
    run(&mut i, "(erase-buffer)");
    run(&mut i, "(insert \"* Head\\nbody text\\n* Two\\n\")");
    run(&mut i, "(org-mode)");
    elisp::gc::collect(&mut i, false).unwrap();
    run(&mut i, "(goto-char (point-min))");
    gc_every_step(&mut i, &ed, "TAB");
    let row = grid_row(&i, &ed, 0);
    assert!(row.contains("..."), "org fold lost after gc: {}", row);
    gc_every_step(&mut i, &ed, "C-c C-t");
    assert!(run(&mut i, "(buffer-string)").starts_with("\"* TODO Head"));

    // Window split + isearch still fine under constant collection.
    gc_every_step(&mut i, &ed, "C-x 2");
    gc_every_step(&mut i, &ed, "C-x o");
    gc_every_step(&mut i, &ed, "C-x 1");
    gc_every_step(&mut i, &ed, "C-s b o d y RET");

    // After all that, a final collection still reclaims fresh garbage.
    run(&mut i, "(let ((a (list 1))) (setcdr a a) nil)");
    let out = elisp::gc::collect(&mut i, false).unwrap();
    assert!(out.freed >= 1, "expected the fresh cycle to be freed");
}

#[test]
fn text_properties_buffer_side() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (30, 6);
    run(&mut i, "(insert \"plain styled plain\")");

    // put/get roundtrip; positions outside the range read nil.
    run(&mut i, "(put-text-property 7 13 'face 'bold)");
    assert_eq!(run(&mut i, "(get-text-property 7 'face)"), "bold");
    assert_eq!(run(&mut i, "(get-text-property 12 'face)"), "bold");
    assert_eq!(run(&mut i, "(get-text-property 13 'face)"), "nil");
    assert_eq!(run(&mut i, "(get-text-property 1 'face)"), "nil");

    // The invisible property drives redisplay exactly like overlays do
    // (same store) — folding via text properties works.
    run(&mut i, "(put-text-property 7 13 'invisible t)");
    let row = grid_row(&i, &ed, 0);
    assert!(
        row.contains("plain ...") && row.contains("plain"),
        "row: {}",
        row
    );

    // Positions adjust across edits like overlays.
    run(&mut i, "(goto-char 1)");
    run(&mut i, "(insert \"XX\")");
    assert_eq!(run(&mut i, "(get-text-property 9 'face)"), "bold");

    // remove-text-properties clears named props in the range.
    run(
        &mut i,
        "(remove-text-properties 1 100 '(face nil invisible nil))",
    );
    assert_eq!(run(&mut i, "(get-text-property 9 'face)"), "nil");
    let row2 = grid_row(&i, &ed, 0);
    assert!(!row2.contains("..."), "unfolded row: {}", row2);
}

#[test]
fn regexp_buffer_search() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"* One\\n** TODO Two\\n* Three\\n\")");
    run(&mut i, "(goto-char (point-min))");

    // re-search-forward moves point to match end, returns 1-based pos,
    // sets match data with buffer positions.
    let r = run(
        &mut i,
        "(list (re-search-forward \"^\\\\*\\\\* \\\\(TODO \\\\)?\\\\(.*\\\\)$\" nil t)
               (match-string 2)
               (match-beginning 2))",
    );
    assert_eq!(r, "(18 \"Two\" 15)");

    // looking-at is anchored at point and does not move it.
    assert_eq!(run(&mut i, "(looking-at \"\\n\\\\* Three\")"), "t");
    let p = run(&mut i, "(point)");
    assert_eq!(p, "18");

    // re-search-backward finds the previous heading, point at match start.
    let r2 = run(
        &mut i,
        "(list (re-search-backward \"^\\\\* \" nil t) (point))",
    );
    assert_eq!(r2, "(1 1)");

    // Failed search without noerror signals; with noerror returns nil.
    assert!(run(&mut i, "(re-search-forward \"zzz\")").starts_with("ERROR:"));
    assert_eq!(run(&mut i, "(re-search-forward \"zzz\" nil t)"), "nil");
}

/// M81 R2: `noerror` means "not found isn't an error" — it must NOT
/// also swallow a `regexp-too-complex` limit hit (a budget blow-up is
/// not "not found"). All three buffer-search entry points that call
/// into `elisp::regex::Regex::search`/`match_at`
/// (`crates/core/src/builtins/editing.rs`'s `re-search-forward`,
/// `re-search-backward`, `looking-at`) get their own assertion here —
/// each is its own `?`-before-`noerror`-check call site, so a fix to
/// one doesn't cover the other two.
///
/// `\(a+\)+z` against a 30-char buffer of `a`s with no `z` (M81 R4:
/// switched from the original `[^;]*`/2,000,000-char shape — that shape
/// is PURELY LINEAR and, after the R4 fix to scale the frame budget
/// with `hay.len()`, now correctly SUCCEEDS instead of giving up, so it
/// can no longer serve as a "gives up" repro here; see
/// `regex_tests.rs`'s `long_linear_match_is_not_misclassified_as_too_complex`
/// for the fixed version of that scenario, asserted to succeed).
/// `\(a+\)+z` is the textbook nested-quantifier catastrophic-
/// backtracking shape: exponential in the number of ways to partition
/// the run of `a`s between the outer and inner `+`, so it exceeds the
/// (haystack-length-scaled) step budget almost immediately regardless
/// of how short the haystack is.
#[test]
fn regexp_too_complex_not_swallowed_by_noerror() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert (make-string 30 ?a))");

    run(&mut i, "(goto-char (point-min))");
    assert_eq!(
        run(
            &mut i,
            "(condition-case e (re-search-forward \"\\\\(a+\\\\)+z\" nil t) (error (car e)))"
        ),
        "regexp-too-complex"
    );

    run(&mut i, "(goto-char (point-max))");
    assert_eq!(
        run(
            &mut i,
            "(condition-case e (re-search-backward \"\\\\(a+\\\\)+z\" nil t) (error (car e)))"
        ),
        "regexp-too-complex"
    );

    run(&mut i, "(goto-char (point-min))");
    assert_eq!(
        run(
            &mut i,
            "(condition-case e (looking-at \"\\\\(a+\\\\)+z\") (error (car e)))"
        ),
        "regexp-too-complex"
    );
}

#[test]
fn quit_protection() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("q");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "hi").unwrap();
    run(
        &mut i,
        &format!("(find-file-internal \"{}\")", path.display()),
    );
    feed_keys(&mut i, &ed, "x").unwrap();
    feed_keys(&mut i, &ed, "C-x C-c").unwrap();
    assert!(
        !ed.borrow().quit,
        "should warn, not quit, with unsaved changes"
    );
    feed_keys(&mut i, &ed, "C-x C-c").unwrap();
    assert!(ed.borrow().quit, "second C-x C-c quits");
}

/// M43 period 3: `GapBuffer::chars_from` (backing `render_window`'s and
/// `next_row_start`'s per-cell loops) must produce the exact same
/// content whether or not the chars it walks straddle the gap, *and*
/// must correctly convert a genuinely non-zero starting char position to
/// its byte offset — not just handle position 0, which trivially maps to
/// byte 0 whether or not the conversion is actually implemented.
///
/// A one-off "中\n" prefix goes before the visible window on purpose:
/// with it, char position 2 (the start of "line one", where `window_start`
/// is forced to sit below) has a byte offset of 4, not 2 — so a char<->
/// byte conflation bug can't hide behind the "0 maps to 0 either way"
/// coincidence a `window_start == 0` test would have. The gap (via
/// `GapBuffer::insert(pos, "")`, which relocates the gap without
/// touching content — the same trick `gapbuffer.rs`'s own
/// `as_strs_reflects_gap_split`/`gap_adjacent_to_multibyte_char_boundaries`
/// tests use) is then forced to sit strictly inside the CJK line's byte
/// range, so some of that line's chars come from the pre-gap `&str`
/// segment and the rest from the post-gap one — genuinely exercising
/// `chars_from`'s `Chain` splice, not just its single-segment fast path.
#[test]
fn render_cjk_line_with_gap_mid_line_via_chars_from() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (30, 10);
    run(
        &mut i,
        "(insert \"中\\nline one\\nline two\\n春眠不覺曉\\nline four\\n\")",
    );
    let buf = ed
        .borrow()
        .find_buffer("*scratch*")
        .expect("scratch buffer must exist");

    // "中\n" (2 chars) + "line one\n" (9) + "line two\n" (9) = char 20 is
    // the CJK line's start; char 22 is 2 chars into it (right after
    // "春眠").
    let cjk_line_start = 20;
    let gap_char_pos = cjk_line_start + 2;
    let byte_pos = buf.borrow().text.char_to_byte(gap_char_pos);
    let line_start_byte = buf.borrow().text.char_to_byte(cjk_line_start);
    let line_end_byte = buf.borrow().text.char_to_byte(cjk_line_start + 5);
    // Confirm the gap actually straddles the CJK line's byte range,
    // rather than just landing elsewhere in the buffer.
    assert!(
        byte_pos > line_start_byte && byte_pos < line_end_byte,
        "gap byte offset {byte_pos} must fall strictly inside the CJK \
         line's byte range [{line_start_byte}, {line_end_byte})"
    );
    buf.borrow_mut().text.insert(gap_char_pos, "");

    // Force window_start to char 2 ("line one"'s start, right after the
    // "中\n" prefix) directly, rather than relying on scrolling to land
    // there: the render loop seeds its `chars_from` iterator exactly
    // once, at window_start, so this is what actually exercises the
    // char->byte conversion at a non-zero position.
    let sel = ed.borrow().selected_window;
    ed.borrow_mut()
        .windows
        .get_mut(&sel)
        .expect("selected window must exist")
        .window_start = 2;

    assert_eq!(grid_row(&i, &ed, 0), "line one");
    assert_eq!(grid_row(&i, &ed, 1), "line two");
    assert_eq!(grid_row(&i, &ed, 2), "春眠不覺曉");
    assert_eq!(grid_row(&i, &ed, 3), "line four");
}

// --- M72: a non-selected window's `point` must shift with edits made in
// another window on the same buffer, exactly like its `window_start`
// already does. `window_start_stays_in_sync_across_windows` in
// lsp_format_tests.rs is the pre-existing other half of this coverage —
// it only ever asserted `window_start`; these six assert `.point`
// (W1-W4) and the end-to-end user-visible symptom (W6), reading the
// `Editor`/`Window` structs directly since there's no elisp accessor for
// a non-selected window's point.

/// The id of whichever window in `ed` is NOT currently selected — same
/// two-window-split shape `window_start_stays_in_sync_across_windows`
/// uses.
fn other_window_id(ed: &Rc<RefCell<Editor>>) -> usize {
    let editor = ed.borrow();
    let mut ids: Vec<usize> = editor.windows.keys().copied().collect();
    ids.sort_unstable();
    ids.into_iter()
        .find(|id| *id != editor.selected_window)
        .expect("split created a second window")
}

/// Split the selected window, freeze the new (second) window's point at
/// elisp position `pos` (1-based, as passed to `goto-char`) by actually
/// moving its buffer point there while it's selected (setting `.point`
/// directly on a still-selected window wouldn't stick — `select_window`
/// overwrites it from the buffer's real point the moment it stops being
/// selected — see `editor::select_window`), then switch back to the
/// first window. Returns the second window's id. NOTE: `Window.point`
/// itself is stored 0-based internally (`get_pos`/`int_pos` in
/// builtins/mod.rs do the +-1 conversion at the elisp boundary) — so
/// asserting on the raw field afterward means subtracting 1 from `pos`.
fn split_and_freeze_other_point(i: &mut Interp, ed: &Rc<RefCell<Editor>>, pos: usize) -> usize {
    run(i, "(split-window-internal nil)");
    run(i, "(other-window 1)");
    run(i, &format!("(goto-char {pos})"));
    run(i, "(other-window 1)"); // back to the first window; freezes the second's point at `pos`
    other_window_id(ed)
}

/// W1: an insertion strictly before another window's frozen point shifts
/// it forward by exactly the number of characters inserted — the
/// `Buffer.point` insert threshold (`>=`, buffer.rs
/// `adjust_positions_insert`) applied to a window's saved point instead
/// of the buffer's live one.
#[test]
fn window_point_shifts_forward_past_insert_before_it() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    let other_id = split_and_freeze_other_point(&mut i, &ed, 15);

    run(&mut i, "(goto-char 5)");
    run(&mut i, "(insert \"xyz\")"); // 3 chars inserted before position 15

    assert_eq!(ed.borrow().windows[&other_id].point, 17); // (15 - 1) + 3
}

/// W2: a deletion strictly before another window's frozen point shifts it
/// back by exactly the number of characters removed.
#[test]
fn window_point_shifts_back_past_delete_before_it() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    let other_id = split_and_freeze_other_point(&mut i, &ed, 15);

    run(&mut i, "(delete-region 3 6)"); // 3 chars removed before position 15

    assert_eq!(ed.borrow().windows[&other_id].point, 11); // (15 - 1) - 3
}

/// W3: edits strictly AFTER another window's frozen point must leave it
/// unchanged. Without this, an "unconditionally shift every window's
/// point" bug would still pass W1/W2.
#[test]
fn window_point_unchanged_by_edits_after_it() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    let other_id = split_and_freeze_other_point(&mut i, &ed, 5);

    run(&mut i, "(goto-char 15)");
    run(&mut i, "(insert \"xyz\")");
    assert_eq!(ed.borrow().windows[&other_id].point, 4); // (5 - 1), unchanged

    run(&mut i, "(delete-region 18 21)");
    assert_eq!(ed.borrow().windows[&other_id].point, 4); // still unchanged
}

/// W4: a deletion whose range covers another window's frozen point must
/// clamp that point to the deleted range's start (never leave it
/// dangling inside now-gone text, and never past the end of the
/// now-shorter buffer) — same collapse `Buffer.point` gets in
/// `adjust_positions_delete`.
#[test]
fn window_point_clamps_when_delete_range_covers_it() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    let other_id = split_and_freeze_other_point(&mut i, &ed, 10);

    run(&mut i, "(delete-region 5 15)"); // covers position 10

    let point = ed.borrow().windows[&other_id].point;
    assert_eq!(point, 4); // clamped to the delete range's (0-based) start
    let buf_len: usize = run(&mut i, "(buffer-size)").parse().unwrap();
    assert!(
        point <= buf_len,
        "point {point} must not exceed buffer length {buf_len}"
    );
}

/// W6: end-to-end equivalent of the repro this milestone fixes — edit in
/// one window, switch to the other, and confirm the SECOND window's next
/// edit lands on the character the user actually sees there, not on
/// whatever character used to be at that numeric offset before the first
/// window's edit. This is the only one of these tests that exercises the
/// real command path (`other-window`, `delete-char`) rather than reading
/// `Window` fields directly, so it's the one that would actually catch a
/// regression a user could hit.
#[test]
fn window_point_end_to_end_across_window_switch() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJ\")"); // 10 chars
    let _other_id = split_and_freeze_other_point(&mut i, &ed, 5); // just before 'E'

    // Back in the first window: insert 3 chars at the very start.
    run(&mut i, "(goto-char 1)");
    run(&mut i, "(insert \"XYZ\")");

    // Switch to the second window and delete forward one char. If its
    // point had NOT been shifted, it would still numerically be 5 --
    // which, in the now-13-char buffer "XYZABCDEFGHIJ", sits just before
    // 'B' instead of 'E', and `delete-char` would remove the wrong one.
    run(&mut i, "(other-window 1)");
    run(&mut i, "(delete-char 1)");

    assert_eq!(run(&mut i, "(buffer-string)"), "\"XYZABCDFGHIJ\"");
}

/// W5: undoing an edit in one window must un-shift the OTHER window's
/// `point` AND `window_start` back to where they were before that edit
/// -- `undo-internal` (builtins/editing.rs) bypasses `edit_insert`/
/// `edit_delete` entirely (it calls `Buffer::undo_step_from` directly),
/// so before M72 period 2 this path never touched `Window` fields at
/// all: `window_start`'s drift on undo was an existing, unfixed half of
/// the M72 period-1 bug (only ever caught by `window_start_stays_in_
/// sync_across_windows` for the forward-edit path, never for undo), and
/// `point`'s drift on undo is the coordinator-reported repro that
/// motivated period 2 (`A-parked`/`B-deleted-5`/`B-undone` in the task
/// history: `A` drifted only after `B`'s edit was UNDONE, not before).
#[test]
fn window_point_and_window_start_both_resync_on_undo() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    run(&mut i, "(undo-boundary)");
    let other_id = split_and_freeze_other_point(&mut i, &ed, 15); // internal 14

    // Freeze the other window's window_start too, same direct-field
    // trick `window_start_stays_in_sync_across_windows`
    // (lsp_format_tests.rs) uses -- window_start isn't buffer-mirrored
    // like point is, so setting it directly on the (now non-selected)
    // window sticks without needing a select_window round-trip.
    ed.borrow_mut()
        .windows
        .get_mut(&other_id)
        .unwrap()
        .window_start = 8;

    // Edit in the first window: insert 3 chars at position 5 (internal
    // 4), strictly before both frozen values (14 and 8).
    run(&mut i, "(goto-char 5)");
    run(&mut i, "(insert \"xyz\")");
    {
        let editor = ed.borrow();
        let w = &editor.windows[&other_id];
        assert_eq!(w.window_start, 11); // 8 + 3
        assert_eq!(w.point, 17); // 14 + 3
    }

    // Undo the insert (single entry, no intervening boundary needed --
    // `undo-boundary` above already closed off the initial big insert).
    run(&mut i, "(undo-internal)");

    let w = &ed.borrow().windows[&other_id];
    // `window_start`'s existing (unrelated to this bug) delete threshold
    // is `>=`/`>`, matching `Buffer.point`'s exactly -- see
    // `adjust_windows_for_edit`'s doc -- so both channels round-trip
    // back to their pre-edit values here.
    assert_eq!(w.window_start, 8);
    assert_eq!(w.point, 14);
}

/// W5b: an undo group can contain MULTIPLE `AppliedEdit`s at different
/// positions (e.g. two `insert` calls typed with no `undo-boundary`
/// between them) -- `Buffer::undo_step_from` reports them in the exact
/// order it applied them, and `undo-internal` must feed them to
/// `adjust_windows_for_edit` ONE AT A TIME IN THAT ORDER, not batched.
/// This is the one test in this file that can actually tell "applied
/// sequentially" apart from "applied in some other order" -- W5 above
/// only ever undoes a single-entry group, which both a correct and a
/// order-scrambled implementation would pass identically.
///
/// Worked out by hand (all positions elisp 1-based unless marked
/// "internal"; `Window.point`/`window_start` are stored internal
/// 0-based, see `split_and_freeze_other_point`'s doc):
///
/// Starting text "ABCDEFGHIJKLMNOPQRST" (20 chars), other window frozen
/// at elisp 8 (internal 7, just before 'H'). Then, with NO boundary
/// between them:
///   1. `(goto-char 3) (insert "11")` -- inserts 2 chars at internal 2.
///      Frozen point 7 >= 2, shifts to 9.
///   2. `(goto-char 10) (insert "22")` -- position 10 here is in the
///      buffer AS IT STANDS after step 1 (22 chars long), landing at
///      internal 9 -- exactly where the frozen point now sits (both
///      "at" the 'H' that step 1 already pushed there). Frozen point
///      9 >= 9, shifts to 11.
///
/// Buffer is now "AB11CDEFG22HIJKLMNOPQRST" (24 chars), frozen point at
/// internal 11 (still immediately before 'H').
///
/// One `(undo-internal)` call undoes BOTH inserts as a single group,
/// newest-first (per `undo_step_from`'s doc): first it deletes the "22"
/// (`AppliedEdit::Delete{start:9,n:2}`, in step-1's post-edit
/// coordinates), then the "11" (`Delete{start:2,n:2}`, in the ORIGINAL
/// coordinates). Applied to the frozen point IN THAT ORDER: 11 -> 9 (>=
/// end=11) -> 7 (>= end=4). Applied in the WRONG order (11 first, i.e.
/// against step-1's delete before step-2's), the second delete
/// (`start:9,n:2`, end=11) would find point already at 9 (not >= 11 and
/// not > 9), leaving it stuck at 9 instead of continuing down to 7 --
/// this is the discrepancy this test exists to catch.
#[test]
fn window_point_resyncs_correctly_through_a_multi_edit_undo_group() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    run(&mut i, "(undo-boundary)");
    let other_id = split_and_freeze_other_point(&mut i, &ed, 8); // internal 7

    run(&mut i, "(goto-char 3)");
    run(&mut i, "(insert \"11\")");
    run(&mut i, "(goto-char 10)");
    run(&mut i, "(insert \"22\")");
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        "\"AB11CDEFG22HIJKLMNOPQRST\""
    );
    assert_eq!(ed.borrow().windows[&other_id].point, 11);

    run(&mut i, "(undo-internal)"); // undoes both inserts as one group

    assert_eq!(run(&mut i, "(buffer-string)"), "\"ABCDEFGHIJKLMNOPQRST\"");
    assert_eq!(ed.borrow().windows[&other_id].point, 7);
}

/// W7: `(erase-buffer)` must sync every OTHER window's `point` AND
/// `window_start` exactly like any other buffer-clearing delete does --
/// it goes through `editor::edit_delete` now (M72 period 2 fix for the
/// one direct-`Buffer::delete` call site the crate had left; see
/// `builtins/buffers.rs`'s `erase-buffer` doc). Both frozen values sit
/// inside the erased range `[0, len)`, so both must clamp to the
/// (now-only-valid) position 0, and neither may end up pointing past the
/// now-empty buffer.
#[test]
fn erase_buffer_syncs_other_window_point_and_window_start() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"ABCDEFGHIJKLMNOPQRST\")"); // 20 chars
    let other_id = split_and_freeze_other_point(&mut i, &ed, 15); // internal 14

    // Freeze the other window's window_start too (same direct-field
    // trick as W5 -- window_start isn't buffer-mirrored, so it sticks
    // without a select_window round-trip).
    ed.borrow_mut()
        .windows
        .get_mut(&other_id)
        .unwrap()
        .window_start = 8;

    run(&mut i, "(erase-buffer)");

    let w = &ed.borrow().windows[&other_id];
    assert_eq!(w.point, 0);
    assert_eq!(w.window_start, 0);
    let buf_len: usize = run(&mut i, "(buffer-size)").parse().unwrap();
    assert_eq!(buf_len, 0);
    assert!(w.point <= buf_len);
    assert!(w.window_start <= buf_len);
}

/// W8: edits in one buffer must leave windows showing a DIFFERENT buffer
/// completely untouched -- `adjust_windows_for_edit`'s `Rc::ptr_eq`
/// filter is what this test exists to pin down. Every other test in this
/// file (W1-W7, plus the pre-existing `window_start_stays_in_sync_
/// across_windows`) uses a single shared buffer across both windows, so
/// none of them would go red if that filter were deleted outright.
#[test]
fn window_point_unaffected_by_edits_in_a_different_buffer() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"AAAAAAAAAA\")"); // buffer "one" (unnamed default), 10 chars

    run(&mut i, "(split-window-internal nil)");
    run(&mut i, "(other-window 1)");
    run(&mut i, "(switch-to-buffer-internal \"two\")");
    run(&mut i, "(insert \"BBBBBBBBBBBBBBBBBBBB\")"); // buffer "two", 20 chars
    run(&mut i, "(goto-char 15)");
    run(&mut i, "(other-window 1)"); // back to the first window (buffer "one"); freezes buffer "two" window's point at 15

    let other_id = other_window_id(&ed);
    ed.borrow_mut()
        .windows
        .get_mut(&other_id)
        .unwrap()
        .window_start = 8;
    assert_ne!(
        Rc::as_ptr(&ed.borrow().windows[&other_id].buffer),
        Rc::as_ptr(&ed.borrow().current),
        "the frozen window must be showing a DIFFERENT buffer than the one about to be edited"
    );

    // Edit buffer "one" with an insert AND a delete that do NOT cancel
    // out: +3 then -1, a net +2. The first version of this test inserted
    // 3 and deleted 3 at the same spot, and mutation M8 (drop the
    // `Rc::ptr_eq` filter) SURVIVED it -- without the filter the frozen
    // values would have moved +3 and then -3, landing back on exactly
    // the numbers the assertions expected. The test's own comment
    // claimed a filter-less implementation "would visibly move them";
    // the mutation run proved that claim false. Keeping the two edits
    // asymmetric is the whole point: it is what makes the difference
    // between "filtered" and "not filtered" observable at all.
    run(&mut i, "(goto-char 1)");
    run(&mut i, "(insert \"XYZ\")");
    run(&mut i, "(delete-region 1 2)");

    // Unchanged. Without the filter these would be 16 and 10
    // (point 14 +3 -1, window_start 8 +3 -1).
    let w = &ed.borrow().windows[&other_id];
    assert_eq!(w.point, 14);
    assert_eq!(w.window_start, 8);
}

/// `string-width` through the real, registered builtin -- not
/// `display_width::string_width_elisp` directly.
///
/// This is the only thing standing between a future "unify the width
/// functions" refactor and a silent elisp-visible behaviour change:
/// `string-width` deliberately does NOT expand tabs the way the buffer
/// grid's `char_width` does (`(string-width "a\tb")` is 3, not 9), and
/// nothing in `layout_golden_tests.rs` can see that divergence because
/// the golden master never calls `string-width` -- it only exercises
/// `core::redisplay::render()`. `org.el` (`org.el:234`, `:276-277`) is
/// the real consumer, using `string-width` for table column alignment;
/// if this builtin silently started expanding tabs, org-mode tables
/// with a tab inside a cell would misalign with no test in this suite
/// noticing (confirmed by mutation: rewiring the builtin to an inline
/// `char_width` loop survived `org_tests` outright, because none of
/// its tests put a tab inside a table cell).
#[test]
fn string_width_builtin_does_not_expand_tabs() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(string-width \"a\\tb\")"), "3");
    assert_eq!(run(&mut i, "(string-width \"中文\")"), "4");
    assert_eq!(run(&mut i, "(string-width \"abc\")"), "3");
}
