//! M21: the bottom selector panel — opens with the minibuffer for file
//! (C-x C-f) and buffer (C-x b / C-x k) prompts, listing candidates in
//! the bottom third of the frame. File rows reuse the dired listing
//! core, so C-x C-f shows the same colored ls -al columns as dired.

use std::cell::RefCell;
use std::rc::Rc;

use elisp::{Interp, Value};

use crate::complete::{self, Source};
use crate::editor::{Editor, PanelRow, PanelState};

/// Rebuild the panel from the current minibuffer input. Selection
/// resets to the first row. No-op for sources the panel doesn't serve.
pub fn refresh(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    let Some((source, input)) = ({
        let editor = ed.borrow();
        editor.minibuffer.as_ref().map(|mb| {
            let spec = &mb.pending.specs[mb.pending.index];
            (complete::source_for_spec(spec), mb.input.clone())
        })
    }) else {
        return;
    };
    let state = match source {
        Source::File => Some(file_rows(&input)),
        Source::Buffer => Some(buffer_rows(interp, ed, &input)),
        Source::Custom(ref cands) => Some(custom_rows(cands, &input)),
        // M84 D4: Command/Function/Symbol (M-x and friends) used to have
        // no panel at all — TAB opened the old popup (`CompletionState`)
        // instead, which is why typing used to clear the candidate list
        // rather than filter it live. Routing them through the same
        // panel they now share with File/Buffer/Custom gets them
        // type-as-you-filter for free via the keystroke refresh below.
        s @ (Source::Command | Source::Function | Source::Symbol) => {
            Some(name_rows(interp, ed, s, &input))
        }
        Source::None => None,
    };
    if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
        mb.panel = state;
    }
}

fn file_rows(input: &str) -> PanelState {
    let (dir_part, stem) = match input.rfind('/') {
        Some(i) => (&input[..i + 1], &input[i + 1..]),
        None => ("", input),
    };
    let stem_start = dir_part.chars().count();
    let dir = complete::expand_file_input(if dir_part.is_empty() { "." } else { dir_part });
    // M22: no remote listing in the panel — an ssh round trip per
    // keystroke would block typing (documented).
    if crate::remote::parse(&dir).is_some() {
        return PanelState {
            rows: Vec::new(),
            selected: 0,
            chosen: false,
            stem_start,
            source: Source::File,
        };
    }
    // "." and ".." are dired furniture, not completion candidates —
    // they would poison the common-prefix computation.
    let files: Vec<crate::dired::FileInfo> = crate::dired::list_dir(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.name != "." && f.name != ".." && f.name.starts_with(stem))
        .collect();
    let widths = crate::dired::ColWidths::of(&files);
    let rows = files
        .iter()
        .map(|fi| PanelRow {
            accept: if fi.is_dir {
                format!("{}/", fi.name)
            } else {
                fi.name.clone()
            },
            segments: crate::dired::format_row(fi, &widths),
        })
        .collect();
    PanelState {
        rows,
        selected: 0,
        chosen: false,
        stem_start,
        source: Source::File,
    }
}

fn buffer_rows(interp: &Interp, ed: &Rc<RefCell<Editor>>, input: &str) -> PanelState {
    let editor = ed.borrow();
    // Current buffer first (the required default selection), then the
    // rest in creation order.
    let mut bufs: Vec<Rc<RefCell<crate::buffer::Buffer>>> = vec![editor.current.clone()];
    for b in &editor.buffers {
        if !Rc::ptr_eq(b, &editor.current) {
            bufs.push(b.clone());
        }
    }
    // F2 (M84 fix round): orderless (`complete::orderless_rank`) rather
    // than plain `starts_with` -- this used to be the only remaining
    // `Source` still on the pre-M84 single-token prefix filter, missed
    // because `mb.panel` was already populated for `Source::Buffer`
    // before M84 (M21), so `minibuffer_tab`'s `complete::candidates`
    // fallback (which DOES go through orderless via `filter_sorted`)
    // was unreachable for this source -- the panel path here never
    // called it. Deliberately NOT re-sorted into rank-0/rank-1 tiers
    // like `filter_sorted` does for Command/Function/Symbol: this list
    // must keep "current buffer first" (`bufs`'s own order, set up
    // above), which a rank-based re-sort could break the moment the
    // current buffer is only a rank-1 (substring, non-prefix) match.
    let rows = bufs
        .iter()
        .filter(|b| complete::orderless_rank(&b.borrow().name, input).is_some())
        .map(|b| {
            let bb = b.borrow();
            let mode = match &bb.major_mode {
                Value::Sym(id) => interp.sym_name(*id).to_string(),
                _ => "Fundamental".to_string(),
            };
            let place = place_for(
                bb.file.as_deref(),
                bb.default_directory.as_deref(),
                std::env::var("HOME").ok().as_deref(),
            );
            let mut segments = vec![(bb.name.clone(), "panel-buffer-name")];
            if bb.modified {
                segments.push((" *".to_string(), "panel-modified"));
            }
            segments.push((format!("  {}", mode), "panel-buffer-mode"));
            if !place.is_empty() {
                segments.push((format!("  {}", place), "panel-buffer-file"));
            }
            PanelRow {
                accept: bb.name.clone(),
                segments,
            }
        })
        .collect();
    PanelState {
        rows,
        selected: 0,
        chosen: false,
        stem_start: 0,
        source: Source::Buffer,
    }
}

/// M61: the buffer-list panel's "place" column — `.file` (now always
/// absolute; find-file-internal normalizes it) falling back to
/// `default_directory`, then HOME abbreviated to `~` for display. No
/// further relativization: this list spans buffers from different
/// directories, so "relative to what" would be ambiguous and could make
/// two same-named files look alike.
///
/// HOME is injectable (rather than this function reading
/// `std::env::var("HOME")` itself) so a test can verify the abbreviation
/// actually fires for a path under HOME without touching the real,
/// process-global `HOME` env var — a fake home prefix is enough to
/// exercise the same code path.
///
/// Takes borrowed paths rather than owned `String`s so the caller doesn't
/// have to clone both fields to ask about one: `.file` wins whenever it's
/// `Some`, and cloning `default_directory` to pass it in would be wasted
/// work on every row of every panel refresh (which runs per keystroke
/// while filtering).
fn place_for(file: Option<&str>, default_directory: Option<&str>, home: Option<&str>) -> String {
    file.or(default_directory)
        .map(|p| crate::complete::abbreviate_home_with(p, home))
        .unwrap_or_default()
}

/// M84 D4: rows for the Command/Function/Symbol sources, filtered
/// through `complete::candidates` (orderless + sorted, same as TAB would
/// compute — `Source::Command`/`Function`/`Symbol` share the D6 session
/// cache, so this costs nothing extra beyond the first keystroke of the
/// session).
fn name_rows(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
    source: Source,
    input: &str,
) -> PanelState {
    let (stem_start, filtered) = complete::candidates(interp, ed, source.clone(), input);
    let rows = filtered
        .into_iter()
        .map(|cand| PanelRow {
            accept: cand.clone(),
            segments: vec![(cand, "panel-buffer-name")],
        })
        .collect();
    PanelState {
        rows,
        selected: 0,
        chosen: false,
        stem_start,
        source,
    }
}

/// M47: rows for a `completing-read` caller-supplied collection, filtered
/// through `complete::custom_filter` (prefix matches, then substring
/// matches, both in the collection's own order — see that function's
/// doc for why this doesn't sort like `file_rows`/`buffer_rows`).
fn custom_rows(cands: &Rc<Vec<String>>, input: &str) -> PanelState {
    let filtered = complete::custom_filter(cands, input);
    let rows = filtered
        .into_iter()
        .map(|cand| PanelRow {
            accept: cand.clone(),
            segments: vec![(cand, "panel-buffer-name")],
        })
        .collect();
    PanelState {
        rows,
        selected: 0,
        chosen: false,
        stem_start: 0,
        source: Source::Custom(cands.clone()),
    }
}

/// Move the panel selection by `delta` (wrapping). Returns true if a
/// panel consumed the motion.
pub fn move_selection(ed: &Rc<RefCell<Editor>>, delta: i64) -> bool {
    let mut editor = ed.borrow_mut();
    let Some(mb) = editor.minibuffer.as_mut() else {
        return false;
    };
    let Some(panel) = mb.panel.as_mut() else {
        return false;
    };
    let n = panel.rows.len();
    if n > 0 {
        panel.selected = (panel.selected as i64 + delta).rem_euclid(n as i64) as usize;
        panel.chosen = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // M61 fixup: `buffer_panel_place_shows_full_absolute_path_outside_home`
    // (panel_tests.rs) alone can't tell "abbreviate_home_with was called
    // and was a no-op because the fixture is outside home" apart from
    // "abbreviate_home_with was never called at all" — both produce the
    // same full absolute path. This test exercises `place_for` directly
    // with an injected fake home, so it actually observes the
    // abbreviation firing without touching the real, process-global
    // `HOME` env var.
    #[test]
    fn place_for_abbreviates_a_path_under_the_injected_home() {
        assert_eq!(
            place_for(Some("/home/u/rtl/top/x.sv"), None, Some("/home/u")),
            "~/rtl/top/x.sv"
        );
    }

    #[test]
    fn place_for_leaves_a_path_outside_the_injected_home_untouched() {
        assert_eq!(
            place_for(Some("/other/rtl/x.sv"), None, Some("/home/u")),
            "/other/rtl/x.sv"
        );
    }

    #[test]
    fn place_for_falls_back_to_default_directory_when_no_file() {
        assert_eq!(
            place_for(None, Some("/home/u/scratch/"), Some("/home/u")),
            "~/scratch/"
        );
    }
}
