// SPDX-License-Identifier: Apache-2.0

//! The `baton` program run the way a person runs it: real processes talking over real
//! sockets, two devices on this computer.
//!
//! Each device has a state folder of its own and settings that need nothing beyond this
//! computer's own addresses (no relay, no discovery, no port mapping), so the tests run
//! the same on a laptop and in CI. Devices find each other through the addresses in the
//! pairing ticket, as they would on a network where discovery is blocked.
//!
//! A device that is listening is driven through its prompt, and one-shot commands are run
//! to the end. Every wait has a limit, so a program that hangs fails the test with
//! everything it printed instead of stalling the run.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    fmt,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use tempfile::TempDir;

/// How long to wait for the program to print something or exit. Generous, so a slow
/// machine passes, but not so long that a hang takes the whole run with it.
const WAIT: Duration = Duration::from_secs(60);

/// The characters a terminal would act on if they reached it as they are.
const ESCAPE: char = '\u{1b}';
const BELL: char = '\u{7}';

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Out,
    Err,
}

/// One device: a name, and a state folder of its own.
struct Device {
    name: &'static str,
    home: TempDir,
}

impl Device {
    fn new(name: &'static str) -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            format!(
                "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\n\
                 lan_discovery = false\nport_mapping = false\n"
            ),
        )
        .unwrap();
        Self { name, home }
    }

    /// Starts `baton <args>` and returns at once.
    fn start(&self, args: &[&str]) -> Process {
        let mut command = Command::new(env!("CARGO_BIN_EXE_baton"));
        command
            .arg("--home")
            .arg(self.home.path())
            .args(args)
            .env_remove("BATON_HOME")
            .env_remove("BATON_LOG")
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Process::spawn(
            format!("{} `baton {}`", self.name, args.join(" ")),
            &mut command,
        )
    }

    /// Runs `baton <args>` to the end with nothing typed: a question it asks gets no
    /// answer.
    fn run(&self, args: &[&str]) -> Output {
        self.start(args).finish()
    }

    /// Runs `baton <args>` to the end with `input` piped in.
    fn run_with_input(&self, args: &[&str], input: &str) -> Output {
        let mut process = self.start(args);
        process.write(input);
        process.finish()
    }

    /// Starts `baton listen` and waits until it says it is listening.
    fn listen(&self) -> Process {
        let mut process = self.start(&["listen"]);
        process.wait_for(&format!("Listening as \"{}\"", self.name));
        process
    }
}

/// A running `baton`, with what it has printed and a way to type to it.
struct Process {
    label: String,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<(Stream, String)>,
    log: Vec<(Stream, String)>,
    /// Where the next wait starts: lines before it were already looked at.
    cursor: usize,
}

impl Process {
    fn spawn(label: String, command: &mut Command) -> Self {
        let mut child = command.spawn().unwrap();
        let stdin = child.stdin.take();
        let (sender, lines) = mpsc::channel();
        pump(Stream::Out, child.stdout.take().unwrap(), sender.clone());
        pump(Stream::Err, child.stderr.take().unwrap(), sender);
        Self {
            label,
            child,
            stdin,
            lines,
            log: Vec::new(),
            cursor: 0,
        }
    }

    /// Types `text`. A program that has already ended does not take it, and what it
    /// printed explains why, so a failure to write is not the error worth reporting.
    fn write(&mut self, text: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin
                .write_all(text.as_bytes())
                .and_then(|()| stdin.flush());
        }
    }

    fn type_line(&mut self, line: &str) {
        self.write(&format!("{line}\n"));
    }

    /// The next line it prints that has not been looked at yet.
    fn next_line(&mut self, waiting_for: &str) -> String {
        let deadline = Instant::now() + WAIT;
        while self.cursor >= self.log.len() {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => self.log.push(line),
                Err(RecvTimeoutError::Timeout) => {
                    self.fail(&format!("nothing came while waiting for {waiting_for}"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.fail(&format!("it ended while waiting for {waiting_for}"));
                }
            }
        }
        let line = self.log[self.cursor].1.clone();
        self.cursor += 1;
        line
    }

    /// Skips lines until one contains `needle`, and returns it.
    fn wait_for(&mut self, needle: &str) -> String {
        let waiting_for = format!("{needle:?}");
        loop {
            let line = self.next_line(&waiting_for);
            if line.contains(needle) {
                return line;
            }
        }
    }

    /// The first line that is not blank after the one containing `header`, trimmed: how
    /// the program prints a code on a line of its own.
    fn code_after(&mut self, header: &str) -> String {
        self.wait_for(header);
        loop {
            let line = self.next_line(&format!("what follows {header:?}"));
            if !line.trim().is_empty() {
                return line.trim().to_owned();
            }
        }
    }

    /// Answers the question about a device that wants to pair. The answer is typed only
    /// once the question has been printed, which is when the program is listening for it.
    fn answer_pairing(&mut self, answer: &str) {
        self.wait_for("Type yes to pair");
        self.type_line(answer);
    }

    /// The ticket from the line `baton pair join <ticket>` that `pair offer` prints.
    fn ticket(&mut self) -> String {
        let line = self.wait_for("baton pair join BATON1:");
        line.trim()
            .strip_prefix("baton pair join ")
            .unwrap()
            .to_owned()
    }

    /// Everything it has printed so far, whether or not it was looked at: for checking
    /// what must never have been printed.
    fn transcript(&mut self) -> String {
        while let Ok(line) = self.lines.try_recv() {
            self.log.push(line);
        }
        self.log
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn fail(&mut self, what: &str) -> ! {
        let transcript = self.transcript();
        panic!(
            "{}: {what}\n--- everything it printed ---\n{transcript}\n---",
            self.label
        );
    }

    /// Closes its input, waits for it to end and collects what it printed.
    fn finish(mut self) -> Output {
        drop(self.stdin.take());
        let deadline = Instant::now() + WAIT;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                self.fail("it did not end");
            }
            thread::sleep(Duration::from_millis(20));
        };
        // The readers stop when its output closes; take what they have not delivered yet.
        while let Ok(line) = self.lines.recv_timeout(WAIT) {
            self.log.push(line);
        }
        let text = |wanted: Stream| {
            let mut text = String::new();
            for (_, line) in self.log.iter().filter(|(stream, _)| *stream == wanted) {
                text.push_str(line);
                text.push('\n');
            }
            text
        };
        Output {
            code: status.code().unwrap_or(-1),
            stdout: text(Stream::Out),
            stderr: text(Stream::Err),
        }
    }

    /// Types `quit` at the prompt and waits for it to stop.
    fn quit(mut self) -> Output {
        self.type_line("quit");
        self.finish()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // A test that failed leaves nothing running.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads `pipe` a line at a time on a thread of its own, so that a program that prints a
/// lot never waits for a test that is not reading.
fn pump(stream: Stream, pipe: impl Read + Send + 'static, sender: mpsc::Sender<(Stream, String)>) {
    thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            match pipe.read_until(b'\n', &mut raw) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\n', '\r']).to_owned();
                    if sender.send((stream, line)).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// How a program that ran to the end went.
struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Output {
    /// Checks that it succeeded.
    fn succeeded(self) -> Self {
        assert_eq!(self.code, 0, "{self}");
        self
    }

    /// Checks that it failed with exactly this exit status.
    fn failed_with(self, code: i32) -> Self {
        assert_eq!(self.code, code, "{self}");
        self
    }

    /// Checks that standard error mentions `text` (and standard output does not).
    fn explained(self, text: &str) -> Self {
        assert!(self.stderr.contains(text), "expected {text:?} in\n{self}");
        assert!(
            !self.stdout.contains(text),
            "{text:?} went to standard output:\n{self}"
        );
        self
    }
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "exit status {}\n--- standard output ---\n{}--- standard error ---\n{}---",
            self.code, self.stdout, self.stderr
        )
    }
}

/// Starts both devices listening and pairs them: `a` offers and `b` joins, each at its
/// prompt. Returns the two running programs.
fn paired_and_listening(a: &Device, b: &Device) -> (Process, Process) {
    let mut on_a = a.listen();
    let mut on_b = b.listen();
    on_a.type_line("pair offer");
    let ticket = on_a.ticket();
    on_b.type_line(&format!("pair join {ticket}"));
    on_a.wait_for("A device wants to pair with this one");
    on_a.answer_pairing("yes");
    on_a.wait_for(&format!("Paired with {}", b.name));
    on_b.wait_for(&format!("Paired with {}", a.name));
    (on_a, on_b)
}

#[test]
fn two_devices_pair_then_trade_a_link_and_some_text() {
    let (alpha, bravo) = (Device::new("Alpha"), Device::new("Bravo"));
    let mut a = alpha.listen();
    let mut b = bravo.listen();

    // Alpha offers; Bravo joins with the ticket. Both show the same code, and the
    // fingerprint Alpha shows for the joiner is the one Bravo reports for itself.
    a.type_line("pair offer");
    let ticket = a.ticket();
    b.type_line(&format!("pair join {ticket}"));
    let code_on_bravo = b.code_after("The code for this pairing is:");
    a.wait_for("A device wants to pair with this one");
    let fingerprint_line = a.wait_for("Fingerprint:");
    let code_on_alpha = a.code_after("it is showing this code:");
    assert_eq!(code_on_alpha, code_on_bravo);
    assert_eq!(code_on_alpha.chars().count(), 7, "{code_on_alpha:?}");
    let shown = fingerprint_line
        .split_once("Fingerprint:")
        .unwrap()
        .1
        .trim();
    let own_id = bravo.run(&["id"]).succeeded();
    assert!(own_id.stdout.contains(shown), "{shown}\n{own_id}");

    a.answer_pairing("yes");
    a.wait_for("Paired with Bravo");
    b.wait_for("Paired with Alpha");

    // Each lists the other, from another window, while it keeps listening.
    let listed = alpha.run(&["devices"]).succeeded();
    assert!(listed.stdout.contains("Bravo"), "{listed}");
    let listed = bravo.run(&["devices"]).succeeded();
    assert!(listed.stdout.contains("Alpha"), "{listed}");

    // A link from Alpha to Bravo.
    a.type_line(r#"send link https://example.com/page?x=1 --title "A page""#);
    a.wait_for("Sent the link to Bravo.");
    b.wait_for(r#"[#1] link from Alpha: https://example.com/page?x=1 ("A page")"#);

    // Some text back.
    b.type_line("send text hello from Bravo");
    b.wait_for("Sent the text to Alpha.");
    a.wait_for("[#1] text from Bravo (16 bytes): hello from Bravo");

    a.type_line("ping");
    a.wait_for("Bravo answered in");

    // The inbox is read, and emptied, from another window while Alpha listens.
    let inbox = alpha.run(&["inbox"]).succeeded();
    assert!(inbox.stdout.contains("Bravo"), "{inbox}");
    assert!(inbox.stdout.contains("hello from Bravo"), "{inbox}");
    let item = alpha.run(&["inbox", "show", "1"]).succeeded();
    assert!(item.stdout.contains("hello from Bravo"), "{item}");
    alpha.run(&["inbox", "rm", "1"]).succeeded();
    let inbox = alpha.run(&["inbox"]).succeeded();
    assert!(inbox.stdout.contains("The inbox is empty."), "{inbox}");

    // What arrives next is numbered after what was removed, as the listener keeps its
    // place in the same file.
    b.type_line("send text and again");
    a.wait_for("[#2] text from Bravo");
    let inbox = alpha.run(&["inbox"]).succeeded();
    assert!(inbox.stdout.contains("and again"), "{inbox}");
    assert!(!inbox.stdout.contains("hello from Bravo"), "{inbox}");

    // A device runs in one process at a time; the second says so and changes nothing.
    alpha
        .run(&["ping"])
        .failed_with(1)
        .explained("already running");

    a.quit().succeeded();
    b.quit().succeeded();
}

#[test]
fn one_shot_commands_reach_a_listening_device() {
    let (alpha, bravo) = (Device::new("Alpha"), Device::new("Bravo"));
    let mut a = alpha.listen();

    // Bravo joins with a command of its own, and Alpha says yes at its prompt.
    a.type_line("pair offer");
    let ticket = a.ticket();
    let mut join = bravo.start(&["pair", "join", &ticket]);
    let code_on_bravo = join.code_after("The code for this pairing is:");
    a.wait_for("A device wants to pair with this one");
    assert_eq!(a.code_after("it is showing this code:"), code_on_bravo);
    a.answer_pairing("yes");
    let joined = join.finish().succeeded();
    assert!(joined.stdout.contains("Paired with Alpha"), "{joined}");
    a.wait_for("Paired with Bravo");

    // A link, as one command.
    let sent = bravo
        .run(&[
            "send",
            "link",
            "https://example.com/a?b=1",
            "--title",
            "A title",
        ])
        .succeeded();
    assert!(sent.stdout.contains("Sent the link to Alpha."), "{sent}");
    a.wait_for(r#"[#1] link from Bravo: https://example.com/a?b=1 ("A title")"#);

    // Text piped in, which is how a script or a shell pipeline sends it.
    let sent = bravo
        .run_with_input(&["send", "text"], "first line\nsecond line\n")
        .succeeded();
    assert!(sent.stdout.contains("Sent the text to Alpha."), "{sent}");
    a.wait_for("[#2] text from Bravo (22 bytes): first line…");
    let item = alpha.run(&["inbox", "show", "2"]).succeeded();
    assert!(item.stdout.contains("first line\nsecond line"), "{item}");

    // Whatever Bravo sends, it cannot act on Alpha's terminal: control characters
    // arrive as visible escapes in the announcement, the list and the full view.
    bravo
        .run_with_input(&["send", "text"], "before\u{1b}[2Jafter\u{7}\u{9b}31m\n")
        .succeeded();
    let announced = a.wait_for("[#3] text from Bravo");
    assert!(announced.contains("\\u{1b}[2J"), "{announced:?}");
    let listed = alpha.run(&["inbox"]).succeeded();
    let shown = alpha.run(&["inbox", "show", "3"]).succeeded();
    for output in [&listed.stdout, &shown.stdout, &a.transcript()] {
        assert!(!output.contains(ESCAPE), "{output:?}");
        assert!(!output.contains(BELL), "{output:?}");
        assert!(!output.contains('\u{9b}'), "{output:?}");
    }
    assert!(shown.stdout.contains("\\u{1b}[2J"), "{shown}");

    // Bravo checks that Alpha answers, with a command of its own.
    let ping = bravo.run(&["ping"]).succeeded();
    assert!(ping.stdout.contains("Alpha answered in"), "{ping}");

    // A link can be removed, and the inbox emptied, with a yes typed at the prompt.
    let cleared = alpha.run(&["inbox", "clear", "--yes"]).succeeded();
    assert!(cleared.stdout.contains("Removed 3 items."), "{cleared}");

    a.quit().succeeded();
}

#[test]
fn a_pairing_the_person_declines_pairs_nothing() {
    let (alpha, bravo) = (Device::new("Alpha"), Device::new("Bravo"));
    let mut a = alpha.listen();
    a.type_line("pair offer");
    let ticket = a.ticket();
    let join = bravo.start(&["pair", "join", &ticket]);

    a.wait_for("A device wants to pair with this one");
    a.answer_pairing("no");
    a.wait_for("Declined.");

    join.finish().failed_with(1).explained("declined");
    assert!(
        alpha
            .run(&["devices"])
            .succeeded()
            .stdout
            .contains("No devices are paired yet.")
    );
    assert!(
        bravo
            .run(&["devices"])
            .succeeded()
            .stdout
            .contains("No devices are paired yet.")
    );

    // The ticket is spent: asking again with it does not get a second request.
    let again = bravo.run(&["pair", "join", &ticket]).failed_with(1);
    assert!(!again.stderr.is_empty(), "{again}");
    a.quit().succeeded();
}

#[test]
fn a_path_typed_at_the_prompt_keeps_its_backslashes() {
    let (alpha, bravo) = (Device::new("Alpha"), Device::new("Bravo"));
    let (mut a, mut b) = paired_and_listening(&alpha, &bravo);

    let path = r"C:\Users\jay\notes.txt";
    b.type_line(&format!("send text {path}"));
    b.wait_for("Sent the text to Alpha.");
    a.wait_for(&format!(
        "[#1] text from Bravo ({} bytes): {path}",
        path.len()
    ));

    a.quit().succeeded();
    b.quit().succeeded();
}

#[test]
fn a_pairing_request_waits_for_the_question_that_is_being_answered() {
    let (alpha, bravo, charlie) = (
        Device::new("Alpha"),
        Device::new("Bravo"),
        Device::new("Charlie"),
    );
    let (mut a, mut b) = paired_and_listening(&alpha, &bravo);
    // Something in Alpha's inbox, so that clearing it asks.
    b.type_line("send text keep me");
    a.wait_for("[#1] text from Bravo");

    // Alpha offers a ticket, then asks itself a question: remove everything in the inbox?
    a.type_line("pair offer");
    let ticket = a.ticket();
    a.type_line("inbox clear");
    a.wait_for("Type yes to remove them.");

    // Charlie joins while that question is open. Its request reaches Alpha, which holds it.
    let mut join = charlie.start(&["pair", "join", &ticket]);
    join.wait_for("The code for this pairing is:");
    thread::sleep(Duration::from_secs(1));
    assert!(
        !a.transcript().contains("Charlie"),
        "the request was put to the person while another question was open:\n{}",
        a.transcript()
    );

    // The yes is for the inbox, and only for the inbox.
    a.type_line("yes");
    a.wait_for("Removed 1 item.");
    a.wait_for("A device wants to pair with this one");
    a.wait_for("Charlie");
    assert!(
        !a.transcript().contains("Paired with Charlie"),
        "a yes meant for another question paired a device:\n{}",
        a.transcript()
    );

    // The pairing has its own question, and it was not answered by that yes.
    a.answer_pairing("no");
    a.wait_for("Declined.");
    join.finish().failed_with(1).explained("declined");
    let devices = alpha.run(&["devices"]).succeeded();
    assert!(!devices.stdout.contains("Charlie"), "{devices}");

    a.quit().succeeded();
    b.quit().succeeded();
}

#[test]
fn a_damaged_inbox_stops_the_commands_that_need_it_and_none_of_the_others() {
    let alpha = Device::new("Alpha");
    let file = alpha.home.path().join("inbox.json");
    std::fs::write(&file, "{ this is not an inbox").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // As private as the inbox's own file, so that it is the contents that are refused.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    alpha.run(&["id"]).succeeded();
    alpha.run(&["devices"]).succeeded();

    // These would have nowhere to keep what arrives, and say what to do about the file.
    for args in [&["inbox"][..], &["listen"], &["pair", "offer"]] {
        alpha
            .run(args)
            .failed_with(1)
            .explained("The inbox cannot be read")
            .explained("rename that file");
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "{ this is not an inbox",
        "the damaged file was changed"
    );

    // Renamed, it starts empty, and works.
    std::fs::rename(&file, alpha.home.path().join("inbox.json.old")).unwrap();
    let inbox = alpha.run(&["inbox"]).succeeded();
    assert!(inbox.stdout.contains("The inbox is empty."), "{inbox}");
}

#[test]
fn a_device_that_was_unpaired_there_says_so_here() {
    let (alpha, bravo) = (Device::new("Alpha"), Device::new("Bravo"));
    let (mut a, mut b) = paired_and_listening(&alpha, &bravo);

    // Alpha unpairs Bravo at its prompt, answering the question it asks.
    a.type_line("unpair Bravo");
    a.wait_for("Type yes to unpair.");
    a.type_line("yes");
    a.wait_for("Unpaired Bravo");
    a.type_line("send text anyone there?");
    a.wait_for("No devices are paired yet.");

    // Bravo still lists Alpha, but what it sends is turned away, and it is told why
    // instead of being asked whether Alpha is online.
    b.type_line("send text still paired?");
    let message = b.wait_for("baton:");
    assert!(message.contains("unpaired"), "{message}");
    assert!(!message.contains("online"), "{message}");
    assert!(
        !a.transcript().contains("still paired?"),
        "the text must not arrive"
    );
    assert!(
        alpha
            .run(&["inbox"])
            .succeeded()
            .stdout
            .contains("The inbox is empty.")
    );

    a.quit().succeeded();
    b.quit().succeeded();
}

#[test]
fn problems_are_explained_and_exit_with_a_status() {
    let device = Device::new("Alpha");

    // Nothing is paired yet.
    let output = device.run(&["send", "text", "hello"]);
    output.failed_with(1).explained("No devices are paired yet");
    device
        .run(&["ping"])
        .failed_with(1)
        .explained("No devices are paired yet");
    device
        .run(&["unpair", "nobody"])
        .failed_with(1)
        .explained("No devices are paired yet");
    let devices = device.run(&["devices"]).succeeded();
    assert!(
        devices.stdout.contains("No devices are paired yet."),
        "{devices}"
    );

    // An empty inbox, and an item that is not there.
    let inbox = device.run(&["inbox"]).succeeded();
    assert!(inbox.stdout.contains("The inbox is empty."), "{inbox}");
    device
        .run(&["inbox", "show", "1"])
        .failed_with(1)
        .explained("There is no item #1");
    device
        .run(&["inbox", "rm", "1"])
        .failed_with(1)
        .explained("Nothing was removed");

    // A ticket that is not one.
    device
        .run(&["pair", "join", "BATON1:not-a-ticket"])
        .failed_with(1)
        .explained("That ticket does not work");
    device
        .run(&["pair", "join", "-"])
        .failed_with(1)
        .explained("No ticket was given");

    // Mistakes in how a command is typed exit with 2, and say how to do it.
    let output = device.run(&["frobnicate"]).failed_with(2);
    assert!(output.stderr.contains("Usage"), "{output}");
    device.run(&["send", "link"]).failed_with(2);

    // Asking who this device is makes its identity once; the answer stays the same.
    let first = device.run(&["id"]).succeeded();
    let second = device.run(&["id"]).succeeded();
    assert!(first.stdout.contains("Alpha"), "{first}");
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn a_second_process_cannot_take_a_device_that_is_running_but_can_read_it() {
    let device = Device::new("Alpha");
    let mut listening = device.listen();

    for command in [&["pair", "offer"][..], &["listen"]] {
        device
            .run(command)
            .failed_with(1)
            .explained("already running");
    }
    // Reading needs no lock.
    device.run(&["id"]).succeeded();
    device.run(&["devices"]).succeeded();
    device.run(&["inbox"]).succeeded();

    // Quitting releases the device for the next process.
    listening.type_line("quit");
    listening.wait_for("Stopping...");
    let output = listening.finish().succeeded();
    assert!(output.stdout.contains("Stopping..."), "{output}");
    let again = device.listen();
    again.quit().succeeded();
}

#[test]
fn help_and_version_need_no_state() {
    let device = Device::new("Alpha");
    let version = device.run(&["--version"]).succeeded();
    assert!(version.stdout.starts_with("baton "), "{version}");
    let help = device.run(&["--help"]).succeeded();
    for word in [
        "pair", "send", "inbox", "listen", "devices", "ping", "unpair",
    ] {
        assert!(help.stdout.contains(word), "{word} is not in\n{help}");
    }
    let help = device.run(&["send", "--help"]).succeeded();
    assert!(help.stdout.contains("link"), "{help}");
}
