//! Line-level Myers diff producing a minimal set of character-range
//! replacements ("hunks") between an old and a new string.
//!
//! This exists so `replace-region-contents` (M46) can apply an LSP
//! `TextEdit`'s `newText` incrementally instead of nuking and reinserting
//! the whole region. A naive delete-everything-then-insert-everything
//! replace collapses every marker, point, and overlay in the region to a
//! single position — see the `replace-region-contents` docstring in
//! `crates/core/src/builtins/editing.rs` for the measured fallout
//! (overlay start/end swapped, `overlays-in` blind to the surviving
//! overlay). Computing the actual diff and applying only the hunks that
//! changed lets unaffected markers/point/overlays ride out the edit
//! untouched.
//!
//! ## The `budget` parameter
//!
//! Full-buffer reformatting (e.g. `verible-verilog-ls` re-indenting an
//! entire file) touches nearly every line, so the line-level edit
//! distance D approaches 2N (delete every old line, insert every new
//! line). At that point there is nothing left to preserve — every line
//! *did* change — so falling back to a single whole-region replace is not
//! a degraded outcome, it *is* the correct outcome; the algorithm should
//! not spend O(N·D) work computing a diff whose hunks would cover
//! (almost) the entire buffer anyway. `budget` caps the line-level edit
//! distance the Myers search is allowed to explore; once the search would
//! need more than `budget` steps, `diff_hunks` gives up and returns
//! `None` so the caller can do the whole-region replace instead. This
//! mirrors GNU Emacs's `replace-region-contents`, which has the same
//! escape hatch via its `MAX-SECS`/`MAX-COSTS` arguments — "no diff is
//! cheap enough to be worth it" is a legitimate answer, not a bug.
//!
//! ## Space complexity
//!
//! [`myers_trace`] stores one full `v`-array snapshot per explored edit
//! distance `d` (0..=max_d) so [`backtrack`] can walk back through the
//! search afterward ("Myers with history"). Each snapshot has width
//! `O(max_d)`, so total memory is `O(max_d^2)` — bounded by `budget^2`,
//! not by the *actual* edit distance `D^2` of the two inputs. A large
//! `budget` therefore costs memory proportional to the cap even when the
//! real diff is small; this is inherent to keeping the full backtrace
//! (the classic Myers algorithm has an `O(ND)`-time, `O(D)`-space
//! variant that gives up the backtrace and recovers the script via a
//! separate divide-and-conquer pass instead), not a bug in this
//! implementation.

/// A single replacement: substitute the half-open character range
/// `[start, end)` of the *original* string with `text`.
///
/// `start`/`end` are 0-based **character** (not byte, not line) offsets
/// into the original string. Hunks are returned in strictly increasing
/// `start` order, and never overlap or touch (no two hunks are adjacent
/// with nothing preserved between them — such hunks are merged upstream
/// by the algorithm never producing them in the first place... actually:
/// adjacency across a byte gap can't happen because the char-level
/// trimming step only shrinks hunks, never re-splits them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Default line-level edit-distance budget for [`diff_hunks`].
///
/// Chosen so that ordinary multi-hunk edits (a handful of lines changed
/// across a file of any size) always compute an incremental diff, while
/// pathological whole-file rewrites (D on the order of the file's line
/// count) fall back to a full replace well before the O((N+M)·D) Myers
/// search becomes expensive. 4096 comfortably covers "a few thousand
/// scattered single-line edits" — far more than any realistic LSP
/// formatting/codeAction response — while still being tiny next to the
/// tens-of-thousands-of-lines file sizes where an O((N+M)·D) search at
/// this D would start to be felt.
pub const DEFAULT_BUDGET: usize = 4096;

/// Split `s` into lines, each retaining its trailing `\n` (the final line
/// has none if `s` doesn't end in `\n`). This makes the pieces
/// concatenate back to exactly `s`, so line indices translate losslessly
/// to character offsets.
fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split_inclusive('\n').collect()
}

/// Cumulative character-count prefix sums: `offsets[i]` is the number of
/// characters in `lines[0..i]`. `offsets.len() == lines.len() + 1`.
fn char_offsets(lines: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len() + 1);
    let mut acc = 0usize;
    offsets.push(0);
    for l in lines {
        acc += l.chars().count();
        offsets.push(acc);
    }
    offsets
}

enum Op {
    Keep,
    Del,
    Ins,
}

/// Myers shortest-edit-script search over line slices `a` and `b`, capped
/// at edit distance `max`. Returns the backtrace (one V-array snapshot
/// per explored distance, plus the `offset` used to index each snapshot)
/// on success, or `None` if no edit script of distance `<= max` exists.
///
/// The offset must be returned alongside the trace (rather than
/// recomputed by the caller from `trace.len()`) because it's derived
/// from the *budget-clamped* `max_d`, not from the distance at which the
/// search actually terminated — those two only coincide when the search
/// runs all the way to `max_d` without finding a solution early, which
/// is the uncommon case. Recomputing it from `trace.len()` silently
/// indexes into the wrong slots of every stored `v` snapshot whenever
/// the true edit distance is smaller than `max`, corrupting the
/// backtrack without any observable panic (found the hard way: it
/// produces a same-length but *invalid* edit script — duplicated/dropped
/// lines — not a crash).
fn myers_trace(a: &[&str], b: &[&str], max: usize) -> Option<(usize, Vec<Vec<i64>>)> {
    let n = a.len() as i64;
    let m = b.len() as i64;
    // Saturate rather than cast-and-wrap: `max` can legitimately be a
    // huge value (a caller expressing "no real cap", e.g. `usize::MAX`)
    // and `max as i64` would silently two's-complement-wrap negative for
    // any `max >= 2^63`. A negative `max_d` makes the `0..=max_d` search
    // loop below never execute, so `myers_trace` would wrongly return
    // `None` even for a trivial one-line change — the exact opposite of
    // what an "unlimited" budget should mean. `.min(n + m)` afterward
    // still clamps it to something the search can actually reach.
    let max_d = i64::try_from(max).unwrap_or(i64::MAX).min(n + m);
    // `offset` is padded by one beyond `max_d` so that the neighbor
    // lookups (`idx - 1` / `idx + 1`) the algorithm performs at the
    // extremes `k == -d` / `k == d` never run off the end of `v`, even
    // when `max_d == 0`.
    let offset = (max_d + 1) as usize;
    let width = 2 * offset + 1;
    let mut v = vec![0i64; width];
    let mut trace = Vec::new();
    for d in 0..=max_d {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + offset as i64) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                return Some((offset, trace));
            }
            k += 2;
        }
    }
    None
}

/// Walk `trace` backwards from `(a.len(), b.len())` to `(0, 0)`, yielding
/// one `Op` per unit step of the shortest edit script, oldest-first.
/// Each `Op::Del`/`Op::Ins` carries the consumed index via the returned
/// `(op, a_idx, b_idx)` triples so callers don't need to re-derive
/// position from op order alone. `offset` must be the same value
/// [`myers_trace`] returned alongside this `trace`.
fn backtrack(a: &[&str], b: &[&str], offset: usize, trace: &[Vec<i64>]) -> Vec<(Op, i64, i64)> {
    let mut x = a.len() as i64;
    let mut y = b.len() as i64;
    let mut steps: Vec<(Op, i64, i64)> = Vec::new();

    for d in (0..trace.len()).rev() {
        let d = d as i64;
        let v = &trace[d as usize];
        let k = x - y;
        let prev_k = if k == -d
            || (k != d && v[(k - 1 + offset as i64) as usize] < v[(k + 1 + offset as i64) as usize])
        {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[(prev_k + offset as i64) as usize];
        let prev_y = prev_x - prev_k;

        // Diagonal (matching) steps walked while extending the snake.
        while x > prev_x && y > prev_y {
            steps.push((Op::Keep, x - 1, y - 1));
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            if x == prev_x {
                // Vertical move: an insertion of b[prev_y].
                steps.push((Op::Ins, prev_x, prev_y));
            } else {
                // Horizontal move: a deletion of a[prev_x].
                steps.push((Op::Del, prev_x, prev_y));
            }
        }
        x = prev_x;
        y = prev_y;
    }
    steps.reverse();
    steps
}

/// Line-level Myers diff between `old` and `new`, returning the minimal
/// set of character-range replacements (in increasing, non-overlapping
/// `start` order) needed to turn `old` into `new`.
///
/// Returns `None` if the line-level edit distance exceeds `budget` — see
/// the module docs for why that's the correct behavior, not a limitation
/// to work around.
pub fn diff_hunks(old: &str, new: &str, budget: usize) -> Option<Vec<Hunk>> {
    if old == new {
        return Some(Vec::new());
    }

    let old_lines = split_lines(old);
    let new_lines = split_lines(new);

    // Phase 1: strip common line-level prefix/suffix so Myers only runs
    // over the actual window of change.
    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];

    // Phase 2: Myers diff on the reduced window, budget-capped.
    let (offset, trace) = myers_trace(old_mid, new_mid, budget)?;
    let steps = backtrack(old_mid, new_mid, offset, &trace);

    let old_offsets = char_offsets(&old_lines);

    // Phase 3: group consecutive non-Keep steps into hunks, then trim
    // each hunk's common character-level prefix/suffix.
    let mut hunks = Vec::new();
    let mut i = 0usize;
    while i < steps.len() {
        if matches!(steps[i].0, Op::Keep) {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < steps.len() && !matches!(steps[i].0, Op::Keep) {
            i += 1;
        }
        let run = &steps[run_start..i];

        // Position (in mid-array line indices) before/after this run:
        // the first step's a/b index is where the run starts consuming;
        // the last step's post-step position is where it ends.
        let (a_start, b_start) = match &run[0] {
            (Op::Del, ai, bi) => (*ai, *bi),
            (Op::Ins, ai, bi) => (*ai, *bi),
            (Op::Keep, ..) => unreachable!(),
        };
        let (mut a_end, mut b_end) = (a_start, b_start);
        for (op, ai, bi) in run {
            match op {
                Op::Del => a_end = ai + 1,
                Op::Ins => b_end = bi + 1,
                Op::Keep => unreachable!(),
            }
        }

        let old_lo = prefix + a_start as usize;
        let old_hi = prefix + a_end as usize;
        let new_lo = prefix + b_start as usize;
        let new_hi = prefix + b_end as usize;

        let start = old_offsets[old_lo];
        let end = old_offsets[old_hi];
        let text: String = new_lines[new_lo..new_hi].concat();

        if let Some(h) = trim_hunk(old, start, end, text) {
            hunks.push(h);
        }
    }

    Some(hunks)
}

/// Character-level trim: strip the common prefix/suffix between
/// `old[start..end]` (by char offset) and `text`, shrinking the hunk to
/// the actually-changed characters. Drops the hunk entirely (returns
/// `None`) if trimming leaves nothing to replace.
fn trim_hunk(old: &str, start: usize, end: usize, text: String) -> Option<Hunk> {
    let old_chars: Vec<char> = old.chars().collect();
    let old_slice = &old_chars[start..end];
    let new_chars: Vec<char> = text.chars().collect();

    let mut pre = 0usize;
    let max_pre = old_slice.len().min(new_chars.len());
    while pre < max_pre && old_slice[pre] == new_chars[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    let max_suf = old_slice.len() - pre;
    let max_suf = max_suf.min(new_chars.len() - pre);
    while suf < max_suf
        && old_slice[old_slice.len() - 1 - suf] == new_chars[new_chars.len() - 1 - suf]
    {
        suf += 1;
    }

    let new_start = start + pre;
    let new_end = end - suf;
    let new_text: String = new_chars[pre..new_chars.len() - suf].iter().collect();

    if new_start == new_end && new_text.is_empty() {
        None
    } else {
        Some(Hunk {
            start: new_start,
            end: new_end,
            text: new_text,
        })
    }
}

/// Apply `hunks` (in the order returned by [`diff_hunks`], i.e.
/// increasing `start`) to `old`, back-to-front so earlier hunks' offsets
/// stay valid. Used only by tests to assert diff/apply round-trip.
#[cfg(test)]
fn apply_hunks(old: &str, hunks: &[Hunk]) -> String {
    let mut chars: Vec<char> = old.chars().collect();
    for h in hunks.iter().rev() {
        let replacement: Vec<char> = h.text.chars().collect();
        chars.splice(h.start..h.end, replacement);
    }
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic xorshift PRNG so the round-trip test is
    /// reproducible without pulling in a `rand` dependency.
    struct Prng(u64);
    impl Prng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn range(&mut self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                (self.next() % n as u64) as usize
            }
        }
    }

    fn gen_lines(prng: &mut Prng, n: usize) -> Vec<String> {
        let words = [
            "logic",
            "[3:0]",
            "a;",
            "endmodule",
            "input",
            "wire",
            "// 註解",
            "😀x",
        ];
        (0..n)
            .map(|_| {
                let w = prng.range(4) + 1;
                (0..w)
                    .map(|_| words[prng.range(words.len())])
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    fn mutate(prng: &mut Prng, lines: &[String]) -> Vec<String> {
        let mut out = lines.to_vec();
        let ops = prng.range(5) + 1;
        for _ in 0..ops {
            if out.is_empty() {
                out.push("x".to_string());
                continue;
            }
            match prng.range(5) {
                0 => {
                    // Delete a random line.
                    let i = prng.range(out.len());
                    out.remove(i);
                }
                1 => {
                    // Insert a random line.
                    let i = prng.range(out.len() + 1);
                    out.insert(i, gen_lines(prng, 1).pop().unwrap());
                }
                2 => {
                    // Whitespace perturbation on a random line (the
                    // Verilog-alignment case textdiff must handle well).
                    let i = prng.range(out.len());
                    out[i] = format!("  {}  ", out[i]);
                }
                3 => {
                    // Reorder: swap two random lines. Myers diff over
                    // moved (not just inserted/deleted/edited) lines is
                    // otherwise untested — a swap forces the algorithm to
                    // treat both positions as changed rather than finding
                    // a no-op alignment.
                    if out.len() >= 2 {
                        let i = prng.range(out.len());
                        let j = prng.range(out.len());
                        out.swap(i, j);
                    }
                }
                _ => {
                    // Replace a random line's content entirely.
                    let i = prng.range(out.len());
                    out[i] = gen_lines(prng, 1).pop().unwrap();
                }
            }
        }
        out
    }

    fn join(lines: &[String]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    /// Like `join`, but without a trailing newline after the last line —
    /// the shape `replace-region-contents`'s actual callers produce when
    /// the region is `line-beginning-position`..`line-end-position` (a
    /// single line, no newline included) or any other region that
    /// doesn't happen to end exactly at a line boundary. `join` alone
    /// never exercises this: every input it produces ends in `\n`.
    fn join_no_trailing_newline(lines: &[String]) -> String {
        lines.join("\n")
    }

    #[test]
    fn round_trip_random_edits() {
        let mut prng = Prng(0x1234_5678_9abc_def0);
        for case in 0..500 {
            let n = prng.range(30) + 1;
            let old_lines = gen_lines(&mut prng, n);
            let new_lines = mutate(&mut prng, &old_lines);
            let old = join(&old_lines);
            let new = join(&new_lines);

            let hunks = diff_hunks(&old, &new, DEFAULT_BUDGET);
            let hunks = match hunks {
                Some(h) => h,
                None => continue, // budget exceeded is a valid outcome
            };

            // Hunks must be increasing and non-overlapping.
            let mut prev_end = 0usize;
            for h in &hunks {
                assert!(
                    h.start >= prev_end,
                    "case {case}: hunks out of order/overlapping"
                );
                assert!(h.start <= h.end, "case {case}: inverted hunk");
                prev_end = h.end;
            }

            let applied = apply_hunks(&old, &hunks);
            assert_eq!(
                applied, new,
                "case {case}: round-trip mismatch (old={old:?} new={new:?})"
            );
        }
    }

    /// Same round-trip property as `round_trip_random_edits`, but over
    /// inputs that don't end in a trailing newline — the shape a region
    /// bounded by `line-beginning-position`/`line-end-position` actually
    /// has (two existing integration tests replace exactly such a
    /// region), which `join`'s always-`\n`-terminated output never
    /// produces.
    #[test]
    fn round_trip_random_edits_no_trailing_newline() {
        let mut prng = Prng(0xfeed_face_dead_beef);
        for case in 0..500 {
            let n = prng.range(30) + 1;
            let old_lines = gen_lines(&mut prng, n);
            let new_lines = mutate(&mut prng, &old_lines);
            let old = join_no_trailing_newline(&old_lines);
            let new = join_no_trailing_newline(&new_lines);

            let hunks = diff_hunks(&old, &new, DEFAULT_BUDGET);
            let hunks = match hunks {
                Some(h) => h,
                None => continue, // budget exceeded is a valid outcome
            };

            let mut prev_end = 0usize;
            for h in &hunks {
                assert!(
                    h.start >= prev_end,
                    "case {case}: hunks out of order/overlapping"
                );
                assert!(h.start <= h.end, "case {case}: inverted hunk");
                prev_end = h.end;
            }

            let applied = apply_hunks(&old, &hunks);
            assert_eq!(
                applied, new,
                "case {case}: round-trip mismatch (old={old:?} new={new:?})"
            );
        }
    }

    /// A `usize::MAX` budget used to be silently reinterpreted as a
    /// *negative* `i64` inside `myers_trace` (two's-complement
    /// wraparound on `max as i64`), which made the search loop's
    /// `0..=max_d` never execute — `diff_hunks` would wrongly return
    /// `None` for a change well within any sane budget, even a
    /// one-character edit. Not reachable from elisp today
    /// (`replace-region-contents`'s MAX-COST clamps negative input to 0,
    /// never to a huge positive value), but `diff_hunks` is a public
    /// `crate::textdiff` function the M47 spec calls out for direct
    /// reuse, so a future caller passing `usize::MAX` to mean "no real
    /// cap" must not silently get worse-than-no-budget behavior.
    #[test]
    fn budget_near_usize_max_does_not_overflow_into_a_spurious_none() {
        assert_eq!(
            diff_hunks("a\n", "b\n", usize::MAX),
            diff_hunks("a\n", "b\n", DEFAULT_BUDGET)
        );
    }

    #[test]
    fn identical_input_is_empty() {
        let s = "line one\nline two\nline three\n";
        assert_eq!(diff_hunks(s, s, DEFAULT_BUDGET), Some(Vec::new()));
    }

    #[test]
    fn hunks_are_increasing_and_non_overlapping() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nX\nc\nY\ne\n";
        let hunks = diff_hunks(old, new, DEFAULT_BUDGET).unwrap();
        assert!(
            hunks.len() >= 2,
            "expected at least 2 separate hunks, got {hunks:?}"
        );
        let mut prev_end = 0;
        for h in &hunks {
            assert!(h.start >= prev_end);
            prev_end = h.end;
        }
    }

    #[test]
    fn budget_zero_gives_up_on_any_change() {
        assert_eq!(diff_hunks("a\n", "b\n", 0), None);
    }

    #[test]
    fn budget_zero_still_fine_when_unchanged() {
        assert_eq!(diff_hunks("a\n", "a\n", 0), Some(Vec::new()));
    }

    /// Reformatting an entire file (every line touched) should exceed a
    /// tight budget and fall back to `None`, per the module docs.
    #[test]
    fn whole_file_reformat_exceeds_small_budget() {
        let old: String = (0..50).map(|i| format!("line{i}\n")).collect();
        let new: String = (0..50).map(|i| format!("LINE{i}\n")).collect();
        assert_eq!(diff_hunks(&old, &new, 10), None);
    }

    /// Anti-degeneration lock: if this ever collapses to "just strip the
    /// common prefix/suffix", a file with two scattered one-line changes
    /// would produce a single hunk spanning almost the whole file instead
    /// of two tight single-line hunks. This test is the executable
    /// justification for phase 2 (Myers) existing at all.
    #[test]
    fn scattered_edits_stay_as_separate_tight_hunks() {
        let mut old_lines: Vec<String> = (0..200).map(|i| format!("line{i}")).collect();
        old_lines[1] = "line1 CHANGED".to_string();
        old_lines[198] = "line198 CHANGED".to_string();
        let new_lines: Vec<String> = (0..200)
            .map(|i| match i {
                1 => "line1 EDITED".to_string(),
                198 => "line198 EDITED".to_string(),
                n => format!("line{n}"),
            })
            .collect();
        let old = join(&old_lines);
        let new = join(&new_lines);

        let hunks = diff_hunks(&old, &new, DEFAULT_BUDGET).unwrap();
        assert_eq!(
            hunks.len(),
            2,
            "expected exactly 2 hunks, got {}: {hunks:?}",
            hunks.len()
        );

        for h in &hunks {
            let covered = old[h.start..h.end].matches('\n').count();
            assert!(covered <= 1, "hunk covers more than one line: {h:?}");
        }
    }

    #[test]
    fn char_level_trim_shrinks_to_actual_change() {
        // Verilog alignment case from the spec: only whitespace changes.
        // `a;` itself is untouched by either edit, so a cursor sitting on
        // `a` must fall outside every hunk regardless of exactly how the
        // common prefix/suffix trim splits the collapsed whitespace run
        // (that split is inherently non-unique; only "does it stay tight
        // and leave `a;` alone" is a real invariant).
        let old = "logic [3:0]     a;\n";
        let new = "logic [3:0] a;\n";
        let hunks = diff_hunks(old, new, DEFAULT_BUDGET).unwrap();
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        // The changed region should be just (part of) the collapsed
        // whitespace run, not the whole line.
        assert!(h.end - h.start <= 5, "hunk not tightly trimmed: {h:?}");
        let a_pos = old.find("a;").unwrap();
        assert!(h.end <= a_pos, "hunk reaches into `a;`: {h:?}");
        assert!(
            old[h.start..h.end].chars().all(|c| c == ' '),
            "hunk isn't pure whitespace: {h:?}"
        );
    }
}
