//! A background fetch of every configured registry.
//!
//! The shell starts a job when the Search panel is first opened and polls it once
//! per tick. The network and the disk stay on worker threads; the UI thread only
//! takes finished documents out of the channel, so a slow registry never blocks a
//! frame.
//!
//! Documents are kept in the configured URL order, not completion order, so the
//! rows a user sees do not reshuffle as the slower registries answer.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;

use crate::fetch::{self, Fetched};
use crate::{Index, Registry};

/// What one registry is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// The fetch is still running.
    Loading,
    /// The document is in the index; `cached` says the network was not used.
    Ready {
        /// Whether the document came from the last-good cache.
        cached: bool,
        /// Why the fresh copy was not used, when it was not.
        note: Option<String>,
    },
    /// The registry could not be read and had no cache.
    Failed(String),
}

#[derive(Debug)]
struct Event {
    url: String,
    result: Result<Fetched, String>,
}

/// One fetch of every configured registry.
#[derive(Debug)]
pub struct Job {
    urls: Vec<String>,
    states: Vec<State>,
    documents: Vec<Option<Registry>>,
    index: Index,
    receiver: Option<Receiver<Event>>,
}

impl Job {
    /// Starts one worker per URL, writing successful documents under `cache`.
    pub fn start(urls: Vec<String>, cache: PathBuf) -> Self {
        let (sender, receiver) = channel();
        let mut documents = Vec::new();
        documents.resize_with(urls.len(), || None);

        for url in &urls {
            let url = url.clone();
            let cache = cache.clone();
            let sender = sender.clone();
            thread::spawn(move || {
                let result = fetch::load(&url, &cache, &fetch::Network).map_err(|error| error.to_string());
                let _ = sender.send(Event { url, result });
            });
        }
        drop(sender);

        Self {
            states: vec![State::Loading; urls.len()],
            urls,
            documents,
            index: Index::new(),
            receiver: Some(receiver),
        }
    }

    /// The URLs this job is fetching, in the order they were configured.
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// One state per URL, in the same order.
    pub fn states(&self) -> &[State] {
        &self.states
    }

    /// The registries that have arrived, in configured order.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Whether any registry is still loading.
    pub fn is_loading(&self) -> bool {
        self.states.iter().any(|state| matches!(state, State::Loading))
    }

    /// Takes every finished fetch, returning whether anything changed.
    ///
    /// Call it from the tick; the worker threads never touch the index.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = self.receiver.take() else {
            return false;
        };

        let mut changed = false;
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(event) => {
                    if let Some(index) = self.urls.iter().position(|url| url == &event.url) {
                        match event.result {
                            Ok(fetched) => {
                                self.documents[index] = Some(fetched.document);
                                self.states[index] = State::Ready {
                                    cached: fetched.cached,
                                    note: fetched.note,
                                };
                            }
                            Err(message) => self.states[index] = State::Failed(message),
                        }
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if !disconnected {
            self.receiver = Some(receiver);
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    /// Rebuilds the index in URL order after a document arrives.
    fn rebuild(&mut self) {
        self.index = Index::new();
        for (url, document) in self.urls.iter().zip(&self.documents) {
            if let Some(document) = document {
                self.index.add(url.clone(), document.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn document(name: &str, id: &str) -> Vec<u8> {
        format!(
            "schema = 1\nname = \"{name}\"\nupdated = \"2026-11-28T12:00:00Z\"\n\n[[extensions]]\nid = \"{id}\"\nname = \"{id}\"\n[[extensions.versions]]\nversion = \"1.0.0\"\napi = 1\nsource = {{ type = \"archive\", url = \"https://example.com/{id}.zip\", sha256 = \"{SHA}\" }}\n"
        )
        .into_bytes()
    }

    fn file_url(path: &Path) -> String {
        let text = path.display().to_string().replace('\\', "/");
        if text.starts_with('/') {
            format!("file://{text}")
        } else {
            format!("file:///{text}")
        }
    }

    fn write(directory: &Path, name: &str, text: Vec<u8>) -> String {
        let path = directory.join(name);
        std::fs::write(&path, text).expect("write");
        file_url(&path)
    }

    fn wait(job: &mut Job) {
        for _ in 0..200 {
            job.poll();
            if !job.is_loading() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the fetch job did not finish");
    }

    #[test]
    fn documents_arrive_in_configured_order() {
        let directory = tempfile::tempdir().expect("dir");
        let cache = tempfile::tempdir().expect("cache");
        let urls = vec![
            write(directory.path(), "one.toml", document("One", "a.one")),
            write(directory.path(), "two.toml", document("Two", "b.two")),
        ];

        let mut job = Job::start(urls.clone(), cache.path().to_path_buf());
        wait(&mut job);

        assert_eq!(job.urls(), urls.as_slice());
        assert_eq!(job.states().len(), 2);
        assert!(
            job.states()
                .iter()
                .all(|state| matches!(state, State::Ready { cached: false, .. }))
        );
        let names: Vec<&str> = job
            .index()
            .registries()
            .iter()
            .map(|loaded| loaded.document.name.as_str())
            .collect();
        assert_eq!(names, ["One", "Two"]);
        assert_eq!(job.index().rows().count(), 2);
    }

    #[test]
    fn a_missing_file_is_a_failed_state() {
        let cache = tempfile::tempdir().expect("cache");
        let mut job = Job::start(
            vec!["file:///no/such/registry.toml".to_string()],
            cache.path().to_path_buf(),
        );
        wait(&mut job);

        assert!(matches!(job.states()[0], State::Failed(_)));
        assert!(job.index().is_empty());
    }

    #[test]
    fn a_cached_document_survives_a_lost_file() {
        let directory = tempfile::tempdir().expect("dir");
        let cache = tempfile::tempdir().expect("cache");
        let url = write(directory.path(), "one.toml", document("One", "a.one"));

        // Prime the cache, then remove the source so the worker must fall back.
        fetch::load(&url, cache.path(), &fetch::Network).expect("prime");
        std::fs::remove_file(directory.path().join("one.toml")).expect("remove");

        let mut job = Job::start(vec![url], cache.path().to_path_buf());
        wait(&mut job);

        assert!(matches!(job.states()[0], State::Ready { cached: true, .. }));
        assert_eq!(job.index().rows().count(), 1);
    }

    #[test]
    fn an_empty_job_is_not_loading() {
        let cache = tempfile::tempdir().expect("cache");
        let mut job = Job::start(Vec::new(), cache.path().to_path_buf());

        assert!(!job.is_loading());
        assert!(!job.poll());
        assert!(job.index().is_empty());
    }
}
