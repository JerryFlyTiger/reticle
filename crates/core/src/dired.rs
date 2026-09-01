//! M19: the shared directory-listing core — used by dired (the buffer
//! major mode), the M21 selector panel's file rows, and (M22) remote
//! directories. Pure data + formatting; no editor types.

/// One directory entry with `ls -al`-style metadata.
pub struct FileInfo {
    /// Plain name, no trailing slash.
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub link_target: Option<String>,
    pub is_exec: bool,
    /// "drwxr-xr-x" style mode string.
    pub perms: String,
    pub nlink: u64,
    /// Numeric uid/gid as strings (v1: no name lookup).
    pub user: String,
    pub group: String,
    pub size: u64,
    /// "Jul 22 14:03" (this year) or "Jul 22  2025" (older).
    pub mtime: String,
}

/// Column widths for aligning a set of rows.
pub struct ColWidths {
    pub nlink: usize,
    pub user: usize,
    pub group: usize,
    pub size: usize,
}

impl ColWidths {
    pub fn of(files: &[FileInfo]) -> ColWidths {
        let mut w = ColWidths {
            nlink: 1,
            user: 1,
            group: 1,
            size: 1,
        };
        for f in files {
            w.nlink = w.nlink.max(f.nlink.to_string().len());
            w.user = w.user.max(f.user.len());
            w.group = w.group.max(f.group.len());
            w.size = w.size.max(f.size.to_string().len());
        }
        w
    }
}

/// List `path` like `ls -al`: `.` and `..` first, then alphabetical.
/// `/ssh:` paths dispatch to the remote transport (M22) — dired.el
/// works on remote directories without changes.
pub fn list_dir(path: &str) -> std::io::Result<Vec<FileInfo>> {
    if let Some(rp) = crate::remote::parse(path) {
        return crate::remote::list_dir(&rp).map_err(std::io::Error::other);
    }
    let mut out = Vec::new();
    for special in [".", ".."] {
        let p = std::path::Path::new(path).join(special);
        if let Ok(meta) = std::fs::metadata(&p) {
            out.push(info_from(special.to_string(), &p, &meta, false, None));
        }
    }
    let mut named = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let p = entry.path();
        // symlink_metadata so links are reported as links, not their targets.
        let Ok(lmeta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        let is_symlink = lmeta.file_type().is_symlink();
        let target = if is_symlink {
            std::fs::read_link(&p)
                .ok()
                .map(|t| t.to_string_lossy().to_string())
        } else {
            None
        };
        // For a symlink, perms/size describe the link itself (ls -al does
        // the same); is_dir follows the target so RET can descend.
        let follows_dir = std::fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
        let mut fi = info_from(name, &p, &lmeta, is_symlink, target);
        if is_symlink {
            fi.is_dir = follows_dir;
        }
        named.push(fi);
    }
    named.sort_by(|a, b| a.name.cmp(&b.name));
    out.extend(named);
    Ok(out)
}

fn info_from(
    name: String,
    _path: &std::path::Path,
    meta: &std::fs::Metadata,
    is_symlink: bool,
    link_target: Option<String>,
) -> FileInfo {
    #[cfg(unix)]
    let (perms, nlink, user, group, is_exec) = {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        (
            mode_string(mode, meta.is_dir(), is_symlink),
            meta.nlink(),
            meta.uid().to_string(),
            meta.gid().to_string(),
            !meta.is_dir() && mode & 0o111 != 0,
        )
    };
    #[cfg(not(unix))]
    let (perms, nlink, user, group, is_exec) = (
        "----------".to_string(),
        1u64,
        "-".to_string(),
        "-".to_string(),
        false,
    );
    FileInfo {
        name,
        is_dir: meta.is_dir(),
        is_symlink,
        link_target,
        is_exec,
        perms,
        nlink,
        user,
        group,
        size: meta.len(),
        mtime: mtime_string(meta),
    }
}

#[cfg(unix)]
fn mode_string(mode: u32, is_dir: bool, is_symlink: bool) -> String {
    let kind = if is_symlink {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    };
    let mut s = String::with_capacity(10);
    s.push(kind);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        s.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        s.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    s
}

fn mtime_string(meta: &std::fs::Metadata) -> String {
    let Ok(mtime) = meta.modified() else {
        return "??? ?? ??:??".to_string();
    };
    let Ok(secs) = mtime.duration_since(std::time::UNIX_EPOCH) else {
        return "??? ?? ??:??".to_string();
    };
    format_epoch(secs.as_secs() as i64)
}

/// Civil date/time from a unix timestamp (UTC — v1 keeps it dependency-
/// free; the hour may differ from local `ls` output, documented).
fn format_epoch(secs: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let now_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 / 86_400)
        .unwrap_or(0);
    let (now_y, _, _) = civil_from_days(now_days);
    if y == now_y {
        format!(
            "{} {:2} {:02}:{:02}",
            MONTHS[m as usize - 1],
            d,
            tod / 3600,
            (tod % 3600) / 60
        )
    } else {
        format!("{} {:2}  {}", MONTHS[m as usize - 1], d, y)
    }
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The face name for an entry's file name.
pub fn face_for(fi: &FileInfo) -> &'static str {
    if fi.is_symlink {
        "dired-symlink"
    } else if fi.is_dir {
        "dired-directory"
    } else if fi.is_exec {
        "dired-executable"
    } else {
        "default"
    }
}

/// One formatted row as (text, face-name) segments: metadata columns in
/// dim faces, the name colored by kind, symlink target appended.
pub fn format_row(fi: &FileInfo, w: &ColWidths) -> Vec<(String, &'static str)> {
    let mut segs = Vec::new();
    segs.push((
        format!(
            "{} {:>nl$} {:>u$} {:>g$} ",
            fi.perms,
            fi.nlink,
            fi.user,
            fi.group,
            nl = w.nlink,
            u = w.user,
            g = w.group
        ),
        "dired-perms",
    ));
    segs.push((format!("{:>s$} ", fi.size, s = w.size), "dired-size"));
    segs.push((format!("{} ", fi.mtime), "dired-date"));
    segs.push((fi.name.clone(), face_for(fi)));
    if let Some(t) = &fi.link_target {
        segs.push((format!(" -> {}", t), "dired-symlink"));
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // 2024-01-01
    }

    #[cfg(unix)]
    #[test]
    fn mode_strings() {
        assert_eq!(mode_string(0o755, true, false), "drwxr-xr-x");
        assert_eq!(mode_string(0o644, false, false), "-rw-r--r--");
        assert_eq!(mode_string(0o777, false, true), "lrwxrwxrwx");
    }
}
