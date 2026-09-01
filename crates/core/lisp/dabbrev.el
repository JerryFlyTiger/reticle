;;; dabbrev.el --- dynamic abbreviation expansion (M31) -*- lexical-binding: t -*-

;; GNU Emacs's `dabbrev-expand' (M-/): complete the word before point by
;; finding another word elsewhere in the CURRENT buffer that starts with
;; the same characters, cycling through every match on repeated presses.
;; evil.el's insert-state `C-n'/`C-p' (upstream evil's own names:
;; `evil-complete-next'/`evil-complete-previous', defined in evil.el
;; itself, right after its "Insert state entry points" section) are thin
;; wrappers around the SAME engine here with the opposite default search
;; direction -- see "Search direction" below.
;;
;; --- Prefix -----------------------------------------------------------
;; The "word" being completed is whatever run of `dabbrev--prefix-char-p'
;; characters sits immediately before point -- approximating GNU Emacs's
;; syntax-table notion of a "symbol constituent" well enough for this
;; editor's own lisp/most-programming-language identifiers: letters,
;; digits, `_', and `-' (so e.g. `evil--last-change' scans back as ONE
;; prefix, not three). An empty prefix (point sits right after a
;; character NOT in that set, or at `point-min') expands nothing -- see
;; `dabbrev--start'.
;;
;; --- Candidate collection ----------------------------------------------
;; `dabbrev--collect' runs ONCE per FRESH session (never on a rotation):
;; a single `re-search-forward' sweep of the WHOLE buffer from
;; `point-min' for `prefix + "[A-Za-z0-9_-]+"' -- the prefix literally,
;; then ONE OR MORE further prefix-class characters, so a match must
;; genuinely EXTEND the prefix (the prefix's own not-yet-extended
;; occurrence can never qualify, by construction, and neither can
;; anything else strictly inside [prefix-start, point) -- there is no
;; room for a distinct word there). The literal prefix still sits at the
;; very start of the pattern, so the P2.1 literal-prefix fast-skip
;; (`crates/elisp/src/regex.rs') applies exactly as before -- see
;; dabbrev_perf_tests.rs.
;;
;; A raw regex match can start in the middle of an existing token
;; (there is no `\b' in the pattern -- see `dabbrev--at-word-start-p''s
;; docstring for why NOT), so `dabbrev--collect' filters each match
;; through `dabbrev--at-word-start-p' before keeping it: valid only when
;; the match begins at `point-min' or right after a character that is
;; NOT itself `dabbrev--prefix-char-p'. See `dabbrev--collect''s own
;; docstring for how the surviving matches, before vs. after the cursor,
;; are ordered, `dabbrev--order' for how DIRECTION combines the two
;; halves, and `dabbrev--dedupe-strings' for how repeats of the same
;; word collapse to one candidate.
;;
;; --- Why not plain `\b' -------------------------------------------------
;; An earlier version of this file used `"\\b" + prefix + ...' instead
;; (a plain regex word-boundary assertion right before the prefix). That
;; is WRONG in both directions for this file's own prefix class, because
;; the regex engine's `\b' (`crate::regex''s `is_word': alnum + `_' ONLY
;; -- see `crates/elisp/src/regex.rs') disagrees with
;; `dabbrev--prefix-char-p' (alnum + `_' + `-'):
;; - A prefix that itself STARTS with `-' (e.g. `-foo') can fail to find
;;   real candidates: `\b' does not consider a space-to-`-' transition a
;;   boundary at all (neither side is a "word" character by its OWN
;;   definition), so `"\\b-foo..."' never even starts trying to match
;;   the one place it should.
;; - Worse, and easy to hit BY ACCIDENT in this very codebase (which is
;;   full of `xxx--yyy' names): `\b' treats the transition from `-' to a
;;   letter as a boundary it CAN reach (`-' is non-word, the letter
;;   after it is), so `"\\bsess..."' matches right in the middle of
;;   `dabbrev--session' -- offering "session" (chopping off the
;;   `dabbrev--' that, by this file's OWN character class, is just as
;;   much part of that one identifier) as if it were a real, standalone
;;   word. `dabbrev--at-word-start-p' uses the SAME character class the
;;   prefix scan itself uses, so both problems disappear together: the
;;   boundary check and "what counts as part of an identifier" can no
;;   longer disagree with each other. Pinned by
;;   `hyphen_prefixed_candidate_is_found' and
;;   `does_not_offer_a_fake_candidate_carved_out_of_a_larger_hyphenated_identifier'
;;   in dabbrev_tests.rs.
;;
;; --- Search direction ---------------------------------------------------
;; 'forward (`dabbrev-expand'/`M-/', and evil's `evil-complete-previous'/
;; `C-p'): GNU dabbrev's traditional order -- nearest match BEFORE the
;; prefix first (walking toward `point-min'), then, once that's
;; exhausted, nearest match AFTER point (walking toward `point-max').
;; 'backward (evil's `evil-complete-next'/`C-n'): the exact reverse --
;; matching upstream evil's own pairing of vim's `i_CTRL-N' (search
;; forward first) against `i_CTRL-P' (search backward first, same as
;; `M-/'). See `dabbrev--order'.
;;
;; --- Session / cycling ---------------------------------------------------
;; `dabbrev--session' (see its own docstring) remembers a session's
;; candidate list and how far a run of repeated presses has rotated
;; through it, so `C-n'/`C-p'/`M-/' pressed AGAIN right after an
;; expansion moves to the next candidate instead of re-scanning the
;; buffer and starting over. "Right after" is self-checked
;; (`dabbrev--continue-p') rather than tracked via any notion of "last
;; command" (this codebase's `last-command'/`this-command' have no
;; elisp accessor -- see evil.el's redo section for the identical
;; constraint): a session continues exactly when (a) `(current-buffer)'
;; is still the buffer the session belongs to, (b) point is still
;; wherever the last rotation left it, (c) `(buffer-modified-tick)' has
;; not moved since (nothing anywhere in the buffer was inserted or
;; deleted), AND (d) the text between the prefix's start and point is
;; still exactly what that rotation put there. Any one failing means
;; something else happened in between, so the very next press starts a
;; brand new session against whatever prefix is under point NOW.
;;
;; (b)+(d) alone -- an earlier version of this file's check, before a
;; review caught it -- are NOT enough: `evil-insert-exit' (ESC)
;; unconditionally does `backward-char' on the way out, which can
;; silently re-align point with LAST-POINT even after an edit. Concrete
;; failure: C-n expands "pri" to "println", the user types ONE more
;; character (anything -- buffer now "printlnX", point == LAST-POINT +
;; 1), then ESC -- whose `backward-char' lands point back on exactly
;; LAST-POINT. `(buffer-substring PREFIX-START (point))' over
;; [PREFIX-START, LAST-POINT) never even LOOKS at the character sitting
;; at LAST-POINT (that's the newly typed one, one past the checked
;; range), so it still reads "println" -- both old conditions pass, the
;; stale session "continues", and the NEXT press's "back to original"
;; rewrite deletes exactly "println" (the checked span) and splices
;; "pri" back in front of the untouched "X" -- silently producing
;; "priX" while the checked span itself looks perfectly consistent.
;; (c) (any edit bumps the tick, typing that one extra character
;; included) closes this: it fails independently of what the point/text
;; check happens to see. (a) closes the parallel cross-buffer version of
;; the same false-positive (switching buffers via `C-x b' and landing on
;; a coincidentally-matching point/text in the NEW buffer used to
;; "continue" the OLD buffer's session). Pinned by
;; `typing_one_more_character_then_esc_does_not_falsely_continue_the_session'
;; and `switching_buffers_does_not_falsely_continue_the_session' in
;; dabbrev_tests.rs.
;;
;; Cycling past the last real candidate restores the ORIGINAL,
;; pre-expansion prefix (vim semantics), echoing "(back to original)";
;; one more press starts the candidate list over from the top. Three
;; commands share one session (direction only matters for STARTING a
;; fresh one): pressing `C-p' to continue a session `C-n' started still
;; just advances to the next slot, same as pressing `C-n' again would --
;; there is no notion of "step backward through the list" here, a
;; deliberate simplification given the session state (see
;; `dabbrev--session') tracks no direction of its own.
;;
;; --- Interaction with evil's dot-repeat (`.') ---------------------------
;; No special-casing needed: an expansion is just an ordinary
;; delete-region+insert like any other buffer edit, so it is
;; automatically part of whatever text `evil--finish-insert-session'
;; captures between `evil--insert-start' and point when ESC ends the
;; insert session (see evil.el) -- `.' afterward replays the captured
;; text, completion result included. One caveat, narrow enough not to be
;; worth guarding against: `dabbrev--prefix-start' walks backward by
;; character class alone, with no awareness of `evil--insert-start' --
;; if insert state was entered in the MIDDLE of an already-existing word
;; (so some prefix-class text precedes `evil--insert-start' itself),
;; the scan can reach earlier than the insert session's own tracked
;; start, and the resulting delete-region then edits text from BEFORE
;; that start. `evil--finish-insert-session''s own capture
;; (`buffer-substring' between the fixed `evil--insert-start' position
;; and point) does not know this happened, so a `.' replay afterward can
;; come out wrong in that specific scenario -- the same documented
;; "best-effort outside the session's own span" territory evil.el's file
;; header already stakes out for insert-map, just reached by a different
;; door (a completion instead of a mouse click).
;;
;; --- `M-/' inside evil-mode's normal state -------------------------------
;; `M-/' is bound in the GLOBAL keymap only (see the bottom of this
;; file) -- there is no binding for it anywhere in `evil--normal-map'
;; (evil.el), and evil's `emulation-keymap' is only ever consulted AHEAD
;; of the local/global keymaps, never as a full replacement for them
;; (see evil.el's own file header). So a plain `M-/' in NORMAL state
;; falls straight through evil's keymap layer to this same global
;; binding and runs `dabbrev-expand' there too, exactly as it would with
;; evil-mode off entirely. Deliberately accepted, not shadowed: vim has
;; no standalone-in-normal-state text-completion keybinding to begin
;; with (`i_CTRL-N'/`i_CTRL-P' only exist inside insert mode), so there
;; is nothing vim-authentic for `M-/' to preempt here, and letting it
;; through matches upstream evil-mode's own long-standing philosophy of
;; leaving any key it doesn't itself claim to fall through to its
;; ordinary Emacs binding (the same reason `M-x', `C-x'-prefixed
;; commands, etc. all still work in every evil state). Pinned by
;; `m-slash-falls-through-to-dabbrev-expand-in-evil-normal-state' in
;; dabbrev_tests.rs.

(defvar dabbrev--session nil
  "nil, or the state of an in-progress completion cycle: a list
\(PREFIX-START ORIGINAL-PREFIX CANDIDATES INDEX LAST-POINT LAST-TEXT
LAST-BUFFER LAST-TICK).
  PREFIX-START     buffer position where the prefix/expansion begins.
  ORIGINAL-PREFIX  the literal text the user had typed, before any
                   expansion -- what cycling past the last candidate
                   restores.
  CANDIDATES       the ordered, de-duplicated list of candidate strings
                   for this session (fixed for its whole lifetime).
  INDEX            which slot is currently shown: 0..(length
                   CANDIDATES)-1 is a real candidate; (length
                   CANDIDATES) itself means ORIGINAL-PREFIX is currently
                   shown (\"back to original\").
  LAST-POINT       `(point)' right after the most recent rotation.
  LAST-TEXT        the buffer text between PREFIX-START and LAST-POINT
                   right after that same rotation.
  LAST-BUFFER      `(current-buffer)' at that same moment.
  LAST-TICK        `(buffer-modified-tick)' at that same moment.
LAST-POINT/LAST-TEXT/LAST-BUFFER/LAST-TICK are how `dabbrev--continue-p'
tells a genuine repeat press (nothing else touched the buffer or moved
point since) from a fresh completion attempt -- see the file header's
\"Session / cycling\" section for why LAST-POINT/LAST-TEXT ALONE are not
enough. Global rather than buffer-local, like evil.el's own dot-repeat/
yank state: a session started in one buffer never sensibly continues in
another (LAST-BUFFER is exactly what makes that a checked invariant
instead of an assumption).")

(defun dabbrev--prefix-char-p (c)
  "Non-nil if C belongs to a dabbrev prefix -- letters, digits, `_',
`-' (see the file header for why `-' is included)."
  (and c (or (and (>= c ?a) (<= c ?z))
             (and (>= c ?A) (<= c ?Z))
             (and (>= c ?0) (<= c ?9))
             (= c ?_)
             (= c ?-))))

(defun dabbrev--prefix-start (pos)
  "The start of the run of `dabbrev--prefix-char-p' characters ending at
POS -- POS itself when POS sits right after a non-prefix character (or
at `point-min'), i.e. an empty prefix."
  (let ((p pos))
    (while (and (> p (point-min)) (dabbrev--prefix-char-p (char-after (1- p))))
      (setq p (1- p)))
    p))

(defun dabbrev--at-word-start-p (mb)
  "Non-nil if MB (a buffer position) is a genuine token start under
THIS file's OWN prefix character class (`dabbrev--prefix-char-p'): MB
is `point-min', or the character immediately before it is NOT a prefix
character. Deliberately NOT a plain regex `\\b' -- see the file
header's \"Why not plain \\b\" section for the two ways that disagrees
with this class (misses `-'-initial prefixes; wrongly treats the spot
right after a `-' inside a `xxx--yyy'-style name as a boundary)."
  (or (= mb (point-min))
      (not (dabbrev--prefix-char-p (char-after (1- mb))))))

(defun dabbrev--collect (prefix prefix-start point)
  "One full-buffer scan for words extending PREFIX (see the file
header). Returns (BEFORE . AFTER): BEFORE holds (STRING . POS) pairs
with POS < PREFIX-START, ordered NEAREST-TO-PREFIX-START FIRST; AFTER
holds pairs with POS > POINT, ordered NEAREST-TO-POINT FIRST. The
occurrence exactly AT prefix-start (the cursor's own, still-being-typed
prefix) and anything strictly between PREFIX-START and POINT (which can
only be the prefix overlapping itself, never a distinct word) are both
excluded by construction -- neither satisfies `<' PREFIX-START nor `>'
POINT below, so the `cond' simply does nothing for them. A match that
starts in the middle of some larger token (`dabbrev--at-word-start-p'
says no) is discarded before it ever reaches that `cond' at all.

Built with a single left-to-right pass (matches arrive in ascending
position order): consing onto BEFORE as we go leaves it in DESCENDING
position order for free -- exactly \"nearest to PREFIX-START first\",
since every BEFORE position is < PREFIX-START. AFTER needs the opposite
\(ascending, \"nearest to POINT first\") so it is built the same way and
then reversed once, at the end."
  (let ((pattern (concat (regexp-quote prefix) "[A-Za-z0-9_-]+"))
        (before nil)
        (after nil))
    (save-excursion
      (goto-char (point-min))
      (while (re-search-forward pattern nil t)
        (let ((mb (match-beginning 0)))
          (when (dabbrev--at-word-start-p mb)
            (cond
              ((< mb prefix-start) (setq before (cons (cons (match-string 0) mb) before)))
              ((> mb point) (setq after (cons (cons (match-string 0) mb) after))))))))
    (cons before (nreverse after))))

(defun dabbrev--order (before after direction)
  "BEFORE/AFTER already carry their own internal priority order (see
`dabbrev--collect'); DIRECTION picks which side is tried first --
'forward: BEFORE then AFTER. 'backward: the reverse. See the file
header's \"Search direction\" section."
  (if (eq direction 'forward)
      (append before after)
    (append after before)))

(defun dabbrev--dedupe-strings (pairs)
  "PAIRS is a list of (STRING . POS) in priority order. Returns just the
STRINGs, keeping each distinct string's FIRST (highest-priority, i.e.
closest) occurrence and dropping every later repeat -- a word occurring
three times in the buffer still contributes exactly one candidate."
  (let ((seen nil) (out nil))
    (dolist (p pairs)
      (unless (member (car p) seen)
        (setq seen (cons (car p) seen))
        (setq out (cons (car p) out))))
    (nreverse out)))

(defun dabbrev--replace-region (beg end text)
  "Delete [BEG,END) and insert TEXT in its place, leaving point right
after TEXT -- the shared shape a fresh expansion, a rotation
\(`dabbrev--apply'), and cycling back to the original prefix all use."
  (delete-region beg end)
  (goto-char beg)
  (insert text))

(defun dabbrev--record-session (prefix-start original candidates index text)
  "Build and store the `dabbrev--session' value for INDEX/TEXT having
just been placed at PREFIX-START -- the one place that captures
LAST-BUFFER/LAST-TICK alongside LAST-POINT/LAST-TEXT (see
`dabbrev--session''s docstring), so both `dabbrev--apply' and
`dabbrev--rotate''s \"back to original\" branch record identically."
  (setq dabbrev--session
        (list prefix-start original candidates index (point) text
              (current-buffer) (buffer-modified-tick))))

(defun dabbrev--apply (prefix-start original candidates index)
  "Expand to the INDEXth entry of CANDIDATES at PREFIX-START (replacing
whatever currently occupies [PREFIX-START, point)) and record the
result as `dabbrev--session' -- the one place both a brand new session
\(INDEX 0, from `dabbrev--start') and an ordinary rotation
\(`dabbrev--rotate') land."
  (let ((text (nth index candidates)))
    (dabbrev--replace-region prefix-start (point) text)
    (dabbrev--record-session prefix-start original candidates index text)))

(defun dabbrev--continue-p ()
  "Non-nil when `dabbrev--session' is still live: same buffer, point
hasn't moved since the last rotation, no edit has happened anywhere in
that buffer since (the `buffer-modified-tick' check), AND the text the
last rotation left behind is still there, untouched -- see the file
header's \"Session / cycling\" section for why all four are needed (in
particular: point-and-text-back-to-the-same-value alone is NOT enough,
since ESC's own `backward-char' can silently restore both after an
extra character was typed right at the old point)."
  (and dabbrev--session
       (eq (current-buffer) (nth 6 dabbrev--session))
       (= (point) (nth 4 dabbrev--session))
       (= (buffer-modified-tick) (nth 7 dabbrev--session))
       (equal (buffer-substring (nth 0 dabbrev--session) (point))
              (nth 5 dabbrev--session))))

(defun dabbrev--start (direction)
  "Begin a brand new completion at point, searching in DIRECTION (see
`dabbrev--order'). Always clears any stale `dabbrev--session' first --
by the time this runs, `dabbrev--continue-p' has already said the old
one no longer applies, and nothing here should be able to leave a
half-updated one behind."
  (setq dabbrev--session nil)
  (let* ((point (point))
         (prefix-start (dabbrev--prefix-start point)))
    (if (= prefix-start point)
        (message "No dynamic expansion possible here")
      (let* ((prefix (buffer-substring prefix-start point))
             (groups (dabbrev--collect prefix prefix-start point))
             (ordered (dabbrev--order (car groups) (cdr groups) direction))
             (candidates (dabbrev--dedupe-strings ordered)))
        (if (null candidates)
            (message "No dynamic expansion for \"%s\" found" prefix)
          (dabbrev--apply prefix-start prefix candidates 0))))))

(defun dabbrev--rotate ()
  "Continue a live session: advance to the next slot, wrapping through
\"back to original\" (vim semantics) before the candidate list repeats."
  (let* ((s dabbrev--session)
         (prefix-start (nth 0 s))
         (original (nth 1 s))
         (candidates (nth 2 s))
         (index (nth 3 s))
         (n (length candidates))
         (next (mod (1+ index) (1+ n))))
    (if (= next n)
        (progn
          (dabbrev--replace-region prefix-start (point) original)
          (dabbrev--record-session prefix-start original candidates next original)
          (message "(back to original)"))
      (dabbrev--apply prefix-start original candidates next))))

(defun dabbrev--complete (direction)
  "Shared engine behind `dabbrev-expand' and evil.el's
`evil-complete-next'/`evil-complete-previous': continue the live
session if there is one (`dabbrev--continue-p'), otherwise start a
fresh one searching in DIRECTION."
  (if (dabbrev--continue-p)
      (dabbrev--rotate)
    (dabbrev--start direction)))

(defun dabbrev-expand ()
  "Dynamically expand the word before point, GNU Emacs style: find
another word in this buffer starting with the same characters and
replace the prefix with it (searching backward from point first -- see
`dabbrev--order''s 'forward case). Repeating immediately after cycles
through every match in turn, then restores the original text, then
repeats -- see the file header."
  (interactive)
  (dabbrev--complete 'forward))

(global-set-key "M-/" 'dabbrev-expand)

(provide 'dabbrev)
