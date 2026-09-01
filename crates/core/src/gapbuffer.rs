/// Gap buffer over UTF-8 text. All public positions are 0-based **char**
/// offsets (the elisp layer converts to Emacs' 1-based positions) — the
/// same contract as before M43, even though the internal storage is now
/// `Vec<u8>` rather than `Vec<char>`. `Anchor` (below) is how char
/// positions get translated to the byte offsets the storage actually
/// needs; `char_to_byte`/`byte_to_char`/`as_strs` expose the byte view to
/// callers (regex/search, in a later migration period) who want to work
/// in bytes directly.
///
/// Safety note: char<->byte conversion goes through a hand-written-but-
/// safe decoder (`decode_char_at_raw`), and `as_strs` (no hot-path caller
/// as of M43 period 4 — see its doc comment) still validates via checked
/// `std::str::from_utf8`, per this codebase's safety-first stance on
/// `unsafe` (see also `treesit.rs`'s narrow-unsafe convention):
/// `from_utf8_unchecked` is a data-driven decision, paid only where
/// profiling shows the checked path costs something worth cutting, not a
/// blanket period-1 shortcut. `slice`/`to_string` — which back
/// `Buffer::search_text`'s snapshot rebuild, the hottest full-buffer
/// `&str` reconstruction path in the codebase — did clear that bar in
/// period 4 (measured ~20-23% of snapshot-rebuild time on a 10MB buffer)
/// and use `from_utf8_unchecked`; see the SAFETY comment on `slice` for
/// the invariant argument and the measurement.
pub struct GapBuffer {
    /// UTF-8 bytes. `data[..gap_start]` and `data[gap_end..]` are each
    /// independently valid UTF-8. The gap itself, `data[gap_start..
    /// gap_end]`, holds arbitrary filler bytes — it's never read as
    /// content, only as spare capacity `insert` copies into.
    data: Vec<u8>,
    /// Byte offset; always sits on a char boundary. A multi-byte char's
    /// encoded bytes never straddle the gap: `move_gap` only ever
    /// targets byte offsets computed from char positions, which are
    /// naturally boundary-aligned by construction.
    gap_start: usize,
    /// Byte offset; same char-boundary invariant as `gap_start`.
    gap_end: usize,
    /// Char count of `data[..gap_start]` + `data[gap_end..]` combined.
    /// Maintained incrementally by `insert`/`delete` so `len()` stays
    /// O(1) — its pre-M43 contract — even though the underlying bytes
    /// aren't one-to-one with chars anymore.
    len_chars: usize,
    /// Total newline count in the buffer (M24; unchanged by M43 — still
    /// maintained incrementally by `insert`/`delete`, just counted over
    /// bytes now instead of `char`s).
    newline_count: usize,
    /// Most-recent-query hint: `(char_pos, byte_pos, line_number)`, all
    /// three referring to the same logical position and kept mutually
    /// consistent. Direct successor to the pre-M43 `line_cache:
    /// Cell<(pos, line)>` hint (see `resolve`/`resolve_byte`), extended
    /// with `byte_pos` so char<->byte conversions get the same "scan
    /// only the delta since the last query" amortization line-number
    /// lookups already had. A `Cell` because it's updated from `&self`
    /// query methods.
    anchor: std::cell::Cell<Anchor>,
}

/// See `GapBuffer::anchor`.
#[derive(Clone, Copy)]
struct Anchor {
    char_pos: usize,
    byte_pos: usize,
    line: usize,
}

/// Return type of `GapBuffer::raw_slices`: two `(logical_start_byte,
/// slice)` pieces, in logical order.
type RawSlicePair<'a> = ((usize, &'a [u8]), (usize, &'a [u8]));

impl GapBuffer {
    pub fn new() -> GapBuffer {
        GapBuffer::from_str("")
    }

    /// Infallible constructor; intentionally not the FromStr trait.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> GapBuffer {
        const INITIAL_GAP: usize = 128;
        let mut len_chars = 0usize;
        let mut newline_count = 0usize;
        for c in s.chars() {
            len_chars += 1;
            if c == '\n' {
                newline_count += 1;
            }
        }
        // Gap content doesn't need to be valid UTF-8 — it's never read as
        // content, only overwritten by `insert` — so plain zero bytes are
        // fine filler (unlike the pre-M43 `Vec<char>` version, which had
        // to fill with an actual `char` value).
        let mut data: Vec<u8> = Vec::with_capacity(s.len() + INITIAL_GAP);
        data.resize(INITIAL_GAP, 0u8);
        data.extend_from_slice(s.as_bytes());
        GapBuffer {
            data,
            gap_start: 0,
            gap_end: INITIAL_GAP,
            len_chars,
            newline_count,
            anchor: std::cell::Cell::new(Anchor {
                char_pos: 0,
                byte_pos: 0,
                line: 1,
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.len_chars
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total logical byte length (content with the gap excised). O(1),
    /// like `len()`.
    fn len_bytes(&self) -> usize {
        self.data.len() - (self.gap_end - self.gap_start)
    }

    /// `len_chars == len_bytes()` iff every char in the buffer is 1 byte
    /// — i.e. the whole buffer is ASCII (M43 design §3.3). Both sides are
    /// O(1), so this is a single comparison, and it's checked fresh on
    /// every conversion rather than cached: it's already O(1) to compute,
    /// and its answer changes (a multi-byte char inserted or deleted)
    /// exactly when a cached answer would need invalidating anyway.
    /// When true, char position == byte position everywhere, so
    /// `char_to_byte`/`byte_to_char`/`char_at` all skip the anchor
    /// entirely.
    fn is_ascii_fast_path(&self) -> bool {
        self.len_chars == self.len_bytes()
    }

    /// Map a logical byte offset (content with the gap excised) to its
    /// raw index into `data`. Byte analogue of the pre-M43 char-indexed
    /// `index()`.
    fn index_byte(&self, byte_pos: usize) -> usize {
        if byte_pos < self.gap_start {
            byte_pos
        } else {
            byte_pos + (self.gap_end - self.gap_start)
        }
    }

    /// Whether logical byte offset `byte_pos` sits on a char boundary —
    /// either the end of the buffer, or a non-continuation byte. Used
    /// only in `debug_assert!`s guarding the invariant that every byte
    /// offset this module computes and hands to `move_gap`/slicing is
    /// boundary-aligned: a misaligned offset is a silent-corruption bug,
    /// not a panic, so these asserts are the earliest possible trip-wire
    /// (M43 design §5 risk #1).
    fn is_char_boundary_at(&self, byte_pos: usize) -> bool {
        let len_bytes = self.len_bytes();
        if byte_pos >= len_bytes {
            return byte_pos == len_bytes;
        }
        let raw = self.index_byte(byte_pos);
        (self.data[raw] & 0xC0) != 0x80
    }

    /// Byte length (1..=4) of the UTF-8-encoded char whose leading byte
    /// is `lead`.
    fn utf8_len(lead: u8) -> usize {
        if lead & 0x80 == 0 {
            1
        } else if lead & 0xE0 == 0xC0 {
            2
        } else if lead & 0xF0 == 0xE0 {
            3
        } else {
            4
        }
    }

    pub fn char_at(&self, pos: usize) -> Option<char> {
        if pos >= self.len_chars {
            return None;
        }
        if self.is_ascii_fast_path() {
            // Every char is 1 byte, so char position == byte position;
            // skip the anchor entirely (§3.3).
            return Some(self.data[self.index_byte(pos)] as char);
        }
        let anchor = self.resolve(pos);
        Some(self.decode_char_at_raw(self.index_byte(anchor.byte_pos)))
    }

    /// Decode the char starting at raw index `raw` in `data` without
    /// building a `&str` — a hand-written but safe decoder (see the
    /// module-level safety note) for the single-char case `char_at`
    /// needs. Per the char-boundary invariant, a char's bytes are never
    /// split by the gap, so a plain slice read is always in-bounds and
    /// contiguous.
    fn decode_char_at_raw(&self, raw: usize) -> char {
        let lead = self.data[raw];
        let len = Self::utf8_len(lead);
        let bytes = &self.data[raw..raw + len];
        let cp: u32 = match len {
            1 => lead as u32,
            2 => ((lead as u32 & 0x1F) << 6) | (bytes[1] as u32 & 0x3F),
            3 => {
                ((lead as u32 & 0x0F) << 12)
                    | ((bytes[1] as u32 & 0x3F) << 6)
                    | (bytes[2] as u32 & 0x3F)
            }
            _ => {
                ((lead as u32 & 0x07) << 18)
                    | ((bytes[1] as u32 & 0x3F) << 12)
                    | ((bytes[2] as u32 & 0x3F) << 6)
                    | (bytes[3] as u32 & 0x3F)
            }
        };
        debug_assert!(
            char::from_u32(cp).is_some(),
            "GapBuffer invariant violated: invalid UTF-8 decoded at raw index {raw}"
        );
        char::from_u32(cp).unwrap_or(char::REPLACEMENT_CHARACTER)
    }

    /// Split the logical byte range `[a, b)` into at most two
    /// `(logical_start, slice)` pieces, in order — the byte-storage
    /// shape of the pre-M43 `count_newlines_range`'s gap-splitting.
    /// `[a, b)` maps to a single piece when it's entirely before the gap
    /// or entirely after it, and to both when it straddles `gap_start`.
    /// The second piece is empty (and its `logical_start` meaningless)
    /// in the non-straddling case.
    fn raw_slices(&self, a: usize, b: usize) -> RawSlicePair<'_> {
        debug_assert!(a <= b && b <= self.len_bytes());
        if b <= self.gap_start {
            ((a, &self.data[a..b]), (b, &[]))
        } else if a >= self.gap_start {
            let shift = self.gap_end - self.gap_start;
            ((a, &self.data[a + shift..b + shift]), (b, &[]))
        } else {
            let shift = self.gap_end - self.gap_start;
            (
                (a, &self.data[a..self.gap_start]),
                (self.gap_start, &self.data[self.gap_end..b + shift]),
            )
        }
    }

    /// Count char boundaries (non-continuation bytes) and `\n` bytes in
    /// one pass over a raw slice, 8 bytes at a time via the SWAR bit
    /// trick the M43 design calls out (§1.4/§3.2): read each 8-byte
    /// chunk as a `u64` of 8 packed byte lanes, and for each of the two
    /// "does this lane match X" tests (continuation-byte pattern, `\n`),
    /// use the classic branchless `haszero` idiom to turn "which lanes
    /// match" into a bitmask whose `count_ones()` — one hardware popcount
    /// instruction — is exactly the per-chunk match count. This is a
    /// genuine single pass over the bytes (unlike an earlier version of
    /// this function that ran `str::from_utf8` + `chars().count()` + a
    /// separate `\n` filter — three passes that, despite each being a
    /// fast std primitive, measurably lost to one pass of plain SWAR on
    /// the far-jump `line_number` perf probe). Falls back to a scalar
    /// loop for the final <8-byte remainder.
    fn scan_slice(bytes: &[u8]) -> (usize, usize) {
        const CONT_MASK: u64 = 0xC0C0_C0C0_C0C0_C0C0;
        const CONT_TARGET: u64 = 0x8080_8080_8080_8080;
        const NEWLINE_TARGET: u64 = 0x0A0A_0A0A_0A0A_0A0A;

        // Bit-twiddling-hacks' classic `haszero`: bit 7 of lane `i` in
        // the result is set iff lane `i` of `v` is the zero byte —
        // correct regardless of the borrow chain the subtraction
        // produces, because a non-zero lane always has its own bit 7
        // cleared in `!v`, masking out any borrow "noise" from
        // neighboring lanes.
        fn haszero(v: u64) -> u64 {
            v.wrapping_sub(0x0101_0101_0101_0101) & !v & 0x8080_8080_8080_8080
        }

        // `as_chunks::<8>` rather than `chunks_exact(8)`: the array-typed
        // chunks drop the `try_into().expect(...)` this loop used to need
        // per iteration (the length is in the type now). Switched when
        // Rust 1.98's `clippy::chunks_exact_to_as_chunks` started firing
        // here; behaviour is identical, `remainder` is just `tail`.
        let (chunks, tail) = bytes.as_chunks::<8>();
        let mut chars = 0usize;
        let mut newlines = 0usize;
        for chunk in chunks {
            let v = u64::from_ne_bytes(*chunk);
            let continuation_lanes = haszero((v & CONT_MASK) ^ CONT_TARGET).count_ones();
            chars += 8 - continuation_lanes as usize;
            newlines += haszero(v ^ NEWLINE_TARGET).count_ones() as usize;
        }
        for &b in tail {
            if (b & 0xC0) != 0x80 {
                chars += 1;
            }
            if b == b'\n' {
                newlines += 1;
            }
        }
        (chars, newlines)
    }

    /// Scan the logical byte range `[a, b)` (`a <= b <= len_bytes()`),
    /// counting char boundaries and `\n` bytes in one pass over each of
    /// the (at most two) gap-split slices — the byte-range-driven
    /// counterpart to `scan_chars_forward`/`backward`'s char-count-driven
    /// scan. Rewrite of the pre-M43 `count_newlines_range`, which only
    /// needed to report the newline count (chars == bytes in the old
    /// `Vec<char>` world, so there was nothing else to compute); now that
    /// bytes and chars have diverged, the same single pass reports both.
    fn scan_range(&self, a: usize, b: usize) -> (usize, usize) {
        let ((_, s1), (_, s2)) = self.raw_slices(a, b);
        let (c1, n1) = Self::scan_slice(s1);
        let (c2, n2) = Self::scan_slice(s2);
        (c1 + c2, n1 + n2)
    }

    /// Copy the logical byte range `[a, b)` out of the two gap-split
    /// slices into an owned `Vec<u8>` — two `memcpy`s, no per-char
    /// collection. Shared by `slice`/`to_string`/`delete`.
    fn copy_range(&self, a: usize, b: usize) -> Vec<u8> {
        let ((_, s1), (_, s2)) = self.raw_slices(a, b);
        let mut out = Vec::with_capacity(s1.len() + s2.len());
        out.extend_from_slice(s1);
        out.extend_from_slice(s2);
        out
    }

    fn move_gap(&mut self, byte_pos: usize) {
        debug_assert!(byte_pos <= self.len_bytes());
        debug_assert!(
            self.is_char_boundary_at(byte_pos),
            "move_gap target {byte_pos} is not on a char boundary"
        );
        if byte_pos == self.gap_start {
            return;
        }
        if byte_pos < self.gap_start {
            // Shift [byte_pos, gap_start) right to end of gap.
            let count = self.gap_start - byte_pos;
            self.data
                .copy_within(byte_pos..self.gap_start, self.gap_end - count);
            self.gap_start = byte_pos;
            self.gap_end -= count;
        } else {
            // Shift [gap_end, gap_end + (byte_pos - gap_start)) left to gap_start.
            let count = byte_pos - self.gap_start;
            self.data
                .copy_within(self.gap_end..self.gap_end + count, self.gap_start);
            self.gap_start += count;
            self.gap_end += count;
        }
    }

    fn ensure_gap(&mut self, needed: usize) {
        let gap = self.gap_end - self.gap_start;
        if gap >= needed {
            return;
        }
        let grow = (needed - gap).max(self.data.len() / 2).max(64);
        let old_end = self.gap_end;
        self.data
            .splice(old_end..old_end, std::iter::repeat_n(0u8, grow));
        self.gap_end += grow;
    }

    pub fn insert(&mut self, pos: usize, s: &str) -> usize {
        let pos = pos.min(self.len_chars);
        let mut count = 0usize;
        let mut inserted_newlines = 0usize;
        for c in s.chars() {
            count += 1;
            if c == '\n' {
                inserted_newlines += 1;
            }
        }
        let byte_len = s.len();
        let byte_pos = self.char_to_byte(pos);
        self.ensure_gap(byte_len);
        self.move_gap(byte_pos);
        self.data[self.gap_start..self.gap_start + byte_len].copy_from_slice(s.as_bytes());
        self.gap_start += byte_len;
        self.newline_count += inserted_newlines;
        self.len_chars += count;
        self.adjust_anchor_on_insert(pos, count, byte_len, inserted_newlines);
        count
    }

    /// Delete [start, end), returning the removed text.
    pub fn delete(&mut self, start: usize, end: usize) -> String {
        let end = end.min(self.len_chars);
        let start = start.min(end);
        let byte_start = self.char_to_byte(start);
        let byte_end = self.char_to_byte(end);
        let removed_bytes = self.copy_range(byte_start, byte_end);
        let removed_newlines = removed_bytes.iter().filter(|&&b| b == b'\n').count();
        let removed_chars = end - start;
        self.move_gap(byte_start);
        self.gap_end += byte_end - byte_start;
        self.newline_count -= removed_newlines;
        self.len_chars -= removed_chars;
        self.adjust_anchor_on_delete(start, end, byte_start, byte_end, removed_newlines);
        String::from_utf8(removed_bytes)
            .expect("GapBuffer invariant violated: deleted range was not valid UTF-8")
    }

    /// Keep the anchor correct across an insertion of `n_chars` chars
    /// (`n_bytes` bytes, containing `newlines` of them) at `char_pos`.
    /// If the edit is entirely at-or-before the anchored position, the
    /// anchor's three fields all shift by the amount the edit
    /// contributes; positions strictly after the anchor aren't affected
    /// at all. Byte-aware extension of the pre-M43
    /// `adjust_cache_on_insert` — same two-way branch, one more field.
    fn adjust_anchor_on_insert(
        &self,
        char_pos: usize,
        n_chars: usize,
        n_bytes: usize,
        newlines: usize,
    ) {
        let a = self.anchor.get();
        if char_pos <= a.char_pos {
            self.anchor.set(Anchor {
                char_pos: a.char_pos + n_chars,
                byte_pos: a.byte_pos + n_bytes,
                line: a.line + newlines,
            });
        }
    }

    /// Keep the anchor correct across a deletion of `[char_start,
    /// char_end)` chars (`[byte_start, byte_end)` bytes, containing
    /// `newlines` newlines). Mirrors `adjust_anchor_on_insert`; a
    /// deletion that straddles the anchored position can't be adjusted
    /// cheaply (the anchored position no longer exists), so it resets to
    /// the always-correct `(0, 0, 1)` anchor rather than risk a wrong
    /// answer. Byte-aware extension of the pre-M43
    /// `adjust_cache_on_delete`.
    fn adjust_anchor_on_delete(
        &self,
        char_start: usize,
        char_end: usize,
        byte_start: usize,
        byte_end: usize,
        newlines: usize,
    ) {
        let a = self.anchor.get();
        if a.char_pos <= char_start {
            // Entirely before the deletion: untouched.
        } else if a.char_pos >= char_end {
            self.anchor.set(Anchor {
                char_pos: a.char_pos - (char_end - char_start),
                byte_pos: a.byte_pos - (byte_end - byte_start),
                line: a.line - newlines,
            });
        } else {
            self.anchor.set(Anchor {
                char_pos: 0,
                byte_pos: 0,
                line: 1,
            });
        }
    }

    /// Resolve `target_char` (clamped to `len()`) to a full `Anchor`,
    /// scanning only the delta between the cached anchor and the target
    /// (M43 design §3.2) — and leaves the anchor updated to the result,
    /// so a sequence of nearby queries (the common redisplay/point-
    /// tracking access pattern) is amortized O(1) rather than O(distance)
    /// each time. Backs `char_at`, `char_to_byte`, and `line_number`.
    fn resolve(&self, target_char: usize) -> Anchor {
        let target_char = target_char.min(self.len_chars);
        let a = self.anchor.get();
        let result = match target_char.cmp(&a.char_pos) {
            std::cmp::Ordering::Equal => a,
            std::cmp::Ordering::Greater => {
                let (byte_delta, newlines) =
                    self.scan_chars_forward(a.byte_pos, target_char - a.char_pos);
                Anchor {
                    char_pos: target_char,
                    byte_pos: a.byte_pos + byte_delta,
                    line: a.line + newlines,
                }
            }
            std::cmp::Ordering::Less => {
                let (byte_delta, newlines) =
                    self.scan_chars_backward(a.byte_pos, a.char_pos - target_char);
                Anchor {
                    char_pos: target_char,
                    byte_pos: a.byte_pos - byte_delta,
                    line: a.line - newlines,
                }
            }
        };
        debug_assert!(self.is_char_boundary_at(result.byte_pos));
        self.anchor.set(result);
        result
    }

    /// Byte-driven counterpart to `resolve`: resolve `target_byte`
    /// (clamped to the total byte length) using the same anchor, scanning
    /// the byte range between anchor and target with `scan_range` rather
    /// than walking char boundaries one at a time. Backs `byte_to_char`.
    fn resolve_byte(&self, target_byte: usize) -> Anchor {
        let target_byte = target_byte.min(self.len_bytes());
        let a = self.anchor.get();
        let result = match target_byte.cmp(&a.byte_pos) {
            std::cmp::Ordering::Equal => a,
            std::cmp::Ordering::Greater => {
                let (chars, newlines) = self.scan_range(a.byte_pos, target_byte);
                Anchor {
                    char_pos: a.char_pos + chars,
                    byte_pos: target_byte,
                    line: a.line + newlines,
                }
            }
            std::cmp::Ordering::Less => {
                let (chars, newlines) = self.scan_range(target_byte, a.byte_pos);
                Anchor {
                    char_pos: a.char_pos - chars,
                    byte_pos: target_byte,
                    line: a.line - newlines,
                }
            }
        };
        debug_assert!(self.is_char_boundary_at(result.byte_pos));
        self.anchor.set(result);
        result
    }

    /// From logical byte position `start`, advance past exactly `n` char
    /// boundaries (fewer if the buffer ends first), returning the number
    /// of bytes advanced and the number of `\n` bytes crossed. The
    /// char-count-driven counterpart to `scan_range`'s byte-range-driven
    /// scan — used when the anchor needs to answer "where does char
    /// position X live in bytes" rather than "how many chars/newlines are
    /// in this byte range".
    ///
    /// Stepping one char at a time (`walk_chars_forward`) would be
    /// correct but, for a far jump across a multi-megabyte span, far too
    /// slow: every step pays an `index_byte` branch plus a UTF-8-length
    /// decode, none of which the compiler can vectorize the way it can
    /// `scan_range`'s tight byte loop. So instead: estimate the byte
    /// distance for `n` chars from the buffer's average bytes-per-char
    /// (`len_bytes()/len_chars()`, both O(1)), verify it with one
    /// `scan_range` call (vectorizable), and only fall back to
    /// `walk_chars_forward`/`backward` for whatever small residual the
    /// estimate under- or overshoots by. The residual stays small as
    /// long as multi-byte chars are reasonably evenly distributed over
    /// the scanned span — true of prose, source code, and the
    /// periodic-CJK-lines perf fixture this module is profiled against
    /// alike; a pathologically clustered buffer would degrade toward the
    /// char-by-char cost, but not below correctness.
    fn scan_chars_forward(&self, start: usize, n: usize) -> (usize, usize) {
        if n == 0 {
            return (0, 0);
        }
        if n == 1 {
            // The single most common step size (redisplay's char-by-char
            // cursor, point advancing by one on self-insert): skip the
            // estimate-probe-verify machinery below and decode exactly
            // one leading byte — O(1) with a tiny constant — instead of
            // a `scan_range` SWAR pass plus a possible residual walk to
            // resolve what's already known to be a single-char step.
            return self.walk_chars_forward(start, 1);
        }
        let len_bytes = self.len_bytes();
        let remaining_bytes = len_bytes - start;
        let guess = (n * len_bytes.max(1) / self.len_chars.max(1)).clamp(n, remaining_bytes);
        let mut probe = start + guess;
        while probe < len_bytes && !self.is_char_boundary_at(probe) {
            probe += 1;
        }
        let (chars, mut newlines) = self.scan_range(start, probe);
        let mut end = probe;
        match chars.cmp(&n) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => {
                let (extra_bytes, extra_newlines) = self.walk_chars_forward(end, n - chars);
                end += extra_bytes;
                newlines += extra_newlines;
            }
            std::cmp::Ordering::Greater => {
                let (back_bytes, back_newlines) = self.walk_chars_backward(end, chars - n);
                end -= back_bytes;
                newlines -= back_newlines;
            }
        }
        (end - start, newlines)
    }

    /// Backward counterpart to `scan_chars_forward`: from logical byte
    /// position `start`, retreat past exactly `n` char boundaries (fewer
    /// if the buffer start is reached first). Same estimate-then-verify
    /// shape, mirrored.
    fn scan_chars_backward(&self, start: usize, n: usize) -> (usize, usize) {
        if n == 0 {
            return (0, 0);
        }
        if n == 1 {
            // See `scan_chars_forward`'s n == 1 fast path: same rationale,
            // mirrored for the backward direction (redisplay's backward
            // cursor motion, `delete-backward-char`, ...).
            return self.walk_chars_backward(start, 1);
        }
        let guess = (n * self.len_bytes().max(1) / self.len_chars.max(1)).clamp(n, start);
        let mut probe = start - guess;
        while probe > 0 && !self.is_char_boundary_at(probe) {
            probe -= 1;
        }
        let (chars, mut newlines) = self.scan_range(probe, start);
        let mut begin = probe;
        match chars.cmp(&n) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => {
                let (extra_bytes, extra_newlines) = self.walk_chars_backward(begin, n - chars);
                begin -= extra_bytes;
                newlines += extra_newlines;
            }
            std::cmp::Ordering::Greater => {
                let (fwd_bytes, fwd_newlines) = self.walk_chars_forward(begin, chars - n);
                begin += fwd_bytes;
                newlines -= fwd_newlines;
            }
        }
        (start - begin, newlines)
    }

    /// Char-by-char fallback used only for the (typically small) residual
    /// `scan_chars_forward`'s byte-per-char estimate leaves uncorrected:
    /// advance past exactly `n` char boundaries one at a time (fewer if
    /// the buffer ends first), returning the bytes advanced and `\n`
    /// bytes crossed.
    fn walk_chars_forward(&self, start: usize, n: usize) -> (usize, usize) {
        let mut pos = start;
        let mut newlines = 0usize;
        let len_bytes = self.len_bytes();
        for _ in 0..n {
            if pos >= len_bytes {
                break;
            }
            let lead = self.data[self.index_byte(pos)];
            if lead == b'\n' {
                newlines += 1;
            }
            pos += Self::utf8_len(lead);
        }
        (pos - start, newlines)
    }

    /// Backward counterpart to `walk_chars_forward`, used only for
    /// `scan_chars_backward`'s residual correction. Each step walks back
    /// over any UTF-8 continuation bytes to find the previous char's
    /// leading byte.
    fn walk_chars_backward(&self, start: usize, n: usize) -> (usize, usize) {
        let mut pos = start;
        let mut newlines = 0usize;
        for _ in 0..n {
            if pos == 0 {
                break;
            }
            let mut prev = pos - 1;
            while prev > 0 && (self.data[self.index_byte(prev)] & 0xC0) == 0x80 {
                prev -= 1;
            }
            if self.data[self.index_byte(prev)] == b'\n' {
                newlines += 1;
            }
            pos = prev;
        }
        (start - pos, newlines)
    }

    /// Byte offset corresponding to `char_pos` (clamped to `len()`). O(1)
    /// in the all-ASCII fast path (§3.3); otherwise amortized
    /// O(distance from the last query) via the anchor (§3.2).
    pub fn char_to_byte(&self, char_pos: usize) -> usize {
        let char_pos = char_pos.min(self.len_chars);
        if self.is_ascii_fast_path() {
            return char_pos;
        }
        self.resolve(char_pos).byte_pos
    }

    /// Char offset corresponding to `byte_pos` (clamped to the total byte
    /// length). O(1) in the all-ASCII fast path; otherwise amortized
    /// O(distance) via the anchor.
    pub fn byte_to_char(&self, byte_pos: usize) -> usize {
        let byte_pos = byte_pos.min(self.len_bytes());
        if self.is_ascii_fast_path() {
            return byte_pos;
        }
        self.resolve_byte(byte_pos).char_pos
    }

    /// The buffer's two gap-split contiguous segments as validated
    /// `&str`s (before the gap, after the gap) — the zero-copy view
    /// callers who want bytes (regex/search, in a later migration period)
    /// can use directly. Uses checked `std::str::from_utf8` rather than
    /// `from_utf8_unchecked` — see the module-level safety note.
    pub fn as_strs(&self) -> (&str, &str) {
        let before = std::str::from_utf8(&self.data[..self.gap_start])
            .expect("GapBuffer invariant violated: bytes before the gap are not valid UTF-8");
        let after = std::str::from_utf8(&self.data[self.gap_end..])
            .expect("GapBuffer invariant violated: bytes after the gap are not valid UTF-8");
        (before, after)
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        let end = end.min(self.len_chars);
        let start = start.min(end);
        let byte_start = self.char_to_byte(start);
        let byte_end = self.char_to_byte(end);
        let bytes = self.copy_range(byte_start, byte_end);
        // SAFETY: `bytes` is `data[..gap_start]` ++ `data[gap_end..]`
        // (`copy_range`/`raw_slices`) sliced to `byte_start..byte_end`.
        // The struct invariant (see the doc comment on `data`) guarantees
        // each half is independently valid UTF-8, and `char_to_byte`
        // always returns a byte offset on a char boundary — the anchor is
        // only ever resolved *to* a char position, and every write path
        // (`insert`/`delete`/`move_gap`) keeps `gap_start`/`gap_end` on
        // char boundaries too, so a multi-byte char's encoding can never
        // straddle `byte_start` or `byte_end`. Slicing at two char
        // boundaries out of a valid-UTF-8 sequence yields a valid-UTF-8
        // sequence, so `bytes` is valid UTF-8.
        //
        // This was the checked `String::from_utf8` (see `git blame`)
        // through M43 period 3, per this codebase's "narrow,
        // safety-first" bias on `unsafe` (module doc, and `treesit.rs`'s
        // convention): pay the safe cost until profiling shows it's
        // worth cutting. Period 4 measured it: on a 10MB buffer, the
        // checked validation alone (isolated from the `copy_range` memcpy
        // it sits on top of) cost ~330µs, ~20-23% of the total
        // `search_text()` snapshot-rebuild time that backs every
        // isearch/regex call after an edit — comfortably over the >5%
        // bar the M43 design set for paying this `unsafe`, so it's cut
        // here. Debug builds still pay the check, below.
        debug_assert!(
            std::str::from_utf8(&bytes).is_ok(),
            "GapBuffer invariant violated: sliced range was not valid UTF-8"
        );
        unsafe { String::from_utf8_unchecked(bytes) }
    }

    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.slice(0, self.len_chars)
    }

    /// Iterator over the chars from `char_pos` (clamped to `len()`)
    /// through the end of the buffer. Converts the char position to a
    /// byte position exactly once, via `char_to_byte` (which does the
    /// anchor lookup — see `resolve`), then decodes forward one char at
    /// a time straight off the raw bytes (`index_byte`/
    /// `decode_char_at_raw` — the same safe, non-`&str`-constructing
    /// technique `char_at` uses) — no further anchor lookups per char,
    /// and (important, see below) no `&str` construction at all. Exists
    /// for callers that used to do `char_at(pos); pos += 1` in a loop
    /// (redisplay's per-cell render loop, its row-boundary scan): each
    /// `char_at` call there re-pays the anchor resolution from scratch,
    /// where a single `chars_from` call up front and then plain
    /// `Iterator::next()` per step does the position -> byte conversion
    /// exactly once for the whole walk.
    ///
    /// Deliberately does *not* go through `as_strs`'s
    /// `chars()`-over-two-`&str`-segments approach, even though that
    /// reads cleaner: `as_strs` calls `std::str::from_utf8`, which
    /// validates its *entire* argument slice up front before handing
    /// back a `&str` — an O(buffer size) cost, paid in full on every
    /// `chars_from` call regardless of how much of the iterator the
    /// caller actually consumes. `slice`/`to_string` pay a comparable
    /// O(buffer size) cost up front too (the `copy_range` memcpy; the
    /// `from_utf8` validation that used to sit on top of it was swapped
    /// for `from_utf8_unchecked` in M43 period 4, see the SAFETY comment
    /// on `slice` — a data-driven call, not a reason to make `chars_from`
    /// pay full-buffer cost as well), and that's a fine trade for their
    /// callers (chiefly `Buffer::search_text`'s snapshot cache): already
    /// O(buffer size) themselves and already amortized (rebuilt once per
    /// edit-tick, not once per keystroke). It is not a fine trade here:
    /// redisplay calls `chars_from` roughly once per `render()` — i.e.
    /// once per keystroke — but only ever consumes a small, screen-sized
    /// prefix of what the iterator promises to walk to the end of the
    /// buffer. An earlier version of this function did use `as_strs`,
    /// and measured ~700µs to read 3000 chars from a 10MB buffer this
    /// way — almost entirely `from_utf8` validating the ~10MB
    /// before/after segments up front on a call that only needed the
    /// first 3000 chars — which is exactly the "per-keystroke O(buffer
    /// size) cost" class of regression P1.2's search-snapshot caching
    /// exists to avoid. Manual per-char decoding keeps the cost O(1) per
    /// char actually pulled from `next()`, with nothing proportional to
    /// buffer size paid up front.
    pub fn chars_from(&self, char_pos: usize) -> impl Iterator<Item = char> + '_ {
        let char_pos = char_pos.min(self.len_chars);
        let byte_pos = self.char_to_byte(char_pos);
        CharsFrom {
            buf: self,
            byte_pos,
            len_bytes: self.len_bytes(),
        }
    }

    /// Start of the line containing pos (position after the previous newline).
    pub fn line_start(&self, pos: usize) -> usize {
        let pos = pos.min(self.len_chars);
        let byte_pos = self.char_to_byte(pos);
        let ((off1, s1), (off2, s2)) = self.raw_slices(0, byte_pos);
        if let Some(i) = s2.iter().rposition(|&b| b == b'\n') {
            return self.byte_to_char(off2 + i + 1);
        }
        if let Some(i) = s1.iter().rposition(|&b| b == b'\n') {
            return self.byte_to_char(off1 + i + 1);
        }
        0
    }

    /// End of the line containing pos (position of the newline, or buffer end).
    pub fn line_end(&self, pos: usize) -> usize {
        let pos = pos.min(self.len_chars);
        let byte_pos = self.char_to_byte(pos);
        let ((off1, s1), (off2, s2)) = self.raw_slices(byte_pos, self.len_bytes());
        if let Some(i) = s1.iter().position(|&b| b == b'\n') {
            return self.byte_to_char(off1 + i);
        }
        if let Some(i) = s2.iter().position(|&b| b == b'\n') {
            return self.byte_to_char(off2 + i);
        }
        self.len_chars
    }

    /// 1-based line number at pos. Scans only between the anchored
    /// position and `pos` (see `anchor`/`resolve`), so repeated queries
    /// near the same spot — the common redisplay pattern — are
    /// O(distance) rather than O(pos).
    pub fn line_number(&self, pos: usize) -> usize {
        self.resolve(pos).line
    }

    /// Total number of lines in the buffer (1-based: an empty buffer, or
    /// one with no trailing newline, still counts as 1). O(1) — backed by
    /// `newline_count`, which `insert`/`delete` maintain incrementally.
    pub fn total_lines(&self) -> usize {
        self.newline_count + 1
    }
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Backs `GapBuffer::chars_from` — see that method's doc comment for why
/// this decodes directly off the raw bytes (`index_byte`/
/// `decode_char_at_raw`) instead of iterating a validated `&str`
/// (`as_strs`). `byte_pos` is a logical byte offset (gap excised, like
/// every other byte position in this module); `index_byte`/
/// `decode_char_at_raw` are private methods of `GapBuffer`, reachable
/// here because Rust's privacy is module-scoped, not type-scoped — this
/// struct lives in the same module.
struct CharsFrom<'a> {
    buf: &'a GapBuffer,
    byte_pos: usize,
    len_bytes: usize,
}

impl Iterator for CharsFrom<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if self.byte_pos >= self.len_bytes {
            return None;
        }
        let raw = self.buf.index_byte(self.byte_pos);
        let c = self.buf.decode_char_at_raw(raw);
        self.byte_pos += c.len_utf8();
        Some(c)
    }
}

/// `Some(c)` iff `needle` is exactly one **char** long — not one byte
/// long; a single CJK char is 1 char but 3 bytes, and must still take
/// the single-char fast path below — `None` for an empty or multi-char
/// needle. Backs `find_forward`/`find_backward`'s single-char
/// specialization (M43 period 3): `str::find`/`str::rfind` with a
/// `char` pattern measured ~15-20x faster than the same search with a
/// length-1 `&str` pattern — libstd's `Pattern` impl for `&str` always
/// goes through the general Two-Way search machinery, which has
/// disproportionate setup overhead for a 1-char needle, while the
/// `char` impl has a dedicated fast search. Matters for isearch's first
/// keystroke (a 1-char query) and any regex whose literal prefix is a
/// single char (see `elisp::regex`'s `literal_prefix_char`).
fn single_char(needle: &str) -> Option<char> {
    let mut chars = needle.chars();
    let c = chars.next()?;
    if chars.next().is_none() {
        Some(c)
    } else {
        None
    }
}

/// First BYTE index >= `from` in `hay` where `needle` occurs (M43
/// period 2 — was a char-indexed `&[char]` scan; `from`/the return value
/// are now byte offsets, and callers convert to/from char positions via
/// `char_to_byte`/`byte_to_char`). Delegates straight to `str::find`
/// (libstd's Two-Way/memchr substring search), which is both simpler
/// and substantially faster than the hand-rolled quick-reject loop this
/// replaces — ~7.3x on a realistic buffer size (M43 design doc §1.4) —
/// and, since `needle` is a real `&str`, is guaranteed to only ever
/// report a match at a char boundary of `hay` (UTF-8
/// self-synchronization — see `elisp::regex`'s module doc for the full
/// argument). A single-char needle instead dispatches to `str::find`
/// with a `char` pattern (see `single_char`) — same guarantee, much
/// faster. Shared by `search-forward` and isearch's forward step,
/// both of which search against `Buffer::search_text`'s cached
/// full-buffer snapshot rather than a fresh per-call slice.
/// An empty needle matches at `from` itself, mirroring `str::find("")`.
/// `from` must be on a char boundary of `hay` (all callers derive it
/// from a char position via `char_to_byte`); an off-boundary `from`
/// returns `None` rather than panicking (`str::get` is checked).
pub fn find_forward(hay: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    let start = from.min(hay.len());
    if let Some(c) = single_char(needle) {
        return hay.get(start..)?.find(c).map(|off| start + off);
    }
    hay.get(start..)?.find(needle).map(|off| start + off)
}

/// Last BYTE index in `hay` where `needle` occurs entirely before
/// `before` (i.e. `index + needle.len() <= before`) — the byte-indexed
/// (M43 period 2) backward-search counterpart to `find_forward`, used by
/// isearch's backward step. Truncating `hay` to `..before` before
/// calling `str::rfind` gives exactly the "fully before `before`"
/// contract in one call: any match `rfind` finds in the truncated slice
/// necessarily ends at or before `before`, and `rfind` reports the
/// rightmost (last) such match — the same "last index in range" result
/// the old char-indexed scan computed by hand.
/// An empty needle matches at `before` itself, mirroring `str::rfind("")`.
/// `before` must be on a char boundary of `hay`; off-boundary returns
/// `None` rather than panicking. A single-char needle dispatches to
/// `str::rfind` with a `char` pattern (see `single_char`), same as
/// `find_forward`.
pub fn find_backward(hay: &str, needle: &str, before: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(before.min(hay.len()));
    }
    let limit = before.min(hay.len());
    if let Some(c) = single_char(needle) {
        return hay.get(..limit)?.rfind(c);
    }
    hay.get(..limit)?.rfind(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Self-contained copy of the pre-M43 `Vec<char>`-backed GapBuffer,
    /// kept only as a differential-testing oracle for the byte-backed
    /// `GapBuffer` above (house convention: P1.6/P2.2 used the same
    /// "freeze the old implementation as an oracle" trick). Does not
    /// share any field or method with the new `GapBuffer` — a bug in the
    /// byte rewrite has to independently reproduce here to go unnoticed
    /// by the dogfight below.
    mod oracle {
        pub struct CharGapBuffer {
            data: Vec<char>,
            gap_start: usize,
            gap_end: usize,
            newline_count: usize,
            line_cache: std::cell::Cell<(usize, usize)>,
        }

        impl CharGapBuffer {
            pub fn new() -> CharGapBuffer {
                CharGapBuffer::from_str("")
            }

            #[allow(clippy::should_implement_trait)]
            pub fn from_str(s: &str) -> CharGapBuffer {
                const INITIAL_GAP: usize = 128;
                let mut data: Vec<char> = Vec::with_capacity(s.chars().count() + INITIAL_GAP);
                data.extend(std::iter::repeat_n(' ', INITIAL_GAP));
                let mut newline_count = 0usize;
                for c in s.chars() {
                    if c == '\n' {
                        newline_count += 1;
                    }
                    data.push(c);
                }
                CharGapBuffer {
                    data,
                    gap_start: 0,
                    gap_end: INITIAL_GAP,
                    newline_count,
                    line_cache: std::cell::Cell::new((0, 1)),
                }
            }

            pub fn len(&self) -> usize {
                self.data.len() - (self.gap_end - self.gap_start)
            }

            fn index(&self, pos: usize) -> usize {
                if pos < self.gap_start {
                    pos
                } else {
                    pos + (self.gap_end - self.gap_start)
                }
            }

            pub fn char_at(&self, pos: usize) -> Option<char> {
                if pos >= self.len() {
                    return None;
                }
                Some(self.data[self.index(pos)])
            }

            fn count_newlines_range(&self, a: usize, b: usize) -> usize {
                debug_assert!(a <= b && b <= self.len());
                if b <= self.gap_start {
                    self.data[a..b].iter().filter(|&&c| c == '\n').count()
                } else if a >= self.gap_start {
                    let shift = self.gap_end - self.gap_start;
                    self.data[a + shift..b + shift]
                        .iter()
                        .filter(|&&c| c == '\n')
                        .count()
                } else {
                    let before = self.data[a..self.gap_start]
                        .iter()
                        .filter(|&&c| c == '\n')
                        .count();
                    let shift = self.gap_end - self.gap_start;
                    let after = self.data[self.gap_end..b + shift]
                        .iter()
                        .filter(|&&c| c == '\n')
                        .count();
                    before + after
                }
            }

            fn move_gap(&mut self, pos: usize) {
                debug_assert!(pos <= self.len());
                if pos == self.gap_start {
                    return;
                }
                if pos < self.gap_start {
                    let count = self.gap_start - pos;
                    for i in (0..count).rev() {
                        self.data[self.gap_end - count + i] = self.data[pos + i];
                    }
                    self.gap_start = pos;
                    self.gap_end -= count;
                } else {
                    let count = pos - self.gap_start;
                    for i in 0..count {
                        self.data[self.gap_start + i] = self.data[self.gap_end + i];
                    }
                    self.gap_start += count;
                    self.gap_end += count;
                }
            }

            fn ensure_gap(&mut self, needed: usize) {
                let gap = self.gap_end - self.gap_start;
                if gap >= needed {
                    return;
                }
                let grow = (needed - gap).max(self.data.len() / 2).max(64);
                let old_end = self.gap_end;
                self.data
                    .splice(old_end..old_end, std::iter::repeat_n(' ', grow));
                self.gap_end += grow;
            }

            pub fn insert(&mut self, pos: usize, s: &str) -> usize {
                let count = s.chars().count();
                self.ensure_gap(count);
                self.move_gap(pos);
                let mut inserted_newlines = 0usize;
                for c in s.chars() {
                    if c == '\n' {
                        inserted_newlines += 1;
                    }
                    self.data[self.gap_start] = c;
                    self.gap_start += 1;
                }
                self.newline_count += inserted_newlines;
                self.adjust_cache_on_insert(pos, count, inserted_newlines);
                count
            }

            pub fn delete(&mut self, start: usize, end: usize) -> String {
                let end = end.min(self.len());
                let start = start.min(end);
                let removed: String = (start..end).filter_map(|p| self.char_at(p)).collect();
                let removed_newlines = removed.chars().filter(|&c| c == '\n').count();
                self.move_gap(start);
                self.gap_end += end - start;
                self.newline_count -= removed_newlines;
                self.adjust_cache_on_delete(start, end, removed_newlines);
                removed
            }

            fn adjust_cache_on_insert(&self, ins_pos: usize, n: usize, newlines: usize) {
                let (last_pos, last_line) = self.line_cache.get();
                if ins_pos <= last_pos {
                    self.line_cache.set((last_pos + n, last_line + newlines));
                }
            }

            fn adjust_cache_on_delete(&self, start: usize, end: usize, newlines: usize) {
                let (last_pos, last_line) = self.line_cache.get();
                if last_pos <= start {
                    // Entirely before the deletion: untouched.
                } else if last_pos >= end {
                    self.line_cache
                        .set((last_pos - (end - start), last_line - newlines));
                } else {
                    self.line_cache.set((0, 1));
                }
            }

            pub fn slice(&self, start: usize, end: usize) -> String {
                let end = end.min(self.len());
                let start = start.min(end);
                (start..end).filter_map(|p| self.char_at(p)).collect()
            }

            #[allow(clippy::inherent_to_string)]
            pub fn to_string(&self) -> String {
                self.slice(0, self.len())
            }

            pub fn line_number(&self, pos: usize) -> usize {
                let pos = pos.min(self.len());
                let (last_pos, last_line) = self.line_cache.get();
                let line = match pos.cmp(&last_pos) {
                    std::cmp::Ordering::Equal => last_line,
                    std::cmp::Ordering::Greater => {
                        last_line + self.count_newlines_range(last_pos, pos)
                    }
                    std::cmp::Ordering::Less => {
                        last_line - self.count_newlines_range(pos, last_pos)
                    }
                };
                self.line_cache.set((pos, line));
                line
            }

            pub fn total_lines(&self) -> usize {
                self.newline_count + 1
            }
        }
    }
    use oracle::CharGapBuffer;

    #[test]
    fn insert_and_read() {
        let mut g = GapBuffer::new();
        g.insert(0, "hello");
        assert_eq!(g.to_string(), "hello");
        g.insert(5, " world");
        assert_eq!(g.to_string(), "hello world");
        g.insert(0, ">> ");
        assert_eq!(g.to_string(), ">> hello world");
        g.insert(3, "X");
        assert_eq!(g.to_string(), ">> Xhello world");
        assert_eq!(g.len(), 15);
        assert_eq!(g.char_at(3), Some('X'));
        assert_eq!(g.char_at(999), None);
    }

    #[test]
    fn delete() {
        let mut g = GapBuffer::from_str("hello world");
        assert_eq!(g.delete(5, 11), " world");
        assert_eq!(g.to_string(), "hello");
        let mut g2 = GapBuffer::from_str("abcdef");
        g2.insert(3, "XY");
        assert_eq!(g2.to_string(), "abcXYdef");
        assert_eq!(g2.delete(1, 4), "bcX");
        assert_eq!(g2.to_string(), "aYdef");
    }

    #[test]
    fn find_forward_matches_str_find() {
        let hay = "the quick brown fox jumps over the lazy dog";
        assert_eq!(find_forward(hay, "the", 0), Some(0));
        assert_eq!(find_forward(hay, "the", 1), Some(31));
        assert_eq!(find_forward(hay, "the", 32), None);
        // Empty needle matches at `from` itself, like `str::find("")`.
        assert_eq!(find_forward(hay, "", 5), Some(5));
        assert_eq!(find_forward(hay, "the", hay.len() + 10), None);
    }

    #[test]
    fn find_backward_matches_str_rfind() {
        let hay = "the quick brown fox jumps over the lazy dog";
        // Full buffer: last "the" starts at 31 (before "lazy").
        assert_eq!(find_backward(hay, "the", hay.len()), Some(31));
        // Restrict to before the second "the" ends: only the first
        // occurrence at 0 remains in range.
        assert_eq!(find_backward(hay, "the", 31), Some(0));
        assert_eq!(find_backward(hay, "the", 3), Some(0));
        assert_eq!(find_backward(hay, "the", 2), None);
        assert_eq!(find_backward(hay, "", 5), Some(5));
    }

    /// M43 period 2: multi-byte needle/haystack — byte offsets returned
    /// must land on char boundaries, and (since `find_forward`/
    /// `find_backward` now delegate to `str::find`/`str::rfind`) they
    /// structurally do (UTF-8 self-synchronization — see
    /// `elisp::regex`'s module doc for the full argument). Byte offsets
    /// below are computed by hand from the fixture's known layout, not
    /// re-derived, so a regression that silently shifted a match by one
    /// byte (landing mid-character) would be caught even though it
    /// wouldn't panic.
    #[test]
    fn find_forward_and_backward_multibyte() {
        // byte layout: "a"=0, "中"=1..4, "b"=4, "文"=5..8, "c"=8,
        // "中"=9..12, "d"=12
        let hay = "a中b文c中d";
        assert_eq!(find_forward(hay, "中", 0), Some(1));
        // `from` is a byte offset: 4 is the char boundary right after
        // the first "中" (not a byte offset landing mid-character).
        assert_eq!(find_forward(hay, "中", 4), Some(9));
        assert_eq!(find_forward(hay, "b文", 0), Some(4));
        assert_eq!(find_forward(hay, "z", 0), None);

        assert_eq!(find_backward(hay, "中", hay.len()), Some(9));
        assert_eq!(find_backward(hay, "中", 9), Some(1));
        assert_eq!(find_backward(hay, "中", 1), None);
    }

    /// M43 period 3: pin down `find_forward`/`find_backward`'s
    /// single-char specialization (`single_char`, dispatching to
    /// `str::find`/`str::rfind` with a `char` pattern rather than a
    /// length-1 `&str` one) with hand-computed byte offsets, for both a
    /// single ASCII-char needle and a single CJK-char needle — the fuzz
    /// test above (`find_forward_and_backward_match_naive_scan_under_
    /// fuzzing`) does generate `needle_len == 1` probabilistically, but
    /// doesn't guarantee it on every run, so this locks the behavior
    /// down independent of RNG luck.
    #[test]
    fn find_forward_and_backward_single_char_needle_specialization() {
        // byte layout: "a"=0, "中"=1..4, "a"=4, "文"=5..8, "a"=8; len=9.
        let hay = "a中a文a";
        assert_eq!(hay.len(), 9);

        // Single ASCII-char needle ('a' is 1 char, 1 byte): multiple
        // matches, exercised forward and backward from several points.
        assert_eq!(find_forward(hay, "a", 0), Some(0));
        assert_eq!(find_forward(hay, "a", 1), Some(4));
        assert_eq!(find_forward(hay, "a", 5), Some(8));
        assert_eq!(find_forward(hay, "a", 9), None);

        assert_eq!(find_backward(hay, "a", 9), Some(8));
        assert_eq!(find_backward(hay, "a", 8), Some(4));
        assert_eq!(find_backward(hay, "a", 4), Some(0));
        assert_eq!(find_backward(hay, "a", 0), None);

        // Single CJK-char needle ('中'/'文' are 1 char, 3 bytes each) —
        // must still take the char-count-1 fast path, not a
        // byte-count-1 one.
        assert_eq!(find_forward(hay, "中", 0), Some(1));
        // `from` = 2 lands mid-"中" (not a char boundary of `hay`): the
        // checked `hay.get(start..)?` guard still applies ahead of the
        // single-char dispatch, so this is `None`, not a panic.
        assert_eq!(find_forward(hay, "中", 2), None);
        assert_eq!(find_forward(hay, "文", 0), Some(5));

        assert_eq!(find_backward(hay, "中", 9), Some(1));
        assert_eq!(find_backward(hay, "文", 9), Some(5));
        assert_eq!(find_backward(hay, "中", 1), None);
    }

    #[test]
    fn unicode() {
        let mut g = GapBuffer::from_str("中文abc");
        assert_eq!(g.len(), 5);
        assert_eq!(g.char_at(0), Some('中'));
        g.insert(2, "字");
        assert_eq!(g.to_string(), "中文字abc");
        assert_eq!(g.delete(0, 3), "中文字");
        assert_eq!(g.to_string(), "abc");
    }

    #[test]
    fn lines() {
        let g = GapBuffer::from_str("one\ntwo\nthree");
        assert_eq!(g.line_start(5), 4);
        assert_eq!(g.line_end(5), 7);
        assert_eq!(g.line_start(0), 0);
        assert_eq!(g.line_end(12), 13);
        assert_eq!(g.line_number(0), 1);
        assert_eq!(g.line_number(5), 2);
        assert_eq!(g.line_number(9), 3);
    }

    #[test]
    fn growth() {
        let mut g = GapBuffer::new();
        for i in 0..1000 {
            g.insert(g.len() / 2, &format!("{}", i % 10));
        }
        assert_eq!(g.len(), 1000);
    }

    /// Oracle for `line_number`/`total_lines`: the pre-M24 O(pos) scan,
    /// deliberately bypassing the anchor so it can be used to check the
    /// anchor's answers.
    fn naive_line_number(g: &GapBuffer, pos: usize) -> usize {
        let pos = pos.min(g.len());
        let mut n = 1;
        for p in 0..pos {
            if g.char_at(p) == Some('\n') {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn line_number_empty_buffer() {
        let g = GapBuffer::new();
        assert_eq!(g.line_number(0), 1);
        assert_eq!(g.total_lines(), 1);
    }

    #[test]
    fn total_lines_no_trailing_newline() {
        let g = GapBuffer::from_str("one\ntwo\nthree");
        assert_eq!(g.total_lines(), 3);
        let g2 = GapBuffer::from_str("one\ntwo\nthree\n");
        assert_eq!(g2.total_lines(), 4);
    }

    #[test]
    fn total_lines_tracks_inserts_and_deletes() {
        let mut g = GapBuffer::from_str("one\ntwo\nthree");
        assert_eq!(g.total_lines(), 3);
        g.insert(3, "\nX\nY"); // "one\nX\nY\ntwo\nthree" — 2 more newlines
        assert_eq!(g.total_lines(), 5);
        let removed = g.delete(0, 4); // "one\n"
        assert_eq!(removed, "one\n");
        assert_eq!(g.total_lines(), 4);
    }

    #[test]
    fn cache_stays_valid_for_edits_after_the_cached_position() {
        let mut g = GapBuffer::from_str("one\ntwo\nthree\nfour\n");
        // anchors (4, _, 2)
        assert_eq!(g.line_number(4), 2);
        // Edit strictly after the cached position: anchor must stay
        // valid, not just accidentally correct.
        g.insert(15, "NEW\n");
        assert_eq!(g.line_number(4), 2);
        assert_eq!(naive_line_number(&g, 4), 2);
    }

    #[test]
    fn cache_shifts_for_edits_before_the_cached_position() {
        let mut g = GapBuffer::from_str("one\ntwo\nthree\nfour\n");
        let target = g.line_start(15);
        let before = g.line_number(target); // anchors at `target`
        let prefix = "zero\nzero.5\n";
        g.insert(0, prefix);
        let new_target = target + prefix.chars().count();
        let after = g.line_number(new_target);
        assert_eq!(after, before + 2);
        assert_eq!(after, naive_line_number(&g, new_target));
    }

    #[test]
    fn cache_resets_when_deletion_straddles_it() {
        let mut g = GapBuffer::from_str("one\ntwo\nthree\nfour\n");
        let _ = g.line_number(9); // anchors somewhere inside "three"
        g.delete(2, 12); // deletes a range straddling pos 9
        let expected = naive_line_number(&g, g.len());
        assert_eq!(g.line_number(g.len()), expected);
    }

    /// Naive reference for `find_forward`/`find_backward`: a plain
    /// `windows().position()`/`rposition()` scan with no quick-reject
    /// shortcut, used as the fuzz oracle below.
    fn naive_find_forward(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
        if needle.is_empty() {
            return Some(from.min(hay.len()));
        }
        let start = from.min(hay.len());
        hay[start..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| start + p)
    }

    fn naive_find_backward(hay: &[char], needle: &[char], before: usize) -> Option<usize> {
        if needle.is_empty() {
            return Some(before.min(hay.len()));
        }
        let limit = before.min(hay.len());
        if needle.len() > limit {
            return None;
        }
        hay[..limit]
            .windows(needle.len())
            .rposition(|w| w == needle)
    }

    /// Char position -> byte position in `s` (test-only; production
    /// code gets this from `GapBuffer::char_to_byte`, but these fuzz
    /// cases work over a bare `&str`, not a `GapBuffer`). Clamps past-
    /// the-end positions to `s.len()`, mirroring `find_forward`/
    /// `find_backward`'s own `.min(hay.len())` clamping.
    fn char_to_byte_pos(s: &str, char_pos: usize) -> usize {
        s.char_indices()
            .nth(char_pos)
            .map(|(b, _)| b)
            .unwrap_or(s.len())
    }

    /// Byte position -> char position in `s` (inverse of
    /// `char_to_byte_pos`, test-only).
    fn byte_to_char_pos(s: &str, byte_pos: usize) -> usize {
        s[..byte_pos].chars().count()
    }

    /// Fuzz the production (byte-indexed, M43 period 2) `find_forward`/
    /// `find_backward` against the naive char-indexed oracle above:
    /// alphabet mixes ASCII with multi-byte (CJK) characters so
    /// byte-vs-char divergence is actually exercised, over random
    /// hay/needle/from/before combinations. `from`/`before` are
    /// generated as CHAR positions (matching the oracle's units) and
    /// converted to bytes for the production call; the production
    /// result is converted back to a char position before comparing —
    /// so this is testing unit-conversion-adjusted equivalence, exactly
    /// as `elisp::regex`'s new-vs-legacy-engine dogfight does.
    #[test]
    fn find_forward_and_backward_match_naive_scan_under_fuzzing() {
        let mut rng = Lcg(0xFEED_BEEF);
        let alphabet = ['a', 'b', 'c', '中', '文'];
        for step in 0..2000 {
            let hay_len = rng.range(20);
            let hay_chars: Vec<char> = (0..hay_len)
                .map(|_| alphabet[rng.range(alphabet.len())])
                .collect();
            let needle_len = rng.range(4); // 0..=3, so empty-needle gets exercised too
            let needle_chars: Vec<char> = (0..needle_len)
                .map(|_| alphabet[rng.range(alphabet.len())])
                .collect();
            let hay_str: String = hay_chars.iter().collect();
            let needle_str: String = needle_chars.iter().collect();
            let from = rng.range(hay_len + 2);
            let before = rng.range(hay_len + 2);

            let got_forward = find_forward(&hay_str, &needle_str, char_to_byte_pos(&hay_str, from))
                .map(|b| byte_to_char_pos(&hay_str, b));
            assert_eq!(
                got_forward,
                naive_find_forward(&hay_chars, &needle_chars, from),
                "step {step}: find_forward mismatch, hay={hay_chars:?} needle={needle_chars:?} from={from}"
            );
            let got_backward =
                find_backward(&hay_str, &needle_str, char_to_byte_pos(&hay_str, before))
                    .map(|b| byte_to_char_pos(&hay_str, b));
            assert_eq!(
                got_backward,
                naive_find_backward(&hay_chars, &needle_chars, before),
                "step {step}: find_backward mismatch, hay={hay_chars:?} needle={needle_chars:?} before={before}"
            );
        }
    }

    /// Tiny deterministic PRNG so the fuzz test below is reproducible
    /// without pulling in the `rand` crate.
    struct Lcg(u64);
    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn range(&mut self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                (self.next_u64() % n as u64) as usize
            }
        }
    }

    /// Random-edit dogfight (M43: expanded from a `line_number`/
    /// `total_lines`-only comparison into a full dual-implementation
    /// differential test). Drives a byte-backed `GapBuffer` and the
    /// frozen `CharGapBuffer` oracle through an identical random
    /// insert/delete sequence (fixed seed, for reproducibility) and
    /// compares every public-facing query after each step: `to_string`,
    /// `len`, `char_at` at 8 probes, `line_number` at 8 probes, `slice`
    /// over random sub-ranges, `total_lines`, and `char_to_byte`/
    /// `byte_to_char` round-trip consistency (cross-checked against a
    /// byte offset derived independently from the oracle's `to_string()`,
    /// since `CharGapBuffer` has no byte concept of its own). Catches
    /// anchor-adjustment bugs and char<->byte conversion bugs alike —
    /// e.g. edits landing exactly on the anchored position, edits on
    /// both sides of the gap, or an off-by-one in a UTF-8 length
    /// calculation — that hand-picked scenarios miss.
    #[test]
    fn dual_impl_dogfight_matches_oracle_under_random_edits() {
        let mut g = GapBuffer::new();
        let mut o = CharGapBuffer::new();
        let mut rng = Lcg(0xC0FFEE);
        let chunks = [
            "a",
            "b\n",
            "hello\nworld\n",
            "\n",
            "xyz",
            "中文\n",
            "\n\n\n",
            "word ",
            "",
            "\n\n\n\n\n\n\n\n",
            "line one\nline two\nline three\nline four\n",
            "no newline here at all just plain text padding",
            "端\n末\n",
            "\n\n",
        ];
        for step in 0..1500 {
            let len = g.len();
            assert_eq!(len, o.len(), "step {step}: len diverged before edit");
            if len == 0 || rng.range(2) == 0 {
                let pos = rng.range(len + 1);
                let chunk = chunks[rng.range(chunks.len())];
                g.insert(pos, chunk);
                o.insert(pos, chunk);
            } else {
                let start = rng.range(len);
                let max_del = (len - start).min(20);
                let del_len = rng.range(max_del + 1);
                g.delete(start, start + del_len);
                o.delete(start, start + del_len);
            }

            let glen = g.len();
            let oracle_str = o.to_string();
            assert_eq!(glen, o.len(), "step {step}: len mismatch");
            assert_eq!(g.to_string(), oracle_str, "step {step}: to_string mismatch");
            assert_eq!(
                g.total_lines(),
                o.total_lines(),
                "step {step}: total_lines mismatch"
            );

            // char_at at 8 probes: 0, len, and 6 random positions (which,
            // across 1500 steps, land near the gap plenty often).
            let mut probes = vec![0usize, glen];
            for _ in 0..6 {
                probes.push(rng.range(glen + 1));
            }
            for &p in &probes {
                assert_eq!(
                    g.char_at(p),
                    o.char_at(p),
                    "step {step}: char_at({p}) mismatch"
                );
            }

            // line_number at 8 probes.
            probes.push(rng.range(glen + 1));
            probes.push(rng.range(glen + 1));
            for &p in probes.iter().take(8) {
                assert_eq!(
                    g.line_number(p),
                    o.line_number(p),
                    "step {step}: line_number({p}) mismatch"
                );
            }

            // slice over 2 random sub-ranges.
            for _ in 0..2 {
                let a = rng.range(glen + 1);
                let b = rng.range(glen + 1);
                let (a, b) = (a.min(b), a.max(b));
                assert_eq!(
                    g.slice(a, b),
                    o.slice(a, b),
                    "step {step}: slice({a},{b}) mismatch"
                );
            }

            // char_to_byte/byte_to_char round-trip, cross-checked against
            // a byte offset computed independently from the oracle's
            // `to_string()` (the oracle has no byte concept of its own).
            for _ in 0..2 {
                let p = rng.range(glen + 1);
                let expected_byte: usize = oracle_str.chars().take(p).map(char::len_utf8).sum();
                let byte = g.char_to_byte(p);
                assert_eq!(
                    byte, expected_byte,
                    "step {step}: char_to_byte({p}) mismatch"
                );
                assert_eq!(
                    g.byte_to_char(byte),
                    p,
                    "step {step}: byte_to_char(char_to_byte({p})) round-trip mismatch"
                );

                // Adjacent (n==1) stepping, forward and backward, cross-
                // checked against the same independently-computed oracle
                // byte offsets — exercises `scan_chars_forward`/
                // `scan_chars_backward`'s n==1 fast path (M43 period 3)
                // across the dogfight's already-diverse random gap
                // positions and anchor states. `char_to_byte(p)` just
                // above left the anchor at exactly `p`, so the `p+1`
                // call below is a genuine forward n=1 step; re-seating
                // the anchor at `p` again before the `p-1` call makes
                // that one a genuine backward n=1 step (rather than a
                // n=2 step relative to wherever `p+1` left the anchor).
                if p < glen {
                    let p_next = p + 1;
                    let expected_next: usize =
                        oracle_str.chars().take(p_next).map(char::len_utf8).sum();
                    assert_eq!(
                        g.char_to_byte(p_next),
                        expected_next,
                        "step {step}: char_to_byte({p_next}) [p+1, n==1 fast path] mismatch"
                    );
                }
                if p > 0 {
                    let _ = g.char_to_byte(p); // re-seat the anchor at p
                    let p_prev = p - 1;
                    let expected_prev: usize =
                        oracle_str.chars().take(p_prev).map(char::len_utf8).sum();
                    assert_eq!(
                        g.char_to_byte(p_prev),
                        expected_prev,
                        "step {step}: char_to_byte({p_prev}) [p-1, n==1 fast path] mismatch"
                    );
                }
            }
        }
    }

    /// Naive reference for `scan_range`: recomputes `(chars, newlines)`
    /// for `[a, b)` from `to_string()`. This test's fixture is pure
    /// ASCII, so byte and char positions coincide and `a`/`b` can be used
    /// directly as `str` byte indices into `to_string()`'s output.
    fn naive_scan_range(g: &GapBuffer, a: usize, b: usize) -> (usize, usize) {
        let s = g.to_string();
        let sub = &s[a..b];
        (sub.chars().count(), sub.matches('\n').count())
    }

    /// Direct boundary checks for `scan_range`'s gap-mapping (M43 rename
    /// of the pre-M43 `count_newlines_range_gap_boundaries`, adjusted for
    /// `scan_range`'s `(chars, newlines)` tuple return and byte-offset
    /// parameters): a range ending exactly at `gap_start` (entirely
    /// before the gap), one starting exactly at `gap_start` (entirely
    /// after), one straddling it, a zero-length range at several
    /// positions (including right at `gap_start`), and a range spanning
    /// the whole buffer. `insert(pos, "")` moves the gap to `pos` without
    /// touching content, so each case can pin down exactly where
    /// `gap_start` lands.
    #[test]
    fn scan_range_gap_boundaries() {
        let mut g = GapBuffer::from_str("one\ntwo\nthree\nfour\nfive\n");
        let len = g.len(); // ASCII-only fixture: char length == byte length.

        // Range ends exactly at gap_start: entirely before the gap.
        g.insert(10, "");
        assert_eq!(g.scan_range(0, 10), naive_scan_range(&g, 0, 10));

        // Range starts exactly at gap_start: entirely after the gap.
        assert_eq!(g.scan_range(10, len), naive_scan_range(&g, 10, len));

        // Range straddles gap_start.
        g.insert(12, "");
        assert_eq!(g.scan_range(5, 20), naive_scan_range(&g, 5, 20));

        // Zero-length ranges: before the gap, at the gap, after the gap.
        assert_eq!(g.scan_range(0, 0), (0, 0));
        assert_eq!(g.scan_range(12, 12), (0, 0));
        assert_eq!(g.scan_range(len, len), (0, 0));

        // Whole-buffer range, with the gap left in the middle.
        assert_eq!(g.scan_range(0, len), naive_scan_range(&g, 0, len));
        assert_eq!(g.scan_range(0, len), (len, g.newline_count));
    }

    #[test]
    fn char_to_byte_and_byte_to_char_basic() {
        let g = GapBuffer::from_str("a中b文c");
        // byte layout: 'a'=1, '中'=3, 'b'=1, '文'=3, 'c'=1
        assert_eq!(g.char_to_byte(0), 0);
        assert_eq!(g.char_to_byte(1), 1); // start of '中'
        assert_eq!(g.char_to_byte(2), 4); // start of 'b'
        assert_eq!(g.char_to_byte(3), 5); // start of '文'
        assert_eq!(g.char_to_byte(4), 8); // start of 'c'
        assert_eq!(g.char_to_byte(5), 9); // end of buffer
        for p in 0..=5 {
            let b = g.char_to_byte(p);
            assert_eq!(g.byte_to_char(b), p, "round-trip at char {p}");
        }
    }

    #[test]
    fn as_strs_reflects_gap_split() {
        let mut g = GapBuffer::from_str("hello world");
        g.insert(5, ""); // move gap to char 5, no content change
        let (before, after) = g.as_strs();
        assert_eq!(before, "hello");
        assert_eq!(after, " world");
        assert_eq!(format!("{before}{after}"), g.to_string());
    }

    /// Boundary case: the gap sits immediately before a multi-byte char
    /// and immediately after one. Exercises the "a multi-byte char's
    /// bytes never straddle the gap" invariant at both edges.
    #[test]
    fn gap_adjacent_to_multibyte_char_boundaries() {
        let mut g = GapBuffer::from_str("ab中文cd");
        // Move the gap to sit right before '中' (char index 2).
        g.insert(2, "");
        assert_eq!(g.char_at(2), Some('中'));
        assert_eq!(g.char_at(1), Some('b'));
        assert_eq!(g.slice(0, g.len()), "ab中文cd");

        // Move the gap to sit right after '中' (char index 3, i.e. right
        // before '文').
        g.insert(3, "");
        assert_eq!(g.char_at(2), Some('中'));
        assert_eq!(g.char_at(3), Some('文'));
        assert_eq!(g.slice(0, g.len()), "ab中文cd");

        // Insert right at the gap and confirm content stays correct.
        g.insert(3, "X");
        assert_eq!(g.to_string(), "ab中X文cd");
    }

    /// Boundary case: an insert lands exactly at the anchored position —
    /// the `char_pos <= a.char_pos` branch of `adjust_anchor_on_insert`
    /// with equality, not strict inequality.
    #[test]
    fn insert_at_anchor_position_matches_oracle() {
        let mut g = GapBuffer::from_str("中文one\ntwo\nthree");
        let mut o = CharGapBuffer::from_str("中文one\ntwo\nthree");
        let anchor_pos = 4;
        assert_eq!(g.line_number(anchor_pos), o.line_number(anchor_pos));
        g.insert(anchor_pos, "XYZ\n");
        o.insert(anchor_pos, "XYZ\n");
        assert_eq!(g.to_string(), o.to_string());
        assert_eq!(g.line_number(anchor_pos), o.line_number(anchor_pos));
        assert_eq!(g.line_number(g.len()), o.line_number(o.len()));
    }

    /// Boundary case: a deletion straddles the anchored position — the
    /// "reset to (0,0,1)" branch of `adjust_anchor_on_delete`.
    #[test]
    fn delete_straddling_anchor_matches_oracle() {
        let mut g = GapBuffer::from_str("中文\none\ntwo\nthree\nfour\n");
        let mut o = CharGapBuffer::from_str("中文\none\ntwo\nthree\nfour\n");
        let anchor_pos = 6; // inside "one\ntwo" somewhere
        assert_eq!(g.line_number(anchor_pos), o.line_number(anchor_pos));
        g.delete(2, 12);
        o.delete(2, 12);
        assert_eq!(g.to_string(), o.to_string());
        assert_eq!(g.line_number(g.len()), o.line_number(o.len()));
        assert_eq!(g.total_lines(), o.total_lines());
    }

    #[test]
    fn empty_buffer_matches_oracle_across_api() {
        let g = GapBuffer::new();
        let o = CharGapBuffer::new();
        assert_eq!(g.len(), o.len());
        assert!(g.is_empty());
        assert_eq!(g.to_string(), o.to_string());
        assert_eq!(g.char_at(0), o.char_at(0));
        assert_eq!(g.line_number(0), o.line_number(0));
        assert_eq!(g.total_lines(), o.total_lines());
        assert_eq!(g.char_to_byte(0), 0);
        assert_eq!(g.byte_to_char(0), 0);
        assert_eq!(g.slice(0, 0), "");
        let (before, after) = g.as_strs();
        assert_eq!(before, "");
        assert_eq!(after, "");
    }

    /// Boundary case: a buffer with no ASCII at all, confirming
    /// correctness off the fast path (every position requires a
    /// non-trivial, anchor-driven conversion).
    #[test]
    fn pure_cjk_buffer_matches_oracle() {
        let text = "春眠不覺曉\n處處聞啼鳥\n夜來風雨聲\n花落知多少\n";
        let mut g = GapBuffer::from_str(text);
        let mut o = CharGapBuffer::from_str(text);
        assert_eq!(g.len(), o.len());
        assert_eq!(g.total_lines(), o.total_lines());
        for p in 0..=g.len() {
            assert_eq!(g.char_at(p), o.char_at(p), "char_at({p})");
            assert_eq!(g.line_number(p), o.line_number(p), "line_number({p})");
        }
        assert_eq!(g.to_string(), o.to_string());
        g.insert(3, "、");
        o.insert(3, "、");
        assert_eq!(g.to_string(), o.to_string());
        let removed_g = g.delete(2, 5);
        let removed_o = o.delete(2, 5);
        assert_eq!(removed_g, removed_o);
        assert_eq!(g.to_string(), o.to_string());
    }

    /// Direct, non-render-pipeline check that `chars_from` matches a
    /// naive "chars from `start` to the end" reference, with the gap
    /// swept across every position in a CJK-mixed buffer and `start`
    /// swept across every position too. Exists alongside `redisplay`'s
    /// `render_cjk_line_with_gap_mid_line_via_chars_from` integration
    /// test (core_tests.rs): that test's one particular buffer/gap/
    /// window-start geometry turned out not to exercise every possible
    /// internal mis-implementation of `CharsFrom::next` — confirmed
    /// empirically while hardening this function (M43 period 3): a
    /// deliberate bug that dropped `index_byte`'s gap-shift inside
    /// `next()` (reading `self.byte_pos` directly as the raw index)
    /// slipped past that specific render test, though it was still
    /// caught elsewhere in the full suite. Sweeping every (gap position,
    /// start position) pair over a CJK-heavy buffer leaves no particular
    /// geometry to get lucky against.
    #[test]
    fn chars_from_matches_naive_suffix_across_all_gap_positions() {
        let text = "ab中文cd春眠不覺曉ef處處聞啼鳥gh";
        let char_len = text.chars().count();
        for gap_pos in 0..=char_len {
            let mut g = GapBuffer::from_str(text);
            g.insert(gap_pos, ""); // move the gap to `gap_pos`, content unchanged
            for start in 0..=char_len {
                let got: String = g.chars_from(start).collect();
                let want: String = text.chars().skip(start).collect();
                assert_eq!(
                    got, want,
                    "gap at char {gap_pos}, chars_from({start}) mismatch"
                );
            }
        }
    }

    /// Boundary case: a buffer starts pure-ASCII (fast path engaged), a
    /// multi-byte insert knocks it out of the fast path, and deleting
    /// that text again should bring `len_chars == len_bytes` back — the
    /// fast path re-engages automatically, with no explicit bookkeeping.
    /// Checked via the only two O(1) quantities observable from outside
    /// the module that the fast-path invariant (`len_chars == total byte
    /// count`) relates: `len()` (chars) and the UTF-8 byte length of
    /// `to_string()`.
    #[test]
    fn ascii_fast_path_engages_and_disengages_automatically() {
        let mut g = GapBuffer::from_str("hello world");
        assert_eq!(g.len(), g.to_string().len(), "pure ASCII: chars == bytes");

        g.insert(5, "中文");
        assert_ne!(g.len(), g.to_string().len(), "mixed buffer: chars != bytes");
        assert_eq!(g.to_string(), "hello中文 world");

        let removed = g.delete(5, 7);
        assert_eq!(removed, "中文");
        assert_eq!(g.to_string(), "hello world");
        assert_eq!(
            g.len(),
            g.to_string().len(),
            "back to pure ASCII: chars == bytes again"
        );
    }
}
