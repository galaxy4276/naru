//! Translates every shelved book in the background, a few pages at a time, from the page each book
//! is open at. Progress is the translations folder itself, so a restart picks up where it was.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::{Book, STORE, TRANSLATING, load, texts, translate, translated_pages};
use crate::core::Daemon;

const WORKERS: usize = 2;
/// Tries per page before the queue leaves it to the reader's 다시 시도.
const TRIES: u32 = 3;
/// How long a worker with nothing to do waits before looking again unwoken.
const IDLE: Duration = Duration::from_secs(300);

static WAKE: Condvar = Condvar::new();
static WAKE_LOCK: Mutex<()> = Mutex::new(());
/// Failed tries by book and page.
static FAILED: Mutex<BTreeMap<(String, u32), u32>> = Mutex::new(BTreeMap::new());

struct Job {
    book: Book,
    page: u32,
    text: String,
    before: Option<String>,
}

pub fn start(daemon: &Arc<Daemon>) {
    for _ in 0..WORKERS {
        let daemon = daemon.clone();
        std::thread::spawn(move || work(&daemon));
    }
}

/// New text to translate has arrived.
pub fn wake() {
    WAKE.notify_all();
}

/// The reader asked for the page itself, so the queue may try it again.
pub fn forget_failure(id: &str, page: u32) {
    FAILED
        .lock()
        .expect("library failures lock")
        .remove(&(id.to_owned(), page));
}

fn work(daemon: &Arc<Daemon>) {
    let root = daemon.library_root();
    let mut failures = 0u32;
    loop {
        let job = crate::titles::claude_path(&daemon.home).and_then(|_| next(&root));
        let Some(job) = job else {
            let guard = WAKE_LOCK.lock().expect("library wake lock");
            let _ = WAKE.wait_timeout(guard, IDLE);
            continue;
        };
        let key = (job.book.id.clone(), job.page);
        let result = translate(
            &daemon.home,
            &root,
            &job.book,
            job.page,
            &job.text,
            job.before.as_deref(),
        );
        TRANSLATING.lock().expect("translating lock").remove(&key);
        let failed = result.is_err();
        if let Err(error) = &result {
            eprintln!(
                "서재 번역 실패 {} {}쪽: {error:#}",
                job.book.title, job.page
            );
            *FAILED
                .lock()
                .expect("library failures lock")
                .entry(key)
                .or_default() += 1;
        }
        daemon.library_translated(&job.book.id, job.page, result);
        if failed {
            // A logged-out or rate-limited claude fails every page alike; back off rather than
            // spend the whole book's tries at once.
            failures += 1;
            std::thread::sleep(Duration::from_secs(30 << failures.min(6)));
        } else {
            failures = 0;
        }
    }
}

/// The next page to translate, claimed so the other worker skips it: the book read last first,
/// from the page it is open at to its end, then from its start.
fn next(root: &Path) -> Option<Job> {
    let mut busy = TRANSLATING.lock().expect("translating lock");
    let failed = FAILED.lock().expect("library failures lock");
    let mut books = {
        let _guard = STORE.lock().expect("library lock poisoned");
        load(root).ok()?.books
    };
    books.sort_by_key(|book| (Reverse(book.opened.unwrap_or(0)), book.added));
    for book in books {
        let Some(texts) = texts(root, &book.id).ok().flatten() else {
            continue;
        };
        let done = translated_pages(root, &book.id);
        let order = (book.page..=book.pages).chain(1..book.page);
        let found = order.into_iter().find(|&page| {
            let key = (book.id.clone(), page);
            texts
                .get(page as usize - 1)
                .is_some_and(|text| !text.trim().is_empty())
                && !done.contains(&page)
                && !busy.contains(&key)
                && failed.get(&key).is_none_or(|tries| *tries < TRIES)
        });
        let Some(page) = found else {
            continue;
        };
        busy.insert((book.id.clone(), page));
        let before = page
            .checked_sub(2)
            .and_then(|at| texts.get(at as usize).cloned());
        return Some(Job {
            text: texts[page as usize - 1].clone(),
            book,
            page,
            before,
        });
    }
    None
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
