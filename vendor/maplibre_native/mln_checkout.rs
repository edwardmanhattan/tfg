//! Shared-state guards for the maplibre-native source checkout.
//!
//! Two things about that checkout are process-wide rather than per-build,
//! and both had to be fixed locally because upstream gets them wrong:
//! this module is the lock, and `expected_sha` the revision check that
//! upstream could never satisfy.
//!
//! The build script used to decide a checkout was stale by looking for
//! `CMakeLists.txt` and `include/`, then `fs::remove_dir_all` it and clone
//! again. That is fine for one cargo process and wrong for two, which is
//! the normal case on this machine: an editor's `cargo check` and a
//! terminal's `cargo run` both build `maplibre_native`.
//!
//! The observed failure was exactly that. One process was mid-clone — so
//! `CMakeLists.txt` did not exist yet — and the other concluded the tree was
//! corrupt and deleted the directory the first one was writing to. Both
//! restarted. With 39 submodules to fetch, that is a twenty-five minute
//! loop rather than a twenty-five minute build.
//!
//! Two rules fix it, and both are enforced here rather than in `build.rs`
//! so they can be tested:
//!
//! 1. **Never delete without the lock.** Removal only ever happens while
//!    this process holds the checkout lock, so it can no longer delete a
//!    tree somebody else is populating.
//! 2. **Wait, do not race.** If another process holds the lock, block until
//!    it finishes and re-check. When it has, the checkout is warm and the
//!    work is skipped entirely — so a concurrent `cargo check` costs a
//!    second build script, not a second clone.
//!
//! `usable` and `work` are supplied by the caller so this module stays free
//! of build-script dependencies and can be exercised directly by tests.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Sits next to the checkout, not inside it, so it survives the deletion
/// that `work` may perform.
pub const LOCK_FILE: &str = "maplibre-native.checkout.lock";

/// How often to re-check while another process holds the lock.
const POLL: Duration = Duration::from_millis(250);

/// Backstop for a lock whose holder cannot be probed. Long enough that a
/// live build is never stolen from — the clone of 39 submodules is slow on a
/// bad connection — and finite so a crashed build cannot wedge the tree.
const STEAL_AFTER: Duration = Duration::from_secs(90 * 60);

/// Held for as long as this process may mutate the checkout. Released on
/// drop, including on panic, so a failing build does not wedge the tree.
pub struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Take the lock, or report that somebody else holds it.
///
/// `O_CREAT | O_EXCL` (and `CREATE_NEW` on Windows) is atomic across
/// processes, which is the whole basis of this: there is no window in which
/// two processes both believe they won.
pub fn try_acquire(parent: &Path) -> Option<Lock> {
    let path = parent.join(LOCK_FILE);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .ok()?;
    // The pid lets the next process tell "live build" from "crashed build".
    let _ = writeln!(file, "{}", std::process::id());
    drop(file);
    Some(Lock { path })
}

/// Whether the pid recorded in a lock file is still running.
///
/// Unix-only, and deliberately so: probing a process needs `kill(pid, 0)`,
/// and reaching for that on Windows means pulling in a crate or writing
/// `unsafe` FFI into a build script. Where we cannot tell, the caller falls
/// back to the timeout, which is correct if slow.
#[cfg(unix)]
fn holder_is_alive(path: &Path) -> Option<bool> {
    let pid: i32 = fs::read_to_string(path).ok()?.trim().parse().ok()?;
    // Signal 0 performs the permission and existence checks without
    // delivering anything. EPERM means the process exists but is not ours.
    const EPERM: i32 = 1;
    let rc = unsafe { kill(pid, 0) };
    Some(rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(EPERM))
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(not(unix))]
fn holder_is_alive(_path: &Path) -> Option<bool> {
    None
}

/// Make `dir` usable, at most once across every concurrent build.
///
/// `usable` reports whether `dir` is already good enough. It is called more
/// than once on purpose: before taking the lock, and again while holding
/// it, because the process we were waiting for may have finished in
/// between. `work` performs whatever is needed and is only ever called by
/// the lock holder -- it cannot return a value, because the paths that skip
/// it have none to give.
pub fn ensure<E>(
    parent: &Path,
    dir: &Path,
    usable: impl Fn() -> bool,
    work: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    let mut started = Instant::now();

    loop {
        // Warm already, or the process we waited for finished.
        if usable() {
            return Ok(());
        }

        match try_acquire(parent) {
            Some(_lock) => {
                // Re-check under the lock. Without this, two processes that
                // both arrived cold both clone: the second does not learn
                // from taking the lock that the first has just succeeded.
                if !usable() {
                    if dir.exists() {
                        let _ = fs::remove_dir_all(dir);
                    }
                    return work();
                }
                return Ok(());
            }
            None => {
                // Somebody else is building. Wait rather than racing them.
                let lock = parent.join(LOCK_FILE);
                if holder_is_alive(&lock) == Some(false) {
                    // Crashed build: reclaim immediately instead of waiting
                    // out the timeout.
                    let _ = fs::remove_file(&lock);
                    continue;
                }
                if started.elapsed() >= STEAL_AFTER {
                    // Cannot tell whether the holder is alive, and it has
                    // run implausibly long. Reclaim and risk a redundant
                    // build, which is strictly better than hanging.
                    let _ = fs::remove_file(&lock);
                    started = Instant::now();
                    continue;
                }
                std::thread::sleep(POLL);
            }
        }
    }
}

/// The commit SHA a pinned `MLN_COMMIT` names, when its form reveals one.
///
/// `MLN_COMMIT` is a tag name of the shape `core-<40-hex>`, and `clone_repository`
/// fetches it with `--depth 1`, which stores the tag object but creates no local
/// ref for it. So `git rev-parse <tag>^{commit}` cannot resolve inside a managed
/// checkout, and a revision check written that way fails for every managed
/// checkout -- which meant every build deleted and re-cloned the whole tree. The
/// SHA is in the tag's own name, so read it from there instead.
///
/// Returns `None` for any other shape, leaving the caller to resolve the name
/// through git, which is correct for a caller-managed clone that has real refs.
pub fn expected_sha(commit_name: &str) -> Option<&str> {
    const SHA_LEN: usize = 40;
    let name = commit_name.trim();
    if name.len() <= SHA_LEN {
        return None;
    }
    let (prefix, sha) = name.split_at(name.len() - SHA_LEN);
    // Only a whole suffix: refuse to read 40 hex characters out of the middle
    // of a longer hex run, which would silently compare the wrong commit.
    if prefix
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    sha.chars().all(|c| c.is_ascii_hexdigit()).then_some(sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    // `Send + Sync` because one test moves a result out of a spawned thread.
    type Res = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mln-lock-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lock_is_exclusive() -> Res {
        let dir = tmpdir("exclusive");
        let first = try_acquire(&dir).expect("first acquire");
        assert!(
            try_acquire(&dir).is_none(),
            "a held lock must not be re-taken"
        );
        drop(first);
        assert!(try_acquire(&dir).is_some(), "dropping must release");
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn a_tag_name_yields_its_sha() {
        assert_eq!(
            expected_sha("core-a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5b"),
            Some("a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5b")
        );
        assert_eq!(
            expected_sha("  core-a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5b  "),
            Some("a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5b"),
            "surrounding whitespace is common in Cargo metadata"
        );
        assert_eq!(
            expected_sha("v1.2.3"),
            None,
            "a plain tag has no sha to read"
        );
        assert_eq!(
            expected_sha("a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5b"),
            None,
            "a bare sha is compared directly by the caller, not re-read"
        );
    }

    #[test]
    fn a_longer_hex_run_is_not_mistaken_for_a_sha() {
        // 41 hex characters: reading the last 40 would compare a commit that
        // was never pinned.
        let forty_one = "a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5bcafe";
        assert_eq!(expected_sha(forty_one), None);
        assert_eq!(
            expected_sha("core-not-a-sha-but-long-enough-to-look-like-one-xxxx"),
            None
        );
    }

    #[test]
    fn a_warm_checkout_does_no_work() -> Res {
        let dir = tmpdir("warm");
        let worked = AtomicUsize::new(0);
        ensure(
            &dir,
            &dir.join("maplibre-native"),
            || true,
            || -> Res {
                worked.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )?;
        assert_eq!(
            worked.load(Ordering::SeqCst),
            0,
            "a warm checkout must not be rebuilt"
        );
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn a_cold_checkout_is_built_exactly_once() -> Res {
        let dir = tmpdir("cold");
        let worked = AtomicUsize::new(0);
        let usable = AtomicBool::new(false);
        ensure(
            &dir,
            &dir.join("maplibre-native"),
            || usable.load(Ordering::SeqCst),
            || -> Res {
                worked.fetch_add(1, Ordering::SeqCst);
                usable.store(true, Ordering::SeqCst);
                Ok(())
            },
        )?;
        assert_eq!(worked.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    /// The regression this module exists for.
    ///
    /// One process holds the lock and is part-way through populating the
    /// checkout -- the state that used to read as "corrupt" and got deleted.
    /// The second process must wait, must not delete, and must not re-clone.
    #[test]
    fn a_concurrent_build_is_waited_for_not_raced() -> Res {
        let dir = tmpdir("race");
        let checkout = dir.join("maplibre-native");
        let usable = Arc::new(AtomicBool::new(false));
        let deleted = Arc::new(AtomicBool::new(false));
        let re_cloned = Arc::new(AtomicUsize::new(0));

        // Holder: takes the lock, "clones" for a while, then completes.
        let holder_dir = dir.clone();
        let holder_usable = usable.clone();
        let holder = std::thread::spawn(move || -> Res {
            let _lock = try_acquire(&holder_dir).expect("holder takes the lock");
            std::thread::sleep(Duration::from_millis(300));
            fs::create_dir_all(holder_dir.join("maplibre-native"))?;
            holder_usable.store(true, Ordering::SeqCst);
            Ok(())
        });

        // Waiter: the checkout is unusable on arrival, exactly like a cold
        // start racing a warm one.
        std::thread::sleep(Duration::from_millis(60));
        ensure(
            &dir,
            &checkout,
            || usable.load(Ordering::SeqCst),
            || -> Res {
                re_cloned.fetch_add(1, Ordering::SeqCst);
                deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
        )?;

        holder.join().unwrap()?;
        assert_eq!(
            re_cloned.load(Ordering::SeqCst),
            0,
            "waiter must not re-clone a checkout another process is finishing"
        );
        assert!(
            !deleted.load(Ordering::SeqCst),
            "waiter must never delete a checkout it does not hold the lock for"
        );
        assert!(checkout.is_dir());
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    /// A crashed build must not wedge the tree: the next run reclaims the
    /// lock rather than waiting out the timeout.
    #[test]
    fn a_dead_holders_lock_is_reclaimed() -> Res {
        let dir = tmpdir("dead");
        // A pid above any real one, so `kill(pid, 0)` reports ESRCH.
        fs::write(dir.join(LOCK_FILE), "2147483646\n")?;
        assert_eq!(holder_is_alive(&dir.join(LOCK_FILE)), Some(false));

        let worked = AtomicUsize::new(0);
        let usable = AtomicBool::new(false);
        ensure(
            &dir,
            &dir.join("maplibre-native"),
            || usable.load(Ordering::SeqCst),
            || -> Res {
                worked.fetch_add(1, Ordering::SeqCst);
                usable.store(true, Ordering::SeqCst);
                Ok(())
            },
        )?;
        assert_eq!(
            worked.load(Ordering::SeqCst),
            1,
            "a dead holder's lock must be stolen"
        );
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn a_live_holders_lock_is_not_reclaimed() -> Res {
        let dir = tmpdir("live");
        let path = dir.join(LOCK_FILE);
        fs::write(&path, format!("{}\n", std::process::id()))?;
        assert_eq!(holder_is_alive(&path), Some(true), "we are alive");
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }
}
