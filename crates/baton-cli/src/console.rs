// SPDX-License-Identifier: Apache-2.0

//! Reading what the person types: answers to questions, and commands at a prompt.
//!
//! One thread reads standard input a line at a time. A line goes to the question that
//! is waiting for an answer, if there is one, and otherwise to the prompt, if one is
//! running. That lets a pairing request ask "do you approve?" at a moment of its own
//! while the prompt is waiting for the next command, without the two taking each
//! other's lines.
//!
//! The thread starts only when something first asks for input, so a command that reads
//! all of standard input itself (`baton send text` with a pipe) is not in competition
//! with it.

use std::{
    future::Future,
    io::{self, BufRead, IsTerminal},
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::Poll,
};

use tokio::sync::{mpsc, oneshot};

type Reader = Box<dyn BufRead + Send>;

/// Lines from standard input, handed to whoever is waiting for one.
#[derive(Clone)]
pub struct Console {
    inner: Arc<Inner>,
}

struct Inner {
    /// Taken by the reader thread when it starts.
    reader: Mutex<Option<Reader>>,
    state: Mutex<State>,
    interactive: bool,
}

#[derive(Default)]
struct State {
    question: Option<oneshot::Sender<Option<String>>>,
    prompt: Option<mpsc::UnboundedSender<Option<String>>>,
    ended: bool,
}

impl Console {
    /// Reads from the process's standard input.
    pub fn stdin() -> Self {
        let interactive = io::stdin().is_terminal();
        Self::new(Box::new(io::BufReader::new(io::stdin())), interactive)
    }

    fn new(reader: Reader, interactive: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                reader: Mutex::new(Some(reader)),
                state: Mutex::new(State::default()),
                interactive,
            }),
        }
    }

    /// Whether a person is typing: standard input is a terminal.
    pub fn is_interactive(&self) -> bool {
        self.inner.interactive
    }

    /// Asks for the next line typed. A line that arrives while no question is waiting
    /// goes to the prompt instead.
    ///
    /// The question is waiting from this call on, not from the first time it is polled:
    /// ask first and print the request after, and an answer cannot arrive too early to be
    /// heard, however fast it is piped in.
    pub fn ask(&self) -> Question {
        let (answer, wait) = oneshot::channel();
        {
            let mut state = self.inner.state();
            if state.ended {
                let _ = answer.send(None);
            } else {
                // An earlier question nobody is waiting for any more is replaced.
                state.question = Some(answer);
            }
        }
        self.start();
        Question(wait)
    }

    /// Starts delivering lines that no question wants, and `None` at the end of input.
    pub fn prompt(&self) -> mpsc::UnboundedReceiver<Option<String>> {
        let (lines, received) = mpsc::unbounded_channel();
        {
            let mut state = self.inner.state();
            if state.ended {
                let _ = lines.send(None);
            }
            state.prompt = Some(lines);
        }
        self.start();
        received
    }

    /// Starts the thread that reads, once.
    fn start(&self) {
        let Some(reader) = self
            .inner
            .reader
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("input".to_owned())
            .spawn(move || {
                for line in reader.lines() {
                    match line {
                        Ok(line) => inner.deliver(Some(line)),
                        Err(error) => {
                            tracing::debug!(%error, "standard input could not be read");
                            break;
                        }
                    }
                }
                inner.deliver(None);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start reading standard input");
            self.inner.deliver(None);
        }
    }
}

/// A question that has been asked. Awaiting it gives the line typed in answer, or `None`
/// when standard input ended first or the question was replaced by a newer one.
#[derive(Debug)]
#[must_use = "a question that is not awaited takes no answer"]
pub struct Question(oneshot::Receiver<Option<String>>);

impl Future for Question {
    type Output = Option<String>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0)
            .poll(cx)
            .map(|line| line.ok().flatten())
    }
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hands one line, or the end of input, to whoever should have it.
    fn deliver(&self, line: Option<String>) {
        let mut state = self.state();
        let ended = line.is_none();
        if ended {
            state.ended = true;
        }
        if let Some(question) = state.question.take() {
            // A question whose asker has gone away does not take the line.
            if !question.is_closed() {
                let _ = question.send(line.clone());
                if !ended {
                    return;
                }
            }
        }
        if let Some(prompt) = &state.prompt {
            let _ = prompt.send(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, sync::mpsc as std_mpsc, time::Duration};

    use super::*;

    /// Standard input that a test types into.
    struct Typed(std_mpsc::Receiver<Vec<u8>>);

    impl Read for Typed {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.recv() {
                Ok(bytes) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    Ok(n)
                }
                Err(_) => Ok(0),
            }
        }
    }

    fn console() -> (Console, std_mpsc::Sender<Vec<u8>>) {
        let (keys, typed) = std_mpsc::channel();
        let console = Console::new(Box::new(io::BufReader::new(Typed(typed))), false);
        (console, keys)
    }

    fn type_line(keys: &std_mpsc::Sender<Vec<u8>>, line: &str) {
        keys.send(format!("{line}\n").into_bytes()).unwrap();
    }

    async fn within<T>(future: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), future)
            .await
            .expect("timed out")
    }

    #[tokio::test]
    async fn a_waiting_question_takes_the_next_line_and_the_prompt_does_not() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        let asked = tokio::spawn({
            let console = console.clone();
            async move { console.ask().await }
        });
        // Give the question time to register before typing.
        tokio::time::sleep(Duration::from_millis(100)).await;
        type_line(&keys, "yes");
        assert_eq!(within(asked).await.unwrap().as_deref(), Some("yes"));

        type_line(&keys, "send link x");
        assert_eq!(
            within(prompt.recv()).await.unwrap().as_deref(),
            Some("send link x")
        );
    }

    #[tokio::test]
    async fn a_question_is_waiting_from_the_moment_it_is_asked() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        // Not polled yet: the answer is typed before anything awaits the question, as
        // happens when input is piped in faster than the request is printed.
        let question = console.ask();
        type_line(&keys, "yes");
        assert_eq!(within(question).await.as_deref(), Some("yes"));

        type_line(&keys, "devices");
        assert_eq!(
            within(prompt.recv()).await.unwrap().as_deref(),
            Some("devices")
        );
    }

    #[tokio::test]
    async fn lines_nobody_asked_for_go_to_the_prompt() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        type_line(&keys, "one");
        type_line(&keys, "two");
        assert_eq!(within(prompt.recv()).await.unwrap().as_deref(), Some("one"));
        assert_eq!(within(prompt.recv()).await.unwrap().as_deref(), Some("two"));
    }

    #[tokio::test]
    async fn the_end_of_input_reaches_both_the_question_and_the_prompt() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        let asked = tokio::spawn({
            let console = console.clone();
            async move { console.ask().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(keys);
        assert_eq!(within(asked).await.unwrap(), None);
        assert_eq!(within(prompt.recv()).await.unwrap(), None);
        // After the end, a new question is answered at once.
        assert_eq!(within(console.ask()).await, None);
    }

    #[tokio::test]
    async fn a_question_that_was_given_up_does_not_swallow_a_line() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        // Ask, then stop waiting, as happens when a pairing offer ends.
        let gave_up = tokio::time::timeout(Duration::from_millis(100), console.ask()).await;
        assert!(gave_up.is_err());

        type_line(&keys, "devices");
        assert_eq!(
            within(prompt.recv()).await.unwrap().as_deref(),
            Some("devices")
        );
    }

    #[tokio::test]
    async fn nothing_is_read_until_something_asks() {
        let (console, keys) = console();
        type_line(&keys, "early");
        tokio::time::sleep(Duration::from_millis(100)).await;
        // The line is still waiting to be read, so the first question gets it.
        assert_eq!(within(console.ask()).await.as_deref(), Some("early"));
    }
}
