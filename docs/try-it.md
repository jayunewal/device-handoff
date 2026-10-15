# Try Baton

Baton is pre-alpha. This page gets two computers handing links and text to each other, and says what is not built yet, so a failure is not a surprise. It works between macOS, Windows and Linux.

## Get the program

**Download a build.** The repository builds the program for macOS, Windows and Linux for every change that is merged into `main`. Open the [CI runs](https://github.com/jayunewal/device-handoff/actions/workflows/ci.yml), pick the latest run on `main` with a green tick and, under **Artifacts** at the bottom of the page, download `baton-macos`, `baton-windows` or `baton-linux`. GitHub asks you to sign in to download.

- **macOS** (Apple Silicon and Intel): the download is a zip with `baton-macos.tar.gz` inside. Unzip it first (Safari does that by itself; Chrome and Firefox do not). In Terminal, in the folder it is in, run `tar xzf baton-macos.tar.gz`, then `./baton --help`. The program is not signed by a developer, so macOS may block it. The dialog offers "Done" and "Move to Trash": click **Done**, run `xattr -d com.apple.quarantine ./baton` once and try again.
- **Windows**: unzip `baton-windows.zip`, open PowerShell in that folder and run `.\baton.exe --help`. If Windows says it protected your PC, run `Unblock-File .\baton.exe` first. A Windows 11 PC with **Smart App Control** switched on (Windows Security, App & browser control) blocks every program that is not signed, and `Unblock-File` does not change that; if that is yours, try on a Mac or Linux first.
- **Linux** (x86-64; Ubuntu 22.04 or later, or any distribution with glibc 2.35 or newer): the download is a zip with `baton-linux.tar.gz` inside. Unzip it, then run `tar xzf baton-linux.tar.gz` and `./baton --help`.

The examples below say `baton`. Put the program in a folder on your `PATH`, or type `./baton` (`.\baton.exe` on Windows) in the folder it is in.

**Or build it yourself.** You need a recent stable [Rust toolchain](https://rustup.rs), and a C compiler and linker, since some of the libraries compile C code: on macOS the Xcode Command Line Tools (`xcode-select --install`), on Windows the Visual Studio C++ build tools that the Rust installer offers, on Linux a package such as `build-essential`. From a clone of this repository:

```sh
cargo install --path crates/baton-cli --locked
baton --help
```

That puts a program called `baton` in Cargo's `bin` folder. To run it without installing, use `cargo run -p baton-cli --release --` where the examples say `baton`.

## Pair two computers

Pairing happens once per pair of computers. Call them A and B.

1. On A, run `baton listen`. It prints its name and fingerprint and a `baton>` prompt. Leave it running.
2. At A's prompt, type `pair offer`. It prints a command that starts with `baton pair join BATON1:` and a long ticket. Get that line to B in any way you trust, such as a message to yourself.
3. On B, run that command exactly as printed. B prints a six-digit code and waits.
4. A asks whether to pair with B and shows the same six-digit code. **If the two codes are the same**, type `yes` on A. Anything else is a no.

Both computers then print `Paired with ...`. A ticket works once and for five minutes, and it is a secret until it has been used: send it only to your own other computer.

The codes are what protect the pairing. If they differ, or A shows a device you do not know, say no: someone else has the ticket.

## Send a link and some text

For a computer to receive, `baton listen` must be running on it.

On B (which is paired with A, while A is listening):

```sh
baton send link https://example.com --title "An example"
baton send text hello from B
echo "or pipe it in" | baton send text
```

To send what is on the clipboard, copy something first and run `baton send clipboard`. If you copied the lines above to paste them, those lines are what it would send.

A prints each one as it arrives, for example `[#1] link from B: https://example.com/ ("An example")`. Links are shown in their normal form, which is why a trailing slash appears. Nothing is opened or copied on its own: it waits in the inbox.

With more than one paired device, say which: `--to "Name"` for `send`, or the name after `baton ping`. A fingerprint start works as well as a name. `baton ping` checks that the other computer answers and whether the path is direct.

**To go both ways**, run `baton listen` on B as well, and send from the window where a computer is listening: everything you can type after `baton` also works at the `baton>` prompt, without the word `baton`, so on A type `send text hello from A` there. Only one `baton` runs a computer at a time, so while B is listening, `baton send` in another window on B is refused. Type `send ...` at B's prompt instead.

## Read what arrived

On A, in any window, also while `baton listen` is running:

- `baton inbox` shows the list.
- `baton inbox show 2` prints item 2 in full.
- `baton inbox open 1` opens item 1 in your browser (web links only).
- `baton inbox copy 2` puts item 2 on the clipboard.
- `baton inbox rm 1` removes item 1, and `baton inbox clear` removes everything.

`baton devices` lists the paired computers and `baton unpair NAME` forgets one. `baton id` prints this computer's name, fingerprint and where it keeps its files.

## Try it on one computer first

Each copy of Baton keeps its state in a folder, and `--home` picks another, so one computer can be both. Both copies would call themselves by this computer's name, and a computer is shown under the name it had when it paired, so give the second one its own name first. On a Mac or Linux:

```sh
mkdir -p /tmp/baton-b
echo 'device_name = "B"' > /tmp/baton-b/config.toml
```

On Windows, use `C:\baton-a` and `C:\baton-b` where the steps below say `/tmp/baton-a` and `/tmp/baton-b`, and write the name from PowerShell like this (a plain `echo ... >` saves the file in a format Baton cannot read, and it stops with `not valid UTF-8 text`):

```powershell
mkdir C:\baton-b
Set-Content -Path C:\baton-b\config.toml -Value 'device_name = "B"' -Encoding utf8
```

Then, in the first terminal, play A:

```sh
baton --home /tmp/baton-a listen
```

and at its `baton>` prompt:

```
pair offer
```

In a second terminal, play B, with the ticket that A printed:

```sh
baton --home /tmp/baton-b pair join <ticket from A>
```

Both print a six-digit code. If they are the same, type `yes` in the first terminal. Then, in the second:

```sh
baton --home /tmp/baton-b send text hello
baton --home /tmp/baton-b send link https://example.com
```

and the first terminal prints each item as it arrives. That is a full pairing and send on one machine, with nothing else to set up. Stop with Ctrl+C.

## While a computer is locked

The reason Baton exists is to let you walk away, so this is the part to try. On A, run `baton listen` and keep the computer awake (plugged in, and with sleep switched off for the test; on a Mac, `caffeinate -i baton listen` does it). Lock A. From B, send a link and some text, and run `baton ping`. Unlock A and run `baton inbox`.

What to expect: the items arrive while A is locked, and nothing is opened or copied. What does not work: a computer that has gone to sleep cannot answer, so B says `Could not reach ...`. Waking sleeping computers and sending from a locked one need the background agent, which is not built. [locked-devices.md](https://github.com/jayunewal/device-handoff/blob/main/docs/locked-devices.md) has what each system allows.

## If something does not work

- **`Could not reach ...`**: `baton listen` has to be running on the other computer, and both need to be online. On a guest or office Wi-Fi that stops devices talking to each other, the public relay is the way through, so the internet must work.
- **macOS** asks whether Terminal may find devices on your local network the first time. Allow it (System Settings, Privacy & Security, Local Network). If it refuses to run a downloaded copy, `xattr -d com.apple.quarantine ./baton` clears the download flag.
- **Windows** asks whether to allow `baton` through the firewall the first time it listens. Allow it on private networks.
- **More detail**: add `-v` (or `-vv`) before the command, as in `baton -v listen`. Without it, only Baton's own warnings are shown, and the networking library's messages are left out.
- **A different name** for this computer: put `device_name = "Kitchen laptop"` in `config.toml` in the folder `baton id` shows. Do it before pairing: a computer is shown under the name it had when it paired, so the other computers keep showing the old one.
- **Only one `baton` can run a device at a time.** A second one says so. `baton inbox` and `baton devices` still work while another window listens.

## What is not built yet

- Files and folders. This is next, with its own fast transfer protocol and a benchmark.
- The background agent: nothing starts by itself, so `baton listen` has to be left running in a window, and there is no tray icon. Nothing opens a link or fills the clipboard automatically.
- Lock detection. The program cannot tell whether the computer is locked, and so it never acts on what arrives, locked or not.
- Phones and tablets, accounts that pair every device at once, and window streaming.
- An outbox. Sending to a computer that is asleep or offline fails; it does not wait.
- Real-network testing. Pairing, sending, the inbox and the command line are tested automatically on Linux, macOS and Windows with two copies on one machine. Finding each other across a real Wi-Fi network (mDNS), through the public relays and through a router are written but have not yet been run on real networks, so these are the likeliest places for a surprise. Reports are welcome.
- Signed programs. The builds are not signed by a developer, which is why macOS and Windows may warn.

Everything sent between computers is end-to-end encrypted, including through a relay. The default relays are the iroh project's public ones, which are rate limited.
