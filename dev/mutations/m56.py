# M56 -- Verilog library file search scope (recursive BFS +
# `verible.filelist`). List designed by the reviewer from a cold read of the
# diff, executed by the main conversation (implementers don't verify their
# own fix).
#
# What's special about this one: at handoff, the reviewer already stated
# outright that M6/M7/M8/M9 were **unverifiable at the time** -- BFS swapped
# for DFS, directory scan vs. filelist priority swapped, the filelist's
# `+`/`-` prefix guards, and the ordering of multiple library directories:
# none of these four contracts had any test watching them at the time, and
# tampering with them still left tests all green. The fix-up round added the
# corresponding tests; this file runs to check whether these four entries
# really flip to FAIL now that the tests are in place. **The entries that
# survive are the point**, not "every entry FAILs as expected".
#
# How to run: the primary target is verilog_auto_tests (the vast majority of
# M56's new tests live there), plus a separate pass against verilog_nav_tests
# to see whether the two consumers have independent coverage:
#
#     dev/mutate.py --config dev/mutations/m56.py --test-target verilog_auto_tests
#     dev/mutate.py --config dev/mutations/m56.py --test-target verilog_nav_tests
#
# Following the lesson from M55: each entry deliberately **omits the `test`
# field**, each run exercises the whole target binary. If a test-name filter
# string were written in, running it against the binary where that name
# doesn't exist would match 0 tests, cargo would return 0, and it would be
# recorded as SURVIVED -- a false negative, worse than not running it at all.
#
# Items that are black-box unobservable and **deliberately left out of the
# list**:
#
# 1. The correctness of the O(directories^2) cost explanation at the top of
#    `verilog-auto--library-files-bfs-dir`: pure comment, mutation has no
#    leverage point.
# 2. The 2026-08-10 probe results cited at the top of
#    `verilog-auto--library-filelist-files` (two `textDocument/definition`
#    runs, with and without a filelist): that's an external observation
#    against a **real server**; nothing in the test suite would turn red if
#    the comment were wrong. Its correctness rests on the main conversation
#    having actually run the probe, not on a test.
# 3. When `.git` is closer to the buffer than `verible.filelist`, the
#    filelist is silently not found -- this is a deliberately accepted
#    behavior (consistent with verible's own computed rootUri), and no test
#    pins down "it really isn't found", because pinning down a deliberate
#    blind spot would just add a false alarm the next time someone tries to
#    fix it.
#
# Padding the list with these three items just to hit a count would only
# produce permanent SURVIVED noise, which would make the list less trustworthy.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "M1 depth-limit off-by-one (< to <=, drills one level deeper)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(when (and (< cur-depth verilog-library-max-depth)",
        "new": "(when (and (<= cur-depth verilog-library-max-depth)",
    },
    {
        "label": "M2 file-count-limit off-by-one (>= to >, admits one extra)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(when (>= count verilog-library-max-files)",
        "new": "(when (> count verilog-library-max-files)",
    },
    {
        "label": "M3 remove the skip guard for dot-prefixed subdirectories",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(not (string-prefix-p \".\" name)))",
        "new": "t)",
    },
    {
        "label": "M4 remove own-file exclusion (M39 review fix)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(unless (or (and own (string= path own)) (gethash path seen))",
        "new": "(unless (gethash path seen)",
    },
    {
        "label": "M5 ignore the verilog-library-use-filelist switch",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "  (when verilog-library-use-filelist\n    (let* ((probe-file",
        "new": "  (when t\n    (let* ((probe-file",
    },
    {
        "label": "M6 filelist placed before directory scan (priority swapped)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """        (dolist (dir (verilog-auto--library-dirs))
          (when (file-directory-p dir)
            (verilog-auto--library-files-bfs-dir dir add)))
        (dolist (path (verilog-auto--library-filelist-files))
          (funcall add path))""",
        "new": """        (dolist (path (verilog-auto--library-filelist-files))
          (funcall add path))
        (dolist (dir (verilog-auto--library-dirs))
          (when (file-directory-p dir)
            (verilog-auto--library-files-bfs-dir dir add)))""",
    },
    {
        "label": "M7 BFS queue changed to DFS stack (prepend instead of append)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(setq queue (append queue (nreverse subdirs)))",
        "new": "(setq queue (append (nreverse subdirs) queue))",
    },
    {
        "label": "M8 remove the skip guard for filelist `-` flag lines",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(string-prefix-p \"-\" line))",
        "new": "(string-prefix-p \"\\u0000never\" line))",
    },
    {
        "label": "M9 remove the skip guard for filelist `+` flag lines",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(string-prefix-p \"+\" line)",
        "new": "(string-prefix-p \"\\u0000never\" line)",
    },
    {
        "label": "M10 library directory order reversed (dir list order no longer the outermost sort key)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "    (mapcar (lambda (d) (expand-file-name d base)) verilog-library-directories)))",
        "new": "    (reverse (mapcar (lambda (d) (expand-file-name d base)) verilog-library-directories))))",
    },
    {
        "label": "M11 no longer messages on truncation (silent truncation)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                       (message\n                        "Verilog library scan: stopped at %d files (verilog-library-max-files); results may be incomplete"\n                        verilog-library-max-files)\n',
        "new": "",
    },
    {
        "label": "M12 remove the skip guard for filelist `//` comment lines",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(string-prefix-p \"//\" line)",
        "new": "(string-prefix-p \"\\u0000never\" line)",
    },
    {
        "label": "M13 remove the skip guard for filelist `#` comment lines",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(string-prefix-p \"#\" line)",
        "new": "(string-prefix-p \"\\u0000never\" line)",
    },
]
