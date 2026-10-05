use std::path::{Path, PathBuf};

use super::Excludes;
use super::locate::{config_file_from, ignore_patterns};
use crate::test_repo::temp_dir;
use crate::walker::collect_files_with;

fn init(dir: &Path) {
    git2::Repository::init(dir).expect("init");
}

fn write_config(body: &str) -> PathBuf {
    let file = temp_dir("abcop_cfg").join("abcop");
    std::fs::write(&file, body).unwrap();
    file
}

fn ruby(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, "def a\nend\n").unwrap();
}

fn ruby_repo(tag: &str, files: &[&str]) -> PathBuf {
    let repo = temp_dir(tag);
    init(&repo);
    for file in files {
        ruby(&repo.join(file));
    }
    repo
}

fn loaded(body: &str, repos: &[PathBuf]) -> Excludes {
    Excludes::from_file(Some(write_config(body)), repos)
}

#[test]
fn config_file_prefers_override_then_xdg_then_home() {
    assert_eq!(
        config_file_from(Some("/cfg/abcop"), Some("/xdg"), Some("/home/tester")).unwrap(),
        PathBuf::from("/cfg/abcop")
    );
    assert_eq!(
        config_file_from(None, Some("/xdg"), Some("/home/tester")).unwrap(),
        PathBuf::from("/xdg/abcop")
    );
    assert_eq!(
        config_file_from(None, None, Some("/home/tester")).unwrap(),
        PathBuf::from("/home/tester/.config/abcop")
    );
    assert!(config_file_from(None, None, None).is_none());
}

#[test]
fn ignore_option_is_read_and_other_lines_are_not() {
    let file = write_config("# note\nignore = legacy/\nmax-abc = 12\nignore=lib/old.rb\n");
    assert_eq!(
        ignore_patterns(&file),
        vec!["legacy/".to_string(), "lib/old.rb".to_string()]
    );
}

#[test]
fn ignore_pattern_applies_in_every_project() {
    let repo = ruby_repo("exclude_repo", &["keep.rb", "legacy/old.rb"]);
    let other = ruby_repo("exclude_other", &["legacy/old.rb", "keep.rb"]);
    let excludes = loaded("ignore = legacy/\n", &[repo.clone(), other.clone()]);
    assert!(excludes.skips(&repo.join("legacy/old.rb")));
    assert!(excludes.skips(&repo.join("legacy")));
    assert!(excludes.skips(&other.join("legacy/old.rb")));
    assert!(!excludes.skips(&repo.join("keep.rb")));
    assert!(!excludes.skips(&other.join("keep.rb")));
}

#[test]
fn file_pattern_leaves_siblings() {
    let repo = ruby_repo("exclude_file_repo", &["legacy/old.rb", "legacy/other.rb"]);
    let excludes = loaded("ignore = legacy/old.rb\n", &[repo.clone()]);
    assert!(excludes.skips(&repo.join("legacy/old.rb")));
    assert!(!excludes.skips(&repo.join("legacy/other.rb")));
}

#[test]
fn missing_config_skips_nothing() {
    let repo = ruby_repo("exclude_none_repo", &["legacy/old.rb"]);
    let excludes = Excludes::from_file(
        Some(temp_dir("exclude_none_cfg").join("abcop")),
        &[repo.clone()],
    );
    assert!(!excludes.skips(&repo.join("legacy/old.rb")));
}

#[test]
fn walker_drops_ignored_module_even_when_named() {
    let repo = ruby_repo("exclude_walk_repo", &["keep.rb", "legacy/old.rb"]);
    let excludes = loaded("ignore = legacy/\n", &[repo.clone()]);
    let walked = collect_files_with(&[repo.display().to_string()], true, &excludes);
    assert_eq!(file_names(&walked), vec!["keep.rb".to_string()]);
    let named = repo.join("legacy/old.rb");
    assert!(collect_files_with(&[named.display().to_string()], false, &excludes).is_empty());
}

fn file_names(paths: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
