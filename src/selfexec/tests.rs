//! What counts as "the build underneath me changed", and what happens when becoming it fails.

use super::*;

/// Reproduce the installer's replacement exactly: stage a new file, then `rename` it over the path.
/// That is what `cp … && mv -f` does, and it is why identity rather than content is the signal.
fn replace(path: &Path, body: &str) {
    let staged = path.with_extension("staged");
    std::fs::write(&staged, body).unwrap();
    std::fs::rename(&staged, path).unwrap();
}

#[test]
fn a_renamed_binary_is_a_different_file() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("pmd");
    std::fs::write(&exe, "old").unwrap();

    let before = identity(&exe);
    assert!(before.is_some());
    assert!(
        !replaced(&exe, before),
        "nothing has happened yet, so nothing must restart"
    );
    // Writing THROUGH the same inode is not a replacement, and the installer never does it — but
    // this is the case that separates identity from mtime, so it is worth pinning.
    std::fs::write(&exe, "same inode, new bytes").unwrap();
    assert!(!replaced(&exe, before));

    replace(&exe, "new");
    assert!(
        replaced(&exe, before),
        "the installer's `mv` must read as a new build"
    );
    // And the new baseline is stable, which is what stops a restart loop: the image that comes back
    // records what it is now running.
    let after = identity(&exe);
    assert_ne!(after, before);
    assert!(!replaced(&exe, after));
}

#[test]
fn a_path_that_cannot_be_stated_never_restarts_anything() {
    // THE RESTART LOOP THIS PREVENTS. A missing or unreadable executable is "I cannot tell", and
    // treating it as "it changed" would re-exec every tick on a permissions error — against a path
    // that, by definition, cannot be executed either.
    let missing = Path::new("/nonexistent/pmd");
    assert_eq!(identity(missing), None);
    assert!(
        !replaced(missing, Some((1, 2))),
        "a vanished file is not a new build"
    );
    assert!(!replaced(missing, None));

    // No baseline is the same fact from the other side: a process that could not stat itself at
    // startup never restarts, rather than restarting on every tick.
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("pmd");
    std::fs::write(&exe, "here").unwrap();
    assert!(
        !replaced(&exe, None),
        "no baseline means nothing to compare"
    );
}

#[test]
fn the_running_executable_is_a_real_file() {
    // Not an assertion about WHICH file: this test binary is not pmd. It pins that the thing both
    // binaries build their baseline from is stat-able at all, so `replaced` is answering a real
    // question rather than silently comparing two `None`s forever.
    let exe = current_exe();
    assert!(exe.is_absolute(), "{exe:?}");
    assert!(identity(&exe).is_some(), "{exe:?}");
    assert!(!replaced(&exe, identity(&exe)));
}

#[test]
fn an_absolute_launch_path_is_the_file_to_watch() {
    let dir = tempfile::tempdir().unwrap();
    let launched = dir.path().join("pmd");
    let sandbox_copy = dir.path().join("sandbox/pmd");
    std::fs::create_dir_all(sandbox_copy.parent().unwrap()).unwrap();
    std::fs::write(&launched, "installed build").unwrap();
    std::fs::write(&sandbox_copy, "sandbox copy").unwrap();

    assert_eq!(
        executable_to_watch(Some(launched.as_os_str()), sandbox_copy),
        launched
    );
}

#[test]
fn an_unwatchable_launch_path_falls_back_to_the_kernel_exe() {
    let dir = tempfile::tempdir().unwrap();
    let kernel_exe = dir.path().join("sandbox/pmd");
    std::fs::create_dir_all(kernel_exe.parent().unwrap()).unwrap();
    std::fs::write(&kernel_exe, "sandbox copy").unwrap();

    assert_eq!(
        executable_to_watch(Some(Path::new("pmd").as_os_str()), kernel_exe.clone()),
        kernel_exe
    );
}

#[test]
fn a_sandbox_materialized_launch_path_maps_back_to_the_real_path() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("pmd");
    let sandbox = dir
        .path()
        .join(".sboxcache/run/.sandbox_exec")
        .join(real.strip_prefix("/").unwrap());
    std::fs::create_dir_all(sandbox.parent().unwrap()).unwrap();
    std::fs::write(&real, "installed build").unwrap();
    std::fs::write(&sandbox, "sandbox copy").unwrap();

    assert_eq!(
        executable_to_watch(Some(sandbox.as_os_str()), sandbox.clone()),
        real
    );
}

#[test]
fn becoming_a_file_that_cannot_be_executed_reports_rather_than_exits() {
    // `relaunch` returns ONLY on failure, so this is the only side of it a test may touch: a
    // successful exec here would replace the test runner. The caller's contract is that it gets an
    // error back and stays alive to say so.
    let dir = tempfile::tempdir().unwrap();
    let not_a_program = dir.path().join("pmd");
    std::fs::write(&not_a_program, "not an executable").unwrap();

    let failure = relaunch(&not_a_program);
    assert!(
        matches!(
            failure.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::InvalidData
        ),
        "unexpected failure: {failure:?}"
    );
    // An empty path is what `current_exe` yields on a machine that will not say, and it has to fail
    // the same way rather than exec something off `PATH`.
    assert_eq!(relaunch(Path::new("")).kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn a_refresh_is_silent_until_there_is_a_new_build_and_survives_a_failed_one() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("pmd");
    std::fs::write(&exe, "not an executable").unwrap();
    let before = identity(&exe);

    // NOTHING HAPPENED, so nothing is attempted. This runs every tick, so acting here would be a
    // restart attempt per tick.
    refresh_or_log(&exe, before, "pmd");
    refresh_or_log(&exe, None, "pmd");

    // Replaced, and the new file still cannot be executed — the only side of this a test may reach,
    // because a successful exec would replace the test runner. RETURNING AT ALL is the assertion:
    // a daemon that died here would leave every session it drives unattended.
    let staged = dir.path().join("pmd.staged");
    std::fs::write(&staged, "also not an executable").unwrap();
    std::fs::rename(&staged, &exe).unwrap();
    refresh_or_log(&exe, before, "pmd");

    // And it is still the same process, holding the same temp directory, able to keep working.
    assert!(exe.is_file());
}
