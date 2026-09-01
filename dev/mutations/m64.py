# M64 -- The undo key you can't press in a terminal (TUI's C0 control-key
# decoding).
#
# Source of this list: the reviewer designed 4 entries (M1..M4) from a cold
# read of the diff (it only designs, doesn't execute). After the fix-up
# round, the main conversation added 3 more (K1..K3) -- `key_description`'s
# five new arms didn't exist yet when the reviewer read the diff (that bug
# was hit by the main conversation itself using a pty while running the
# gate: after the fix took effect, pressing `C-\` printed an invisible C0
# byte in the echo area instead of `C-\ is undefined`), and nobody had
# designed observation points for it. **K1..K3 were designed by the main
# conversation itself**, recorded honestly here.
#
# How to run: **must be split into three passes**, each specifying a crate.
#
#     dev/mutate.py --config dev/mutations/m64.py -p frontend-tui --only M1 --only M2 --only M3 --only M4
#     dev/mutate.py --config dev/mutations/m64.py -p core --only K1 --only K2 --only K3
#     dev/mutate.py --config dev/mutations/m64.py -p core --test-target core_tests --only M1b
#
# ## The first run produced a fake result: all 7 entries "survived"
#
# `PACKAGE` was originally left empty (intending to run the whole workspace
# with a test-name filter in one pass, covering a list spanning two crates),
# and the result was all 7 entries reported SURVIVED. It looked like nobody
# was guarding any of this milestone's fixes, but the reality was that **no
# test ran at all** -- this repo's root `Cargo.toml` is itself a package,
# and `cargo test` without `-p` only runs it, so `frontend-tui` / `core`'s
# lib tests were never even compiled in. The evidence is printed right in
# the output: `0 passed; 0 failed; 0 filtered out`. After re-running in
# separate passes with `-p` specified, all 7 entries FAILed as expected.
#
# This gap has already been backfilled into `dev/mutate.py`: it now
# classifies "no test actually ran this pass" as `NOTESTS` rather than
# SURVIVED (lesson 6 in the file header), checking the baseline as well.
# **The two call for opposite handling** -- SURVIVED means add a test,
# NOTESTS means the invocation was wrong, and adding a test in response to
# NOTESTS would only manufacture false reassurance.
#
# ## Live run results: 8 entries, 7 FAILed as expected + 1 SURVIVED as expected
#
# M1b is the only entry **expected to survive**, and that's exactly its
# purpose: while cold-reading, the reviewer pointed out that
# `undo_redo_via_c_underscore` cannot test this decoding fix at all
# (`feed_keys` goes through `parse_kbd`, never touching `convert_key`; and
# `(global-set-key "C-_" 'undo)` already existed in the base commit). With
# M1's fix reverted and the target switched to that test, a live run shows
# `1 passed` -- that test stays green even with the fix stripped out. It's
# fine for it to stay in the test file (it guards the binding itself), but
# this list uses a mutation to turn "it doesn't guard this fix" into an
# observable fact, rather than a claim someone repeats.
#
# ## One item deliberately left out of the list (honestly recorded, not
#    claimed as covered)
#
# **The `cfg!(unix)` guard has no target.** Removing `cfg!(unix) &&` produces
# identical behavior on macOS (`cfg!(unix)` is always true), so all tests
# stay green. It guards crossterm's Windows backend (which goes through
# `ToUnicodeEx`, where `Char('4')+CONTROL` really does mean the user pressed
# the 4 key), which is structurally unobservable on this machine. No test
# was forced into existence for it.

PACKAGE = None
TEST_TARGET = None

MUTATIONS = [
    # --- 4 entries designed by the reviewer ---
    {
        "label": "M1 revert the core fix (whole C0 remap removed)",
        "file": "crates/frontend-tui/src/lib.rs",
        "old": """            let mut code = if cfg!(unix) && ctrl && ('4'..='7').contains(&c) {
                0x1C + (c as u8 - b'4') as i64
            } else if ctrl {
                ctrl_encode(c)
            } else {
                c as i64
            };""",
        "new": """            let mut code = if ctrl { ctrl_encode(c) } else { c as i64 };""",
        "test": "c0_keys_match_their_kbd_bindings",
    },
    {
        "label": "M2 boundary changed to exclusive (excludes '7', exactly the undo byte)",
        "file": "crates/frontend-tui/src/lib.rs",
        "old": """            let mut code = if cfg!(unix) && ctrl && ('4'..='7').contains(&c) {""",
        "new": """            let mut code = if cfg!(unix) && ctrl && ('4'..'7').contains(&c) {""",
        "test": "ctrl_7_is_the_shared_undo_byte",
    },
    {
        "label": "M3 revert the formula off by one (0x1C to 0x1D)",
        "file": "crates/frontend-tui/src/lib.rs",
        "old": """                0x1C + (c as u8 - b'4') as i64""",
        "new": """                0x1D + (c as u8 - b'4') as i64""",
        "test": "c0_keys_match_their_kbd_bindings",
    },
    {
        "label": "M4 remove the ctrl guard (digits pressed without ctrl also get remapped)",
        "file": "crates/frontend-tui/src/lib.rs",
        "old": """            let mut code = if cfg!(unix) && ctrl && ('4'..='7').contains(&c) {""",
        "new": """            let mut code = if cfg!(unix) && ('4'..='7').contains(&c) {""",
        "test": "unmodified_digit_keys_pass_through_as_plain_chars",
    },
    {
        "label": "M1b revert the core fix, target switched to core_tests' C-_ test",
        # The only entry expected to survive, see file header. How to run:
        # -p core --test-target core_tests.
        "file": "crates/frontend-tui/src/lib.rs",
        "old": """            let mut code = if cfg!(unix) && ctrl && ('4'..='7').contains(&c) {
                0x1C + (c as u8 - b'4') as i64
            } else if ctrl {
                ctrl_encode(c)
            } else {
                c as i64
            };""",
        "new": """            let mut code = if ctrl { ctrl_encode(c) } else { c as i64 };""",
        "test": "undo_redo_via_c_underscore",
        "expect": "survived",
    },
    # --- 3 entries added by the main conversation after the fix-up round
    #     (key_description, the reviewer never saw this batch) ---
    {
        "label": "K1 remove the description arm for C-_ (the message for the undo key turns into garbage)",
        "file": "crates/core/src/keymap.rs",
        "old": """                31 => out.push_str("C-_"),
""",
        "new": "",
        "test": "c0_codes_round_trip_through_parse_kbd",
    },
    {
        "label": "K2 C-\\'s description written with the wrong value",
        "file": "crates/core/src/keymap.rs",
        "old": """                28 => out.push_str("C-\\\\"),""",
        "new": """                28 => out.push_str("C-]"),""",
        "test": "c0_control_codes_get_readable_names",
    },
    {
        # K4/K5 were added at the trailing re-review's specific request: K1-K3
        # only cover arms 31/28/0, and 29 and 30 had no target of their own.
        # "It's structurally symmetric so it should also be caught" is an
        # inference, not an observation, so real mutations were added for it.
        "label": "K4 C-]'s description written with the wrong value",
        "file": "crates/core/src/keymap.rs",
        "old": """                29 => out.push_str("C-]"),""",
        "new": """                29 => out.push_str("C-^"),""",
        "test": "c0_control_codes_get_readable_names",
    },
    {
        "label": "K5 C-^'s description written with the wrong value",
        "file": "crates/core/src/keymap.rs",
        "old": """                30 => out.push_str("C-^"),""",
        "new": """                30 => out.push_str("C-_"),""",
        "test": "c0_control_codes_get_readable_names",
    },
    {
        "label": "K3 remove the arm for C-@ (0 falls back to the printable-character branch)",
        "file": "crates/core/src/keymap.rs",
        "old": """                0 => out.push_str("C-@"),
""",
        "new": "",
        "test": "c0_control_codes_get_readable_names",
    },
]
