# Try Baton

Baton is pre-alpha. This page gets two computers handing links and text to each other, and says what is not built yet, so a failure is not a surprise. It works between macOS, Windows and Linux.

## Get the program

You need a recent stable [Rust toolchain](https://rustup.rs). From a clone of this repository:

```sh
cargo install --path crates/baton-cli --locked
baton --help
```

That puts a program called `baton` in Cargo's `bin` folder. To run it without installing, use `cargo run -p baton-cli --release --` where the examples below say `baton`.

## Pair two computers

Pairing happens once per pair of computers. Call them A and B.

1. On A, run `baton listen`. It prints its name and fingerprint and a `baton>` prompt. Leave it running.
2. At A's prompt, type `pair offer`. It prints a command that starts with `baton pair join BATON1:` and a long ticket.
3. On B, run that command exactly as printed. B prints a six-digit code and waits.
4. A asks whether to pair with B and shows the same six-digit code. **If the two codes are the same**, type `yes` on A. Anything else is a no.

Both computers then print `Paired with ...`. A ticket works once and for five minutes, and it is a secret until it has been used: send it only to your own other computer.

The code is what protects you. If it differs, or A shows a device you do not know, say no: someone else has the ticket.

## Send a link and some text

For a computer to receive, `baton listen` must be running on it. To go both ways, run it on both.

On B (which is paired with A):

```sh
baton send link https://example.com --title "An example"
baton send text hello from B
echo "or pipe it in" | baton send text
baton send clipboard
```

A prints each one as it arrives, for example `[#1] link from B: https://example.com ("An example")`. Nothing is opened or copied on its own: it waits in the inbox.

With more than one paired device, say which: `--to "Name"` for `send`, or the name after `baton ping`. A fingerprint start works as well as a name. `baton ping` checks that the other computer answers and whether the path is direct.

Everything you can type after `baton` also works at the `baton>` prompt, without the word `baton`, so A can send back from the window where it is listening.

## Read what arrived

On A, in any window, also while `baton listen` is running:

```sh
baton inbox             # the list
baton inbox show 2      # one item in full
baton inbox open 1      # a link, in your browser (web links only)
baton inbox copy 2      # onto the clipboard
baton inbox rm 1        # remove one; `baton inbox clear` removes all
```

`baton devices` lists the paired computers and `baton unpair NAME` forgets one. `baton id` prints this computer's name, fingerprint and where it keeps its files.

## Try it on one computer first

Each copy of Baton keeps its state in a folder, and `--home` picks another. In two terminals:

```sh
baton --home /tmp/baton-a listen
baton --home /tmp/baton-b pair join <ticket from A>
```

That is a full pairing and send on one machine, with nothing else to set up.

## While a computer is locked

The reason Baton exists is to let you walk away, so this is the part to try. On A, run `baton listen` and keep the computer awake (plugged in, and with sleep switched off for the test; on a Mac, `caffeinate -i baton listen` does it). Lock A. From B, send a link and some text, and run `baton ping`. Unlock A and run `baton inbox`.

What to expect: the items arrive while A is locked, and nothing is opened or copied. What does not work: a computer that has gone to sleep cannot answer, so B says `Could not reach ...`. Waking sleeping computers and sending from a locked one need the background agent, which is not built. [locked-devices.md](locked-devices.md) has what each system allows.

## If something does not work

- **`Could not reach ...`**: `baton listen` has to be running on the other computer, and both need to be online. On a guest or office Wi-Fi that stops devices talking to each other, the public relay is the way through, so the internet must work.
- **macOS** asks whether Terminal may find devices on your local network the first time. Allow it (System Settings, Privacy & Security, Local Network). If it refuses to run a downloaded copy, `xattr -d com.apple.quarantine ./baton` clears the download flag.
- **Windows** asks whether to allow `baton` through the firewall the first time it listens. Allow it on private networks.
- **More detail**: add `-v` (or `-vv`) before the command, as in `baton -v listen`.
- **A different name** for this computer: put `device_name = "Kitchen laptop"` in `config.toml` in the folder `baton id` shows.
- **Only one `baton` can run a device at a time.** A second one says so. `baton inbox` and `baton devices` still work while another window listens.

## What is not built yet

- Files and folders. This is next, with its own fast transfer protocol and a benchmark.
- The background agent: nothing starts by itself, so `baton listen` has to be left running in a window, and there is no tray icon. Nothing opens a link or fills the clipboard automatically.
- Lock detection. The program cannot tell whether the computer is locked, and so it never acts on what arrives, locked or not.
- Phones and tablets, accounts that pair every device at once, and window streaming.
- An outbox. Sending to a computer that is asleep or offline fails; it does not wait.
- Real-network testing. Pairing, sending, the inbox and the command line are tested automatically on Linux, macOS and Windows with two copies on one machine. Finding each other across a real Wi-Fi network (mDNS), through the public relays and through a router are written but have not yet been run on real networks, so these are the likeliest places for a surprise. Reports are welcome.
- Signed programs. The programs this repository builds are not signed, which is why macOS and Windows may warn.

Everything sent between computers is end-to-end encrypted, including through a relay. The default relays are the iroh project's public ones, which are rate limited.
