//! M19: file/path predicates and name manipulation (GNU names), plus
//! the dired listing primitive. Rust keeps to primitives; dired.el owns
//! the command layer.

use elisp::builtins::{defun, need_str, opt};
use elisp::{Interp, Value};

use super::{buffer_arg, cur, ed_handle};
use crate::editor::edit_insert;

pub fn register(interp: &mut Interp) {
    // `/ssh:` paths dispatch to a `test -e` round trip (M41 fix: dired's
    // remote `C'/`R' need this to agree with what the remote `cp'/`mv'
    // actually does to an existing-directory destination — see
    // `file-directory-p` just below and dired.el's `dired--dest-is-
    // directory-p`).
    defun(interp, "file-exists-p", 1, Some(1), |i, a| {
        let p = need_str(i, &a[0])?.to_string();
        let p = crate::complete::expand_file_input(&p);
        let exists = match crate::remote::parse(&p) {
            Some(rp) => crate::remote::exists(&rp).map_err(|e| i.error(e))?,
            None => std::path::Path::new(&p).exists(),
        };
        Ok(Value::bool(exists, i.syms.t))
    });
    // `/ssh:` paths dispatch to a `test -d` round trip rather than
    // always returning nil (M41 fix): dired's `dired--dest-is-
    // directory-p' relies on this to tell whether a remote `C'/`R'
    // destination is an existing directory, matching what the remote
    // `cp'/`mv' shell command itself would do with that same
    // destination (drop the file in as DEST/basename) instead of
    // dired's bookkeeping silently disagreeing with it.
    defun(interp, "file-directory-p", 1, Some(1), |i, a| {
        let p = need_str(i, &a[0])?.to_string();
        let p = crate::complete::expand_file_input(&p);
        let is_dir = match crate::remote::parse(&p) {
            Some(rp) => crate::remote::is_dir(&rp).map_err(|e| i.error(e))?,
            None => std::path::Path::new(&p).is_dir(),
        };
        Ok(Value::bool(is_dir, i.syms.t))
    });
    // M39: plain file-name listing (GNU's `directory-files', v1 subset --
    // no FULL/MATCH/NOSORT). Reuses the M19 dired listing core (so `.`
    // and `..` are included, matching GNU's own default, and `/ssh:`
    // dirs work too, though verilog-auto.el's library scan never needs
    // that -- see this file's header note in the M39 report). Unlike
    // `dired-insert-listing`, this touches no buffer at all: callers
    // that just want names (e.g. scanning `verilog-library-directories`)
    // don't need a listing rendered anywhere.
    defun(interp, "directory-files", 1, Some(1), |i, a| {
        let dir = need_str(i, &a[0])?.to_string();
        let dir = crate::complete::expand_file_input(&dir);
        let files = crate::dired::list_dir(&dir)
            .map_err(|e| i.error(format!("Cannot list {}: {}", dir, e)))?;
        Ok(Value::list(
            files.into_iter().map(|f| Value::string(f.name)).collect(),
        ))
    });
    // M39: read a file straight into a Lisp string, no buffer involved
    // (unlike `insert-file-contents', which inserts into the current
    // buffer at point). verilog-auto.el's library-file scan uses this
    // rather than the `with-temp-buffer'-style dance GNU Emacs itself
    // would use, since there's no `with-temp-buffer' macro here and this
    // is both simpler and can't leave a stray buffer behind on error --
    // see the M39 report. GNU has no single built-in with this exact
    // shape to name-match; local paths only, like `directory-files'
    // above.
    defun(interp, "file-contents-as-string", 1, Some(1), |i, a| {
        let p = need_str(i, &a[0])?.to_string();
        let p = crate::complete::expand_file_input(&p);
        let contents = std::fs::read_to_string(&p)
            .map_err(|e| i.error(format!("Cannot read {}: {}", p, e)))?;
        Ok(Value::string(contents))
    });
    defun(interp, "expand-file-name", 1, Some(2), |i, a| {
        let name = need_str(i, &a[0])?.to_string();
        let dir = match opt(a, 1) {
            Value::Nil => None,
            v => Some(need_str(i, &v)?.to_string()),
        };
        Ok(Value::string(expand_file_name(&name, dir.as_deref())))
    });
    defun(interp, "file-name-directory", 1, Some(1), |i, a| {
        let name = need_str(i, &a[0])?.to_string();
        Ok(match name.rfind('/') {
            Some(idx) => Value::string(&name[..idx + 1]),
            None => Value::Nil,
        })
    });
    defun(interp, "file-name-nondirectory", 1, Some(1), |i, a| {
        let name = need_str(i, &a[0])?.to_string();
        Ok(Value::string(match name.rfind('/') {
            Some(idx) => &name[idx + 1..],
            None => &name,
        }))
    });
    defun(interp, "directory-file-name", 1, Some(1), |i, a| {
        let name = need_str(i, &a[0])?.to_string();
        let trimmed = if name.len() > 1 {
            name.trim_end_matches('/')
        } else {
            &name
        };
        Ok(Value::string(if trimmed.is_empty() {
            "/"
        } else {
            trimmed
        }))
    });
    defun(interp, "file-name-as-directory", 1, Some(1), |i, a| {
        let mut name = need_str(i, &a[0])?.to_string();
        if !name.ends_with('/') {
            name.push('/');
        }
        Ok(Value::string(name))
    });
    // v1 deviation from GNU (documented): default-directory is a
    // function pair, not a buffer-local variable.
    defun(interp, "default-directory", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let dd = b.borrow().default_directory.clone();
        Ok(dd.map(Value::string).unwrap_or(Value::Nil))
    });
    defun(interp, "set-default-directory", 1, Some(1), |i, a| {
        let mut dir = need_str(i, &a[0])?.to_string();
        if !dir.ends_with('/') {
            dir.push('/');
        }
        cur(i).borrow_mut().default_directory = Some(dir.clone());
        Ok(Value::string(dir))
    });
    defun(interp, "buffer-read-only-p", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let ro = b.borrow().read_only;
        Ok(Value::bool(ro, i.syms.t))
    });
    defun(interp, "set-buffer-read-only", 1, Some(2), |i, a| {
        let flag = a[0].truthy();
        let b = buffer_arg(i, &opt(a, 1))?;
        b.borrow_mut().read_only = flag;
        Ok(Value::bool(flag, i.syms.t))
    });
    // Insert an ls -al style listing of DIR at point, attach face
    // overlays, and return ((NAME . DIRP) ...) in row order so dired.el
    // maps line -> entry without parsing text (names with spaces stay
    // correct). Respects the read-only guard: dired.el calls this under
    // inhibit-read-only. Each row starts with a two-space mark column
    // (M41) that dired.el's mark commands overwrite in place.
    defun(interp, "dired-insert-listing", 1, Some(1), |i, a| {
        let dir = need_str(i, &a[0])?.to_string();
        let dir = crate::complete::expand_file_input(&dir);
        super::check_writable(i, &cur(i))?;
        let files = crate::dired::list_dir(&dir)
            .map_err(|e| i.error(format!("Cannot list {}: {}", dir, e)))?;
        let widths = crate::dired::ColWidths::of(&files);
        let b = cur(i);
        let ed = ed_handle(i);
        let face_sym = i.intern("face");
        let mut names = Vec::new();
        for fi in &files {
            let mut line_start = b.borrow().point;
            // M41: a two-char mark column ahead of the permissions field
            // (unfaced, so overlay stacking is untouched) — dired.el's
            // `m`/`d`/`D`/`u` overwrite just this one character in place
            // (net position shift 0) rather than reinserting the row.
            line_start += edit_insert(&ed, &b, line_start, "  ");
            for (text, face) in crate::dired::format_row(fi, &widths) {
                let n = edit_insert(&ed, &b, line_start, &text);
                if face != "default" {
                    let face_val = Value::Sym(i.intern(face));
                    let mut bb = b.borrow_mut();
                    let seq = bb.alloc_overlay_seq();
                    let ov =
                        std::rc::Rc::new(std::cell::RefCell::new(crate::buffer::OverlayData {
                            buffer: std::rc::Rc::downgrade(&b),
                            start: line_start,
                            end: line_start + n,
                            props: vec![(face_sym, face_val)],
                            seq,
                        }));
                    bb.insert_overlay(ov);
                }
                line_start += n;
            }
            let n = edit_insert(&ed, &b, line_start, "\n");
            b.borrow_mut().point = line_start + n;
            names.push(Value::cons(
                Value::string(&fi.name),
                Value::bool(fi.is_dir, i.syms.t),
            ));
        }
        Ok(Value::list(names))
    });

    // M41: dired's file-operation primitives (mark/copy/delete/rename is
    // the command layer, in dired.el; these are just the disk-touching
    // core, dispatched local vs `/ssh:' the same way every other M19/M22
    // path-taking builtin in this file does). v1 boundary, documented
    // here and in dired.el's header: SRC and DST must be on the same
    // side (both local, or the same remote host) -- crossing local/
    // remote or remote-host/remote-host is a clean error, not attempted.
    defun(interp, "delete-file", 1, Some(1), |i, a| {
        let p = need_str(i, &a[0])?.to_string();
        let p = crate::complete::expand_file_input(&p);
        if let Some(rp) = crate::remote::parse(&p) {
            crate::remote::remove_file(&rp).map_err(|e| i.error(e))?;
        } else {
            std::fs::remove_file(&p).map_err(|e| i.error(format!("Cannot delete {}: {}", p, e)))?;
        }
        Ok(Value::Nil)
    });
    defun(interp, "delete-directory", 1, Some(2), |i, a| {
        let p = need_str(i, &a[0])?.to_string();
        let p = crate::complete::expand_file_input(&p);
        let recursive = opt(a, 1).truthy();
        if let Some(rp) = crate::remote::parse(&p) {
            crate::remote::remove_dir(&rp, recursive).map_err(|e| i.error(e))?;
        } else if recursive {
            std::fs::remove_dir_all(&p)
                .map_err(|e| i.error(format!("Cannot delete directory {}: {}", p, e)))?;
        } else {
            std::fs::remove_dir(&p)
                .map_err(|e| i.error(format!("Cannot delete directory {}: {}", p, e)))?;
        }
        Ok(Value::Nil)
    });
    // Plain files only (v1: no recursive directory copy — dired.el's
    // callers check the DIRP flag from the listing and skip directories
    // before ever calling this, but this also refuses on its own so a
    // direct call is never silently wrong). DST is overwritten if it
    // already exists (GNU's OK-IF-ALREADY-EXISTS defaults to an error;
    // the dired-driven v1 use here always wants the overwrite, so that's
    // the only mode offered).
    //
    // SRC == DST is refused rather than attempted: `std::fs::copy`
    // truncate-opens DST before it finishes reading SRC, so when they're
    // the same file it silently empties it and returns `Ok(0)` — no
    // error, just data loss (this is exactly what dired's `C` hits when
    // the destination is the marked file's own directory, since the
    // computed dst then equals src). Checked two ways: the expanded path
    // strings (catches the literal-same-path case cheaply, remote paths
    // included) and, for two local paths that both already exist,
    // `std::fs::canonicalize` (catches a symlink or a `..`-laden alias
    // pointing at the same inode; a canonicalize failure — e.g. DST
    // doesn't exist yet, the normal case — just skips this extra check
    // rather than erroring on its own).
    defun(interp, "copy-file", 2, Some(2), |i, a| {
        let src = crate::complete::expand_file_input(&need_str(i, &a[0])?.to_string());
        let dst = crate::complete::expand_file_input(&need_str(i, &a[1])?.to_string());
        if src == dst {
            return Err(i.error(format!(
                "Cannot copy {} to {}: source and destination are the same file",
                src, dst
            )));
        }
        if crate::remote::parse(&src).is_none() && crate::remote::parse(&dst).is_none() {
            if let (Ok(csrc), Ok(cdst)) = (std::fs::canonicalize(&src), std::fs::canonicalize(&dst))
            {
                if csrc == cdst {
                    return Err(i.error(format!(
                        "Cannot copy {} to {}: source and destination are the same file",
                        src, dst
                    )));
                }
            }
        }
        match (crate::remote::parse(&src), crate::remote::parse(&dst)) {
            (None, None) => {
                if std::path::Path::new(&src).is_dir() {
                    return Err(i.error(format!("Cannot copy {}: is a directory", src)));
                }
                std::fs::copy(&src, &dst)
                    .map_err(|e| i.error(format!("Cannot copy {} to {}: {}", src, dst, e)))?;
            }
            (Some(rs), Some(rd)) if rs.host == rd.host => {
                if crate::remote::is_dir(&rs).map_err(|e| i.error(e))? {
                    return Err(i.error(format!(
                        "Cannot copy {}: is a directory",
                        crate::remote::format_path(&rs.host, &rs.path)
                    )));
                }
                crate::remote::copy_file(&rs.host, &rs.path, &rd.path).map_err(|e| i.error(e))?;
            }
            _ => {
                return Err(i.error(format!(
                    "Cannot copy {} to {}: source and destination must be on the same host",
                    src, dst
                )))
            }
        }
        Ok(Value::Nil)
    });
    // Files and directories alike (GNU's `rename-file` covers both, and
    // so does the remote `mv` fallback). Also keeps any buffer already
    // visiting SRC in sync with the move -- GNU Emacs does the same
    // (`rename-file` on a visited file updates that buffer's
    // `buffer-file-name` and its display name) -- reusing
    // find-file-internal's own <n>-suffix de-dup loop (editing.rs) so
    // the new name can't collide with some other already-open buffer.
    defun(interp, "rename-file", 2, Some(2), |i, a| {
        let src = expand_file_name(&need_str(i, &a[0])?.to_string(), None);
        let dst = expand_file_name(&need_str(i, &a[1])?.to_string(), None);
        match (crate::remote::parse(&src), crate::remote::parse(&dst)) {
            (None, None) => {
                std::fs::rename(&src, &dst)
                    .map_err(|e| i.error(format!("Cannot rename {} to {}: {}", src, dst, e)))?;
            }
            (Some(rs), Some(rd)) if rs.host == rd.host => {
                crate::remote::rename_file(&rs.host, &rs.path, &rd.path).map_err(|e| i.error(e))?;
            }
            _ => {
                return Err(i.error(format!(
                    "Cannot rename {} to {}: source and destination must be on the same host",
                    src, dst
                )))
            }
        }
        let ed = ed_handle(i);
        let visiting = ed
            .borrow()
            .buffers
            .iter()
            .find(|b| b.borrow().file.as_deref() == Some(src.as_str()))
            .cloned();
        if let Some(b) = visiting {
            let base = dst.rsplit('/').next().unwrap_or(&dst).to_string();
            let new_name = {
                let editor = ed.borrow();
                // `b` itself is still carrying its old name here (it's
                // renamed below), so a plain `find_buffer(&base)` would
                // find `b` and treat that as a collision with itself —
                // the common case of moving a file to a new directory
                // without changing its basename would then get wrongly
                // saddled with a "<2>" suffix. Excluded by identity
                // (`Rc::ptr_eq`), not by name, since another buffer could
                // legitimately already hold the exact name `base`.
                let collides = editor
                    .find_buffer(&base)
                    .is_some_and(|existing| !std::rc::Rc::ptr_eq(&existing, &b));
                if !collides {
                    base.clone()
                } else {
                    let mut n = 2;
                    loop {
                        let candidate = format!("{}<{}>", base, n);
                        if editor.find_buffer(&candidate).is_none() {
                            break candidate;
                        }
                        n += 1;
                    }
                }
            };
            b.borrow_mut().file = Some(dst);
            b.borrow_mut().name = new_name;
        }
        Ok(Value::Nil)
    });
}

/// GNU expand-file-name semantics (subset): absolute names pass
/// through, `~` expands, relative names join DIR (or cwd), and `.`/`..`
/// components normalize away.
///
/// M61: also the sole normalization choke point for `find-file-internal`
/// (via `editing.rs`'s `expand_path`), so that the same file typed two
/// different ways (relative vs. absolute, or with `../`) normalizes to
/// the same `.file` string and reuses one buffer instead of opening a
/// second one that silently diverges from the first. This is purely
/// string-level (no `fs::canonicalize`): symlinks are not resolved, so
/// two different symlinked names for the same underlying file still open
/// two buffers — a known, documented gap, consistent with GNU Emacs's own
/// default (`find-file-visit-truename` is nil there too).
pub(crate) fn expand_file_name(name: &str, dir: Option<&str>) -> String {
    let expanded = crate::complete::expand_file_input(name);
    let joined = if expanded.starts_with('/') {
        expanded
    } else if dir.map(|d| d.starts_with("/ssh:")).unwrap_or(false) {
        // M22: joining a relative name onto a remote directory.
        format!("{}/{}", dir.unwrap().trim_end_matches('/'), expanded)
    } else {
        let base = match dir {
            Some(d) => crate::complete::expand_file_input(d),
            None => std::env::current_dir()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/".to_string()),
        };
        format!("{}/{}", base.trim_end_matches('/'), expanded)
    };
    // Remote paths skip local normalization (the remote shell resolves
    // its own dots) — "." and ".." are rare in typed remote paths.
    if joined.starts_with("/ssh:") {
        return joined;
    }
    let mut parts: Vec<&str> = Vec::new();
    for comp in joined.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_file_name_normalizes() {
        assert_eq!(expand_file_name("/a/b/../c", None), "/a/c");
        assert_eq!(expand_file_name("b/./c", Some("/x")), "/x/b/c");
        assert_eq!(expand_file_name("/a//b", None), "/b"); // GNU // shadowing
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_file_name("~/z", None), format!("{}/z", home));
    }

    // M61 T8a: `/ssh:` paths pass through untouched — the remote shell
    // owns any `.`/`..` in them, not this local normalizer.
    #[test]
    fn expand_file_name_ssh_passthrough() {
        assert_eq!(expand_file_name("/ssh:h:/a/../b", None), "/ssh:h:/a/../b");
    }

    // M61 T8b: idempotence — applying expand_file_name to its own output
    // doesn't drift, which is what `expand_path`'s delegation relies on
    // (find-file-internal only normalizes once, but nothing here should
    // break if it were applied twice).
    #[test]
    fn expand_file_name_idempotent() {
        for input in [
            "/a/b/../c",
            "b/./c",
            "/a//b",
            "~/z",
            "/x/y/z",
            "/ssh:h:/a/../b",
        ] {
            let once = expand_file_name(input, None);
            let twice = expand_file_name(&once, None);
            assert_eq!(once, twice, "not idempotent for input {:?}", input);
        }
    }
}
