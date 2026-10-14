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
//! with it. A command that waits for a person at a terminal can start it sooner, so that
//! what is typed before the question is asked is dropped and cannot answer it.
//!
//! Questions are asked one at a time. A second one waits until the first is dropped, and
//! only then begins, so a `yes` typed for one question cannot answer another: a pairing
//! request that arrives while the person is answering `inbox clear` waits its turn, and is
//! put to them after the result of `inbox clear` has been printed.

use std::{
    io::{self, BufRead, IsTerminal},
    sync::{Arc, Mutex, PoisonError},
};

use tokio::sync::{OwnedMutexGuard, mpsc, oneshot};

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
    /// Held by the question that is being asked, so that there is only one at a time.
    turn: Arc<tokio::sync::Mutex<()>>,
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
                turn: Arc::new(tokio::sync::Mutex::new(())),
                interactive,
            }),
        }
    }

    /// Whether a person is typing: standard input is a terminal.
    pub fn is_interactive(&self) -> bool {
        self.inner.interactive
    }

    /// Starts reading now if a person is typing, so that lines typed before anything asks
    /// are dropped. Without this the first question would take whatever was typed while
    /// the command waited, such as an Enter that was meant for nothing, or a `yes` typed
    /// before the pairing request was on the screen.
    ///
    /// Standard input that is not a terminal is left alone: a script may write its answer
    /// first, and it is read when the question comes.
    pub fn ignore_early_typing(&self) {
        if self.inner.interactive {
            self.start();
        }
    }

    /// Asks for the next line typed, once any question already being asked is dropped. A
    /// line that arrives while no question is waiting goes to the prompt instead.
    ///
    /// The question is waiting from the moment this returns, not from the first time it is
    /// read: ask first and print the request after, and an answer cannot arrive too early
    /// to be heard, however fast it is piped in. Ask before printing anything about the
    /// question, since this can wait for another one to finish.
    pub async fn ask(&self) -> Question {
        let turn = Arc::clone(&self.inner.turn).lock_owned().await;
        let (answer, wait) = oneshot::channel();
        {
            let mut state = self.inner.state();
            if state.ended {
                let _ = answer.send(None);
            } else {
                state.question = Some(answer);
            }
        }
        self.start();
        Question {
            wait: Some(wait),
            _turn: turn,
        }
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

/// A question that has been asked. [`answer`](Question::answer) gives the line typed in
/// reply, or `None` when standard input ended first.
///
/// The next question is not asked until this one is dropped, not merely until it is
/// answered. Keep it while acting on the answer and printing what came of it, so that a
/// question that was waiting is put to the person after that, and its details do not land
/// between their `yes` and the result.
#[derive(Debug)]
#[must_use = "a question that is not read takes no answer"]
pub struct Question {
    wait: Option<oneshot::Receiver<Option<String>>>,
    _turn: OwnedMutexGuard<()>,
}

impl Question {
    /// Waits for the answer. Asking again gives `None`.
    pub async fn answer(&mut self) -> Option<String> {
        let wait = self.wait.take()?;
        wait.await.ok().flatten()
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
            async move { console.ask().await.answer().await }
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
        let mut question = console.ask().await;
        type_line(&keys, "yes");
        assert_eq!(within(question.answer()).await.as_deref(), Some("yes"));

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
            async move { console.ask().await.answer().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(keys);
        assert_eq!(within(asked).await.unwrap(), None);
        assert_eq!(within(prompt.recv()).await.unwrap(), None);
        // After the end, a new question is answered at once.
        assert_eq!(
            within(async { console.ask().await.answer().await }).await,
            None
        );
    }

    #[tokio::test]
    async fn a_question_that_was_given_up_does_not_swallow_a_line() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        // Ask, then stop waiting, as happens when a pairing offer ends.
        let gave_up = tokio::time::timeout(Duration::from_millis(100), async {
            console.ask().await.answer().await
        })
        .await;
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
        assert_eq!(
            within(async { console.ask().await.answer().await })
                .await
                .as_deref(),
            Some("early")
        );
    }

    #[tokio::test]
    async fn typing_before_the_question_is_dropped_when_a_person_is_typing() {
        let (keys, typed) = std_mpsc::channel();
        let console = Console::new(Box::new(io::BufReader::new(Typed(typed))), true);
        console.ignore_early_typing();
        // Typed while the command waits: an Enter, and a yes before anything was shown.
        type_line(&keys, "");
        type_line(&keys, "yes");
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut question = console.ask().await;
        type_line(&keys, "no");
        assert_eq!(within(question.answer()).await.as_deref(), Some("no"));
    }

    #[tokio::test]
    async fn input_that_is_not_a_terminal_is_not_read_early() {
        let (console, keys) = console();
        console.ignore_early_typing();
        type_line(&keys, "yes");
        tokio::time::sleep(Duration::from_millis(200)).await;
        // A script wrote its answer first, and the question still gets it.
        assert_eq!(
            within(async { console.ask().await.answer().await })
                .await
                .as_deref(),
            Some("yes")
        );
    }

    #[tokio::test]
    async fn questions_are_asked_one_at_a_time_and_answers_stay_with_their_question() {
        let (console, keys) = console();
        let mut prompt = console.prompt();
        let mut first = console.ask().await;

        // A second question, as from a pairing request that arrives meanwhile, is not
        // asked yet, so it cannot take the line meant for the first.
        let second = tokio::spawn({
            let console = console.clone();
            async move { console.ask().await.answer().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!second.is_finished());

        type_line(&keys, "yes");
        assert_eq!(within(first.answer()).await.as_deref(), Some("yes"));

        // Answered is not finished: the second waits until the first is dropped, which is
        // after what the first command had to print.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!second.is_finished());
        drop(first);

        // Now it is the second one's turn, and its answer is its own.
        tokio::time::sleep(Duration::from_millis(100)).await;
        type_line(&keys, "no");
        assert_eq!(within(second).await.unwrap().as_deref(), Some("no"));

        type_line(&keys, "devices");
        assert_eq!(
            within(prompt.recv()).await.unwrap().as_deref(),
            Some("devices")
        );
    }

    #[tokio::test]
    async fn a_question_that_is_dropped_gives_up_its_turn() {
        let (console, keys) = console();
        let _prompt = console.prompt();
        drop(console.ask().await);
        type_line(&keys, "ignored");
        // Give the line time to be read, and to go to the prompt: nobody is waiting for it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut next = console.ask().await;
        type_line(&keys, "yes");
        assert_eq!(within(next.answer()).await.as_deref(), Some("yes"));
    }
}
