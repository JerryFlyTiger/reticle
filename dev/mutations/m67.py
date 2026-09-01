# M67 -- `describe-key` / `describe-bindings`, and the three new builtins
# they depend on.
#
# The list comes from two batches:
#
# 1. **M-1..M-8, designed by the reviewer from a cold read of the diff**
#    (adopted as-is, numbering kept as M1..M8). Of these, M6 and M8 are
#    **negative designs it flagged itself** -- designed with the expectation
#    that "no test will go red", used to confirm to the main conversation
#    that a coverage gap it reported is real, not something it missed while
#    scanning. M6's expectation has already flipped to FAIL now that the
#    fix-up round added a shadow test; M8 is still expected to SURVIVE.
#
# 2. **M9..M12, added by the main conversation**. The four spots of product
#    code touched by the fix-up round (echo control-character rendering,
#    `describe-function` taking the first line, shadow's prefix-chain check,
#    the `*Help*` self-reference guard) **didn't exist yet** when the
#    reviewer read the diff, so its list couldn't possibly cover them.
#    Without adding these, this round of mutation testing would falsely
#    report a pass on every one of the fix-up round's fixes -- exactly what
#    CLAUDE.md's rule "implementers shouldn't verify their own fix" is meant
#    to prevent.
#
# How to run (single pass, everything lands on the same test file):
#     dev/mutate.py --config dev/mutations/m67.py -p core --test-target help_tests
#
# **Must be run as a background task** (mutate.py file-header lesson 8:
# hitting the 10-minute limit in the foreground gets SIGKILLed, `finally`
# doesn't run, and the source code is left in a mutated state).
#
# `crates/core/lisp/simple.el` is compiled into the binary via `include_str!`
# (`crates/core/src/lib.rs:22`), so elisp mutations also require a recompile
# to take effect, and NOBUILD applies to them equally.
#
# **The two fields `expect_fail` and `note` are not read by `mutate.py`,
# they're purely for humans** (it only recognizes
# `label`/`file`/`old`/`new`/`expect`/`test`/`timeout`). This list
# deliberately sets no `test` filter, so every entry runs all 29 tests of
# `help_tests` -- FAIL means "at least one went red", which incidentally
# catches unexpected collateral damage too. `expect_fail` records **which
# tests were expected to go red at design time**; interpreting the result
# requires a human to eyeball the harness output against it, not assume the
# tooling already checked it for you.
#
# ## M2 is a probe deliberately designed to be "expected to survive",
#    reasons recorded here
#
# What the reviewer's M-1 was meant to prove is the claim "once an upper
# layer claims a binding, it short-circuits and never falls through to a
# lower layer". But actually reading the corresponding test
# `lookup_key_upper_layer_prefix_short_circuits` (`help_tests.rs:168-182`)
# reveals: it only queries a **single key**, `(list ?\C-x)`, asserting that
# the local Command wins over global. That verifies the exact same thing as
# `lookup_key_local_shadows_global`, and **no test has ever queried a
# multi-key sequence** (e.g., `(list ?\C-x ?\C-f)`).
#
# So M2 changes `lookup_layered` to "jump straight to global whenever the
# sequence length is greater than 1" -- single-key behavior is completely
# unchanged. If this survives, it means that test's name claims more than
# it actually verifies, and the reviewer's M-1 can only prove "layering
# exists", not "the short-circuit semantics hold for multi-key sequences".
# This is the same shape of lesson recorded for M66 ("ask which claim a
# mutation the reviewer hands you actually proves") recurring, recorded here
# so it doesn't need rediscovering next time.

PACKAGE = "core"
TEST_TARGET = "help_tests"

MUTATIONS = [
    # ---- eight entries designed by the reviewer ----
    {
        "label": "M1 the whole three-layer lookup priority order reversed (global checked first, emulation last)",
        "file": "crates/core/src/commands.rs",
        "old": """    match lookup_in(emulation, keys) {
        Lookup::Undefined => match lookup_in(local, keys) {
            Lookup::Undefined => (lookup_in(global, keys), Layer::Global),
            other => (other, Layer::Local),
        },
        other => (other, Layer::Emulation),
    }""",
        "new": """    match lookup_in(global, keys) {
        Lookup::Undefined => match lookup_in(local, keys) {
            Lookup::Undefined => (lookup_in(emulation, keys), Layer::Emulation),
            other => (other, Layer::Local),
        },
        other => (other, Layer::Global),
    }""",
        "expect_fail": [
            "lookup_key_local_shadows_global",
            "lookup_key_emulation_shadows_local",
            "lookup_key_upper_layer_prefix_short_circuits",
        ],
    },
    {
        "label": "M2 multi-key sequences skip emulation/local and go straight to global (single-key behavior unchanged) -- probe, expected to survive",
        "file": "crates/core/src/commands.rs",
        "old": """    match lookup_in(emulation, keys) {
        Lookup::Undefined => match lookup_in(local, keys) {""",
        "new": """    if keys.len() > 1 {
        return (lookup_in(global, keys), Layer::Global);
    }
    match lookup_in(emulation, keys) {
        Lookup::Undefined => match lookup_in(local, keys) {""",
        "note": "expected SURVIVED -- no test has ever queried the layered short-circuit with a multi-key sequence, see file header",
    },
    {
        "label": "M3 keymap_layers's local layer always returns Nil (proves describe-bindings consumes the same layering)",
        "file": "crates/core/src/commands.rs",
        "old": "    let local = buf.borrow().keymap.clone();",
        "new": "    let local = Value::Nil;",
        "expect_fail": [
            "describe_bindings_builds_readonly_help_buffer",
            "all_key_bindings_separates_layers",
            "lookup_key_local_shadows_global",
        ],
    },
    {
        "label": "M4 value_to_key returns None for Sym (named-key conversion stops working)",
        "file": "crates/core/src/commands.rs",
        "old": "        Value::Sym(id) => Some(Key::Sym(interp.sym_name(*id).to_string())),",
        "new": "        Value::Sym(_) => None,",
        "expect_fail": ["key_description_symbol_key"],
    },
    {
        "label": "M5 enumerate_bindings does not append the prefix key back onto the sequence (flattening breaks)",
        "file": "crates/core/src/keymap.rs",
        "old": "                seq.insert(0, key.clone());",
        "new": "                let _ = &key;",
        "expect_fail": ["all_key_bindings_flattens_nested_prefix"],
    },
    {
        "label": "M6 help--shadowed-p always returns nil (reviewer originally expected survival; should flip to FAIL after the fix-up round added a test)",
        "file": "crates/core/lisp/simple.el",
        "old": """  (or (member desc seen)
      (let ((shadowed nil))
        (dolist (s seen)
          (when (string-prefix-p (concat s " ") desc)
            (setq shadowed t)))
        shadowed)))""",
        "new": "  nil)",
        "expect_fail": [
            "describe_bindings_marks_exact_duplicate_as_shadowed",
            "describe_bindings_marks_prefix_chain_as_shadowed",
        ],
    },
    {
        "label": "M7 describe-function does not take the first line (reverts the fix-up round's fix)",
        "file": "crates/core/lisp/simple.el",
        "old": '      (doc (message "%s: %s" name (help--first-line doc)))',
        "new": '      (doc (message "%s: %s" name doc))',
        "expect_fail": ["describe_function_shows_only_first_docstring_line"],
    },
    {
        "label": "M8 enumerate_bindings's recursion depth cap loosened from 8 to 20",
        "file": "crates/core/src/keymap.rs",
        "old": """    if depth > 8 {
        return out;
    }""",
        "new": """    if depth > 20 {
        return out;
    }""",
        "note": (
            "The first version wrote `depth > 100_000` and expected SURVIVED, "
            "which is what the reviewer flagged as black-box. It later turned "
            "out the cause of the black-box wasn't the cap itself, it was "
            "**the shape of the test**: the test at the time used a purely "
            "self-referential keymap, and that kind of map has no leaf nodes "
            "at all, so it returns zero entries regardless of whether the "
            "cap is 8 or 100000, making the assertion always hold. After "
            "changing the test to place a real leaf node (`a`) in the cycle, "
            "leaf count = cap + 1, and the cap became observable. 20 is used "
            "here instead of 100_000 so the failure lands cleanly on the "
            "count assertion, rather than relying on a hundred-thousand-deep "
            "recursion blowing the stack -- using a crash as the observation "
            "point isn't reliable."
        ),
        "expect_fail": ["all_key_bindings_terminates_on_self_referential_keymap"],
    },
    # ---- four entries added by the main conversation: fix-up round
    #      changes that did not exist when the reviewer read the diff ----
    {
        "label": "M9 echo area no longer renders control characters as ^X (reverts the fix-up round's redisplay fix)",
        "file": "crates/core/src/redisplay.rs",
        "old": """            c if (c as u32) < 32 => {
                grid.put(echo_row, ecol, '^', Style::default());
                grid.put(
                    echo_row,
                    ecol + 1,
                    char::from_u32((c as u32) + 64).unwrap_or('?'),
                    Style::default(),
                );
            }""",
        "new": "            c if (c as u32) < 32 => grid.put_wide(echo_row, ecol, c, Style::default()),",
        "expect_fail": ["echo_control_chars_render_as_caret_notation"],
    },
    {
        "label": "M10 echo area no longer renders DEL as ^?",
        "file": "crates/core/src/redisplay.rs",
        "old": """            '\\u{7f}' => {
                grid.put(echo_row, ecol, '^', Style::default());
                grid.put(echo_row, ecol + 1, '?', Style::default());
            }""",
        "new": "            '\\u{7f}' => grid.put_wide(echo_row, ecol, '\\u{7f}', Style::default()),",
        "expect_fail": ["echo_del_also_renders_as_the_buffer_path_does"],
    },
    {
        "label": "M11 remove the *Help* self-reference guard (reverts the fix-up round's fix)",
        "file": "crates/core/lisp/simple.el",
        "old": """  (let* ((source (if (equal (buffer-name) "*Help*")
                      ;; M67 review fix: already inside *Help* (e.g. a
                      ;; second `C-h b') -- keep the ORIGINAL source
                      ;; buffer-local value instead of overwriting it
                      ;; with "*Help*" itself, which would make `q'
                      ;; (`help-quit') switch to "*Help*" and go nowhere.
                      help--source-buffer
                    (buffer-name)))""",
        "new": "  (let* ((source (buffer-name))",
        "expect_fail": [
            "describe_bindings_called_again_inside_help_still_returns_to_original_source"
        ],
    },
    {
        "label": "M12 shadow does only full-string comparison, prefix-chain check removed (reverts the fix-up round's fix)",
        "file": "crates/core/lisp/simple.el",
        "old": """  (or (member desc seen)
      (let ((shadowed nil))
        (dolist (s seen)
          (when (string-prefix-p (concat s " ") desc)
            (setq shadowed t)))
        shadowed)))""",
        "new": "  (member desc seen))",
        "expect_fail": ["describe_bindings_marks_prefix_chain_as_shadowed"],
    },
    {
        "label": "M13 remove help-mode from evil-emacs-state-modes (reverts the trailing fix-up round's fix)",
        "file": "crates/core/lisp/evil.el",
        "old": "(defvar evil-emacs-state-modes '(dired-mode eshell-mode ielm-mode help-mode)",
        "new": "(defvar evil-emacs-state-modes '(dired-mode eshell-mode ielm-mode)",
        "note": (
            "This entry guards a bug the main conversation only caught by "
            "pressing `q` in a real TUI: evil normal state's map is mounted "
            "at the emulation layer, which shadows the `q` that help-mode "
            "binds via `use-local-map`, so `help-quit` never runs. **Every "
            "other `q` test fails to catch it** -- they run in an "
            "environment where `emulation-keymap` is nil (the test harness "
            "doesn't enable evil), verifying a world that doesn't exist in "
            "the real editor. So this entry only turns red the one new test "
            "that deliberately enables evil, and the other 32 stay green."
        ),
        "expect_fail": ["describe_bindings_q_reachable_under_evil_normal_state"],
    },
]
