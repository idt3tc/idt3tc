//! String based path helpers and file access. Paths are kept as given on
//! the command line (both '/' and '\' count as separators) so that messages
//! and output placeholders show exactly what the user typed.

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

use super::util::{Error, Result};

/// Largest input file that is read; bigger files are rejected before any
/// allocation (see AGENTS.md: inputs may be hostile).
pub const MAX_FILE_SIZE: u64 = 0x7fff_ffff;

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Byte index of the last path separator.
fn last_sep(path: &str) -> Option<usize> {
    path.rfind(is_sep)
}

pub fn exists(path: &str) -> bool {
    std::fs::metadata(path).is_ok()
}

/// True for directories, also when reached through a symlink.
pub fn is_directory(path: &str) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

pub fn create_directories(path: &str) -> Result<()> {
    if path.is_empty() || is_directory(path) {
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .map_err(|e| Error::runtime(format!("cannot create directory '{path}': {}", os_message(&e))))
}

/// Lists the regular files below `dir` (optionally recursing), sorted by
/// path. Symlinks are followed like the targets they point to, but every
/// directory is visited at most once, so symlink loops cannot make the scan
/// run away.
pub fn list_files(dir: &str, recursive: bool) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    list_into(dir, recursive, &mut out, &mut visited)?;
    out.sort();
    Ok(out)
}

fn list_into(dir: &str, recursive: bool, out: &mut Vec<String>, visited: &mut HashSet<PathBuf>) -> Result<()> {
    // Canonical paths identify directories reached through different links.
    if let Ok(canon) = std::fs::canonicalize(dir)
        && !visited.insert(canon)
    {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|e| Error::runtime(format!("cannot list directory '{dir}': {}", os_message(&e))))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::runtime(format!("cannot list directory '{dir}': {}", os_message(&e))))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let full = join(dir, &name);
        if is_directory(&full) {
            if recursive {
                list_into(&full, recursive, out, visited)?;
            }
        } else if std::fs::metadata(&full).map(|m| m.is_file()).unwrap_or(false) {
            // Only regular files: opening a FIFO found in a scanned tree
            // would block forever, devices could be read endlessly.
            out.push(full);
        }
    }
    Ok(())
}

/// Parent directory: "a/b/c" gives "a/b", "x" gives "", "/x" gives "/".
pub fn dirname(path: &str) -> String {
    match last_sep(path) {
        None => String::new(),
        Some(0) => path[..1].to_string(),
        Some(p) => path[..p].to_string(),
    }
}

pub fn basename(path: &str) -> String {
    match last_sep(path) {
        None => path.to_string(),
        Some(p) => path[p + 1..].to_string(),
    }
}

/// File name without the extension; a leading dot does not start one.
pub fn stem(path: &str) -> String {
    let base = basename(path);
    match base.rfind('.') {
        None | Some(0) => base,
        Some(dot) => base[..dot].to_string(),
    }
}

/// Lowercased extension without the dot, or "".
pub fn extension(path: &str) -> String {
    let base = basename(path);
    match base.rfind('.') {
        None | Some(0) => String::new(),
        Some(dot) => base[dot + 1..].to_ascii_lowercase(),
    }
}

/// The path with the extension removed, keeping the original separator.
pub fn without_extension(path: &str) -> String {
    match last_sep(path) {
        None => stem(path),
        Some(p) => format!("{}{}", &path[..=p], stem(path)),
    }
}

pub fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else if a.ends_with(is_sep) {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// `path` relative to the directory `root` when it lies below it, else the
/// file name.
pub fn relative_to(path: &str, root: &str) -> String {
    let mut r = root;
    while r.len() > 1 && r.ends_with(is_sep) {
        r = &r[..r.len() - 1];
    }
    if !r.is_empty() && path.len() > r.len() && path.starts_with(r) && path[r.len()..].starts_with(is_sep) {
        return path[r.len() + 1..].to_string();
    }
    basename(path)
}

/// Reads a whole file, refusing files larger than MAX_FILE_SIZE.
pub fn read_file(path: &str) -> Result<Vec<u8>> {
    // Refuse FIFOs, devices and directories up front; opening a FIFO
    // blocks until a writer appears.
    if let Ok(m) = std::fs::metadata(path)
        && !m.is_file()
    {
        return Err(Error::runtime(format!("'{path}': not a regular file")));
    }
    let mut f = std::fs::File::open(path)
        .map_err(|e| Error::runtime(format!("cannot open '{path}': {}", os_message(&e))))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > MAX_FILE_SIZE {
        return Err(Error::runtime(format!("'{path}': file too large")));
    }
    let mut data = Vec::with_capacity(len as usize);
    // The limit also holds for files that grow while being read and for
    // special files that report no size.
    (&mut f)
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut data)
        .map_err(|_| Error::runtime(format!("error reading '{path}'")))?;
    if data.len() as u64 > MAX_FILE_SIZE {
        return Err(Error::runtime(format!("'{path}': file too large")));
    }
    Ok(data)
}

/// Writes a whole file atomically: the data goes to a temporary file next
/// to `path`, which is then renamed over it. Concurrent writers of the same
/// path (parallel runs for different inputs) therefore never interleave,
/// and a failed write leaves no partial file behind.
pub fn write_file(path: &str, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let create_err = |e: &std::io::Error| Error::runtime(format!("cannot create '{path}': {}", os_message(e)));
    let dir = dirname(path);
    let tmp_name = format!(
        ".{}.idt3tc-{}-{}.tmp",
        basename(path),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let tmp = if dir.is_empty() { tmp_name } else { join(&dir, &tmp_name) };
    // create_new: never follow or reuse something already at the temporary
    // path.
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(|e| create_err(&e))?;
    let ok = f.write_all(data).and_then(|_| f.flush()).is_ok();
    drop(f);
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::runtime(format!("error writing '{path}'")));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(create_err(&e));
    }
    Ok(())
}

/// The OS error text without Rust's " (os error N)" suffix, matching
/// strerror() output.
fn os_message(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(p) => s[..p].to_string(),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers() {
        assert_eq!(dirname("a/b/c"), "a/b");
        assert_eq!(dirname("c"), "");
        assert_eq!(dirname("/x"), "/");
        assert_eq!(stem("dir/.hidden"), ".hidden");
        assert_eq!(extension("dir/x.TGA"), "tga");
        assert_eq!(extension("dir/.hidden"), "");
        assert_eq!(without_extension("a\\b.png"), "a\\b");
        assert_eq!(join("a/", "b"), "a/b");
        assert_eq!(join("a", "b"), "a/b");
        assert_eq!(relative_to("in/sub/x.png", "in/"), "sub/x.png");
        assert_eq!(relative_to("other/x.png", "in"), "x.png");
    }
}
