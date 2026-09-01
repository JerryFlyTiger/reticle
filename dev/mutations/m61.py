# M61 -- `Buffer.file` path normalization.
#
# M1..M3 and M5..M7 were designed by the reviewer from a cold read of the
# diff, executed by the main conversation (implementers don't verify their
# own fix). M4 is the spot the reviewer explicitly determined was
# **unobservable** by existing tests (the panel's HOME abbreviation): its
# fixture is deliberately placed outside HOME, so calling the abbreviation
# function or not produces the same string, and reverting that line wouldn't
# turn any test red. It only became observable after the fix-up round
# extracted the place-assembly logic into an injectable-home `place_for`
# function, so M4 is **an entry that only became valid after the fix-up
# round**, rewritten by the main conversation according to the reviewer's
# original intent -- recorded here per the "one round limit" rule of the
# loop, not pretended to have been cold-read.
#
# M5/M6 are **deliberately reversed-direction** mutations designed by the
# reviewer ("normalization does too much"), not "not enough": M5 removes the
# `/ssh:` early-exit to see whether remote paths get erroneously normalized;
# M6 lets an empty-string component survive into `parts` to see whether `//`
# and trailing-slash semantics get over-eaten. The blind spot from the
# previous milestone (M60) was exactly that mutations designed by the main
# conversation only thought in the "trims too little" direction; these two
# entries fill in that other perspective.
#
# How to run (TEST_TARGET deliberately left empty: the list spans both
# integration tests and mod tests inside the lib, and mutate.py's
# TEST_TARGET applies to the whole config, so leaving it empty is the only
# way to hit both):
#     dev/mutate.py --config dev/mutations/m61.py
#
# Items that are black-box unobservable, honestly listed but **left out of
# the list**:
#
# 1. `expand_file_input`'s `//` / `/~` masking runs **before** the `/ssh:`
#    prefix check, so `/ssh:h:/some//path` gets chopped down to `/path`, and
#    `/ssh:host:/proj/~/file` gets rewritten to the local machine's
#    `$HOME/file`. This is a **pre-existing flaw, deliberately not fixed in
#    this milestone** (the root cause is in `expand_file_input` in
#    `crates/core/src/complete.rs`; fixing it requires separating the
#    semantics of "what the user typed in the minibuffer" from "the program's
#    internally normalized path", which is another milestone's scope). M61
#    only closes the new entry point that `get-file-buffer` opened for it;
#    that entry is M3.
# 2. The behavior change of `find-file-internal ""` (old: creates a ghost
#    buffer with an empty filename and `.file = Some("")`; new: resolves to
#    cwd -> takes the dired branch). The new behavior matches real GNU Emacs
#    and is a fix rather than a regression, but pinning it down needs a test
#    that touches cwd, and `file_path_tests.rs` deliberately keeps only one
#    such test, T7 (`set_current_dir` is process-global, so parallel tests in
#    the same binary would contaminate each other). No test was added for
#    it, so no mutation is listed either.
# 3. `expand_file_name` produces a path that "looks absolute but actually
#    points to the wrong place" when the `dir` argument itself is a relative
#    path (a leading `/` is unconditionally prepended at the end). Pre-existing
#    flaw, and after M61 makes `default-directory` always absolute there's no
#    known reachable path to it, so no test seat.

PACKAGE = "core"

MUTATIONS = [
    {
        "label": "M1 choke point reverts to only masking/expanding ~ (no longer relative-to-absolute, no longer collapses ..)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "    crate::builtins::files::expand_file_name(p, None)\n",
        "new": "    crate::complete::expand_file_input(p)\n",
        "test": "two_spellings_of_one_file_share_one_buffer",
    },
    {
        "label": "M1b same as above, but against the data-loss regression test (do unsaved edits survive)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "    crate::builtins::files::expand_file_name(p, None)\n",
        "new": "    crate::complete::expand_file_input(p)\n",
        "test": "unsaved_edits_survive_the_second_spelling",
    },
    {
        "label": "M1c same as above, but against whether buffer-file-name is still guaranteed absolute and dot-free",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "    crate::builtins::files::expand_file_name(p, None)\n",
        "new": "    crate::complete::expand_file_input(p)\n",
        "test": "buffer_file_name_is_absolute_and_dot_free",
    },
    {
        "label": "M2 rename-file's src/dst revert to not collapsing .. (visiting buffer not found)",
        "file": "crates/core/src/builtins/files.rs",
        "old": """        let src = expand_file_name(&need_str(i, &a[0])?.to_string(), None);
        let dst = expand_file_name(&need_str(i, &a[1])?.to_string(), None);""",
        "new": """        let src = crate::complete::expand_file_input(&need_str(i, &a[0])?.to_string());
        let dst = crate::complete::expand_file_input(&need_str(i, &a[1])?.to_string());""",
        "test": "rename_file_finds_visiting_buffer_across_spellings",
    },
    {
        "label": "M3 get-file-buffer also normalizes remote paths (a remote query erroneously matches a local buffer)",
        "file": "crates/core/src/builtins/buffers.rs",
        "old": """        let path = if crate::remote::parse(&path).is_none() {
            crate::builtins::files::expand_file_name(&path, None)
        } else {
            path
        };""",
        "new": "        let path = crate::builtins::files::expand_file_name(&path, None);",
        "test": "get_file_buffer_does_not_let_a_mangled_ssh_query_match_a_local_buffer",
    },
    {
        "label": "M3b get-file-buffer does no normalization at all (reverts to plain string comparison, can't find matches across spellings)",
        "file": "crates/core/src/builtins/buffers.rs",
        "old": """        let path = if crate::remote::parse(&path).is_none() {
            crate::builtins::files::expand_file_name(&path, None)
        } else {
            path
        };""",
        "new": "",
        "test": "get_file_buffer_matches_across_spellings",
    },
    {
        "label": "M4 panel's place no longer applies HOME abbreviation (only observable after the fix-up round, see file header)",
        "file": "crates/core/src/panel.rs",
        "old": "        .map(|p| crate::complete::abbreviate_home_with(p, home))\n",
        "new": "",
        "test": "place_for_abbreviates_a_path_under_the_injected_home",
    },
    {
        "label": "M5 reversed direction: remove the /ssh: early-exit (remote paths get erroneously altered by local dot normalization)",
        "file": "crates/core/src/builtins/files.rs",
        "old": """    if joined.starts_with("/ssh:") {
        return joined;
    }
""",
        "new": "",
        "test": "expand_file_name_ssh_passthrough",
    },
    {
        # The trailing re-review corrected the mechanism description of this
        # label: it originally said "// and trailing-slash semantics get
        # over-eaten", but what actually turns the test red is that **every**
        # call gains an extra leading `/` -- `expand_file_name`'s result always
        # starts with `/`, so `split('/')` necessarily produces a leading `""`
        # component, and not discarding it turns `/x/b/c` into `//x/b/c`. It
        # doesn't only trigger in `//` or trailing-slash situations. This is
        # still a valid "normalization does too much" reversed-direction
        # mutation, just with a broader mechanism than originally described.
        "label": "M6 reversed direction: empty-string component no longer discarded (every call gains an extra leading /)",
        "file": "crates/core/src/builtins/files.rs",
        "old": '            "" | "." => {}',
        "new": '            "." => {}',
        "test": "expand_file_name_normalizes",
    },
    {
        "label": "M7 abbreviate_home_with removes the trailing-slash boundary (produces a double slash like ~//x)",
        "file": "crates/core/src/complete.rs",
        "old": '        if let Some(rest) = path.strip_prefix(&format!("{}/", home)) {',
        "new": "        if let Some(rest) = path.strip_prefix(home) {",
        "test": "abbreviate_home_with_cases",
    },
]
