//! What the sidebar's file manager does to files. The functions that touch a disk
//! work on this machine. For another machine, the daemon there does the work, and
//! this holds what both need: where a path is, and how to read the answers.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Folders a search does not walk into, since they are large and rarely wanted.
const SEARCH_SKIPS: [&str; 5] = [".git", "node_modules", "target", ".cache", ".Trash"];
/// How many paths a search lists at most, so a home folder cannot stall the window.
const SEARCH_MAX: usize = 5000;

/// A path on a machine. An empty host is this machine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    pub host: String,
    pub path: PathBuf,
}

impl Place {
    pub fn new(host: &str, path: impl Into<PathBuf>) -> Self {
        Self { host: host.to_string(), path: path.into() }
    }

    pub fn is_here(&self) -> bool {
        self.host.is_empty()
    }

    /// How `tuios cp` names it. `local:` keeps a colon in a path from reading as a host.
    pub fn spec(&self) -> String {
        let host = if self.is_here() { "local" } else { &self.host };
        format!("{host}:{}", self.path.display())
    }

    /// `tuios cp` naming this as a folder to copy into.
    pub fn folder_spec(&self) -> String {
        let spec = self.spec();
        if spec.ends_with('/') { spec } else { spec + "/" }
    }

    pub fn name(&self) -> String {
        self.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())
    }
}

/// When a file last changed, from an answer of `file-stat`. A file that is not there
/// has no time.
pub fn stat_stamp(answer: &serde_json::Value) -> Option<i64> {
    answer["info"]["mtime"].as_i64().filter(|_| answer["exists"] == true)
}

/// A file error from tuios, as one line for the status bar. A copy prints several
/// lines, and a refused write spends most of its words on the name of each machine.
pub fn plain_error(raw: &str) -> String {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.ends_with(": copies") && !l.starts_with("That is the decision"))
        .unwrap_or("That did not work.");
    let line = line.split_once(" failed: ").map_or(line, |(_, why)| why);
    if line.contains("does not let a linked machine") {
        return "Refused: tuios keeps keys, credentials and shell settings from other machines.".to_string();
    }
    let allowed = |marker: &str| line.split_once(marker).and_then(|(_, rest)| rest.split(". ").next()).map(|f| f.trim_end_matches('.').to_string());
    if let Some(folders) = allowed("write only to ") {
        return format!("Refused: that machine takes writes only in {folders}. See files_roots in its tuios config.");
    }
    if let Some(folders) = allowed("read only ") {
        return format!("Refused: that machine allows reads only in {folders}. See files_read_roots in its tuios config.");
    }
    line.to_string()
}

/// Files waiting to be pasted.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub from: Place,
    pub cut: bool,
}

fn refuse(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// A name that stays inside its folder. It may have folders in it, but no way out.
pub fn inside(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let name = name.trim();
    let relative = Path::new(name);
    let stays = relative.components().all(|c| matches!(c, Component::Normal(_)));
    if name.is_empty() || !stays {
        return Err(refuse("That name would leave this folder."));
    }
    Ok(dir.join(relative))
}

/// Makes a new file, or a folder if the name ends in `/`, with any folders it names.
pub fn create(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let path = inside(dir, name)?;
    if name.trim().ends_with('/') {
        if path.exists() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "That folder is already here."));
        }
        std::fs::create_dir_all(&path)?;
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::File::create_new(&path)?;
    }
    Ok(path)
}

/// Gives a path a new name in its folder, and never replaces what has that name.
pub fn rename(path: &Path, name: &str) -> io::Result<PathBuf> {
    let dir = path.parent().ok_or_else(|| refuse("This has no folder to rename in."))?;
    let to = inside(dir, name)?;
    if to.symlink_metadata().is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Something already has that name."));
    }
    std::fs::rename(path, &to)?;
    Ok(to)
}

/// A free name in `dir` for `name`: itself, or `a copy.txt`, `a copy 2.txt` and so on.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    dir.join(free_among(name, |candidate| dir.join(candidate).symlink_metadata().is_ok()))
}

/// The first of `a.txt`, `a copy.txt`, `a copy 2.txt` and so on that is not taken.
pub fn free_among(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let as_path = Path::new(name);
    let stem = as_path.file_stem().map_or(name.into(), |s| s.to_string_lossy());
    let dot = as_path.extension().map_or(String::new(), |e| format!(".{}", e.to_string_lossy()));
    (1..)
        .map(|n| if n == 1 { format!("{stem} copy{dot}") } else { format!("{stem} copy {n}{dot}") })
        .find(|candidate| !taken(candidate))
        .expect("some name is free")
}

fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    let meta = from.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        return std::os::unix::fs::symlink(std::fs::read_link(from)?, to);
    }
    if !meta.is_dir() {
        return std::fs::copy(from, to).map(|_| ());
    }
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        copy_all(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

/// Puts what was copied or cut into a folder, and answers with where it landed.
pub fn paste(clip: &Clip, dir: &Path) -> io::Result<PathBuf> {
    let name = clip.from.path.file_name().ok_or_else(|| refuse("This has no name to paste under."))?;
    let from = &clip.from.path;
    if from.is_dir() && dir.starts_with(from) {
        return Err(refuse("A folder cannot go inside itself."));
    }
    if clip.cut && from.parent() == Some(dir) {
        return Ok(from.clone());
    }
    let to = free_name(dir, &name.to_string_lossy());
    if clip.cut {
        // A rename fails across disks, and then it is a copy and a delete.
        if std::fs::rename(from, &to).is_err() {
            copy_all(from, &to)?;
            delete_forever(from)?;
        }
    } else {
        copy_all(from, &to)?;
    }
    Ok(to)
}

/// Deletes for good.
pub fn delete_forever(path: &Path) -> io::Result<()> {
    let meta = path.symlink_metadata()?;
    if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

/// Whether a search leaves out a path from its folder.
fn skipped_by_search(relative: &str) -> bool {
    relative.split('/').any(|part| SEARCH_SKIPS.contains(&part))
}

/// A search's paths from an answer of `file-walk` on another machine, each with
/// whether it is a folder.
pub fn walk_answer(answer: &serde_json::Value) -> Vec<(String, bool)> {
    let entries = answer["entries"].as_array().into_iter().flatten();
    entries
        .filter_map(|e| Some((e["rel"].as_str()?.to_string(), e["dir"].as_bool().unwrap_or(false))))
        .filter(|(relative, _)| !skipped_by_search(relative))
        .take(SEARCH_MAX)
        .collect()
}

/// Every file and folder under `dir`, as paths from it, for a search to filter.
pub fn walk(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&next).into_iter().flatten().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if found.len() >= SEARCH_MAX {
                return found;
            }
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && SEARCH_SKIPS.contains(&entry.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            if let Ok(relative) = path.strip_prefix(dir) {
                found.push(relative.to_string_lossy().into_owned());
            }
            if is_dir {
                pending.push(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_names_its_machine_for_a_copy() {
        assert_eq!(Place::new("m1", "/a b/c.txt").spec(), "m1:/a b/c.txt");
        assert_eq!(Place::new("", "/a:b").spec(), "local:/a:b");
        assert_eq!(Place::new("m1", "/a").folder_spec(), "m1:/a/");
        assert_eq!(Place::new("m1", "/").folder_spec(), "m1:/");
    }

    #[test]
    fn a_refused_copy_says_what_to_do_in_one_line() {
        let raw = "zz.txt: copies\nThe copy of zz.txt to build:/h/proj/zz.txt failed: laptop cannot write ~/proj/zz.txt. far.local lets other machines write only to ~/Downloads/tuios. To allow ~/proj, add it to files_roots in [hosts.laptop] in the config on far.local.\nThat is the decision of the person who set up build. Ask them, or copy to a folder it allows.";
        assert_eq!(plain_error(raw), "Refused: that machine takes writes only in ~/Downloads/tuios. See files_roots in its tuios config.");
        assert_eq!(plain_error("a: copies\nThe copy of a to /b failed: the folder /b does not exist\n"), "the folder /b does not exist");
        assert_eq!(plain_error("plain words"), "plain words");
        let keys = "laptop cannot read ~/.ssh on this machine. tuios does not let a linked machine read keys or credentials.";
        assert_eq!(plain_error(keys), "Refused: tuios keeps keys, credentials and shell settings from other machines.");
    }

    #[test]
    fn a_file_stat_answer_gives_the_change_time() {
        let there = serde_json::json!({"exists": true, "info": {"mtime": 1793882987284_i64}, "size": 28});
        assert_eq!(stat_stamp(&there), Some(1793882987284));
        assert_eq!(stat_stamp(&serde_json::json!({"exists": false, "size": 0})), None);
    }

    #[test]
    fn a_free_name_avoids_what_is_taken() {
        let taken = ["a.txt", "a copy.txt"];
        assert_eq!(free_among("b.txt", |n| taken.contains(&n)), "b.txt");
        assert_eq!(free_among("a.txt", |n| taken.contains(&n)), "a copy 2.txt");
    }

    #[test]
    fn a_far_search_leaves_out_the_same_folders() {
        let answer = serde_json::json!({"entries": [
            {"rel": "src", "dir": true},
            {"rel": "src/main.rs"},
            {"rel": "node_modules/x/y.js"},
            {"rel": "a/.git/config"},
        ]});
        assert_eq!(walk_answer(&answer), vec![("src".to_string(), true), ("src/main.rs".to_string(), false)]);
    }

    /// An empty folder of its own, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("tessella-files-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn create_makes_files_and_folders_and_stays_inside() {
        let s = Scratch::new("create");
        assert!(create(&s.0, "a.txt").unwrap().is_file());
        assert!(create(&s.0, "sub/").unwrap().is_dir());
        assert!(create(&s.0, "deep/er/b.txt").unwrap().is_file());
        assert!(create(&s.0, "a.txt").is_err(), "must not truncate what is there");
        assert!(create(&s.0, "../out.txt").is_err());
        assert!(create(&s.0, "/tmp/out.txt").is_err());
        assert!(create(&s.0, "  ").is_err());
    }

    #[test]
    fn rename_never_replaces() {
        let s = Scratch::new("rename");
        let a = create(&s.0, "a").unwrap();
        create(&s.0, "b").unwrap();
        assert!(rename(&a, "b").is_err());
        assert!(a.exists());
        assert_eq!(rename(&a, "c").unwrap(), s.0.join("c"));
        assert!(rename(&s.0.join("c"), "../d").is_err());
    }

    #[test]
    fn paste_copies_under_a_free_name_and_cut_moves() {
        let s = Scratch::new("paste");
        let a = create(&s.0, "a.txt").unwrap();
        std::fs::write(&a, "hi").unwrap();
        let copy = Clip { from: Place::new("", a.clone()), cut: false };
        assert_eq!(paste(&copy, &s.0).unwrap(), s.0.join("a copy.txt"));
        assert_eq!(paste(&copy, &s.0).unwrap(), s.0.join("a copy 2.txt"));
        assert_eq!(std::fs::read_to_string(s.0.join("a copy.txt")).unwrap(), "hi");
        let dest = create(&s.0, "dest/").unwrap();
        assert_eq!(paste(&Clip { from: Place::new("", a.clone()), cut: true }, &dest).unwrap(), dest.join("a.txt"));
        assert!(!a.exists());
    }

    #[test]
    fn a_folder_copies_whole_and_not_into_itself() {
        let s = Scratch::new("folder");
        let f = create(&s.0, "f/").unwrap();
        create(&f, "in/x").unwrap();
        let dest = create(&s.0, "dest/").unwrap();
        let clip = Clip { from: Place::new("", f.clone()), cut: false };
        assert!(paste(&clip, &dest).unwrap().join("in/x").is_file());
        assert!(f.join("in/x").is_file());
        assert!(paste(&clip, &f.join("in")).is_err());
        assert!(paste(&Clip { from: Place::new("", f.clone()), cut: true }, &f).is_err());
    }

    #[test]
    fn cut_into_its_own_folder_changes_nothing() {
        let s = Scratch::new("same");
        let a = create(&s.0, "a").unwrap();
        assert_eq!(paste(&Clip { from: Place::new("", a.clone()), cut: true }, &s.0).unwrap(), a);
        assert!(a.exists());
    }

    #[test]
    fn delete_forever_does_not_follow_a_link() {
        let s = Scratch::new("link");
        let kept = create(&s.0, "kept/").unwrap();
        create(&kept, "x").unwrap();
        let link = s.0.join("link");
        std::os::unix::fs::symlink(&kept, &link).unwrap();
        delete_forever(&link).unwrap();
        assert!(kept.join("x").exists());
    }

    #[test]
    fn walk_lists_below_and_skips_the_heavy_folders() {
        let s = Scratch::new("walk");
        create(&s.0, "a/b.txt").unwrap();
        create(&s.0, "node_modules/x.js").unwrap();
        let found = walk(&s.0);
        assert!(found.contains(&"a/b.txt".to_string()));
        assert!(!found.iter().any(|p| p.contains("node_modules")));
    }
}
