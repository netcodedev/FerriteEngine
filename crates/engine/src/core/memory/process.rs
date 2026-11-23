//! The process's memory as the system sees it: resident memory by kind of
//! mapping (`/proc/self/smaps`), Linux only.

/// Resident bytes and mappings, by kind, then the totals. Empty where the
/// system does not tell.
pub fn mappings() -> Vec<(&'static str, u64, u64)> {
    let Ok(smaps) = std::fs::read_to_string("/proc/self/smaps") else {
        return Vec::new();
    };
    group(&smaps)
}

/// The kind of a mapping, by its path column.
fn kind(path: &str) -> &'static str {
    if path.is_empty() {
        "anonymous (malloc arenas and big blocks, driver)"
    } else if path == "[heap]" {
        "malloc heap (brk)"
    } else if path.starts_with("[stack") {
        "stacks"
    } else if path.starts_with("/dev/nvidia") || path.starts_with("/dev/dri") || path.starts_with("/memfd:") || path.starts_with("/dmabuf") {
        "graphics driver mappings"
    } else if path.starts_with('[') {
        "other (vdso, vvar, anon names)"
    } else if path.contains(".so") {
        "shared libraries"
    } else {
        "other files"
    }
}

fn group(smaps: &str) -> Vec<(&'static str, u64, u64)> {
    let mut groups: Vec<(&'static str, u64, u64)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in smaps.lines() {
        let mut fields = line.split_whitespace();
        let Some(first) = fields.next() else { continue };
        if first.contains('-') && first.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            // "start-end perms offset dev inode [path]": the path may hold spaces.
            let path = line.splitn(6, char::is_whitespace).nth(5).map(str::trim).unwrap_or("");
            let k = kind(path);
            let index = groups.iter().position(|g| g.0 == k).unwrap_or_else(|| {
                groups.push((k, 0, 0));
                groups.len() - 1
            });
            groups[index].2 += 1;
            current = Some(index);
        } else if first == "Rss:" {
            if let (Some(i), Some(kib)) = (current, fields.next().and_then(|v| v.parse::<u64>().ok())) {
                groups[i].1 += kib * 1024;
            }
        }
    }
    groups
}

/// Resident set and its parts, swap (`/proc/self/status`).
pub fn totals() -> Vec<(&'static str, u64)> {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return Vec::new();
    };
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kib| kib * 1024)
    };
    [
        ("resident (RSS)", "VmRSS:"),
        ("resident anonymous", "RssAnon:"),
        ("resident files", "RssFile:"),
        ("resident shared", "RssShmem:"),
        ("swapped out", "VmSwap:"),
        ("peak resident", "VmHWM:"),
    ]
    .into_iter()
    .filter_map(|(label, name)| Some((label, field(name)?)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_mappings_by_kind() {
        let smaps = "\
55d0-55e0 r--p 00000000 103:02 123 /usr/bin/ferrite editor
Rss:                 8 kB
55e0-55f0 rw-p 00000000 00:00 0                          [heap]
Rss:               100 kB
7f00-7f10 rw-p 00000000 00:00 0
Rss:               200 kB
7f10-7f20 rw-s 00000000 00:06 5 /dev/nvidiactl
Rss:                50 kB
7f20-7f30 r-xp 00000000 103:02 9 /usr/lib/libc.so.6
Rss:                30 kB
";
        let groups = group(smaps);
        let get = |k: &str| groups.iter().find(|g| g.0 == k).map(|g| g.1);
        assert_eq!(get("malloc heap (brk)"), Some(100 * 1024));
        assert_eq!(get("anonymous (malloc arenas and big blocks, driver)"), Some(200 * 1024));
        assert_eq!(get("graphics driver mappings"), Some(50 * 1024));
        assert_eq!(get("shared libraries"), Some(30 * 1024));
        assert_eq!(get("other files"), Some(8 * 1024));
    }
}
