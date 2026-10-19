# Contributing to Baton

Thanks for helping. Baton is pre-alpha, so the most useful contributions right now are small, well-tested changes to the Rust core and honest bug reports from real devices.

## Setting up

You need a recent stable Rust toolchain (the minimum supported version is `rust-version` in `Cargo.toml`) and [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny). CI uses the newest stable release, and a new release can add Clippy lints that fail `-D warnings`, so run `rustup update stable` before you push to see what CI will see.

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo deny check
```

Tests run on the loopback interface and do not need internet access.

If you change the functions under `crates/baton-ffi/src/api`, write the Dart side of the bridge again and commit the result. CI does the same and fails when the files differ. If you change the app in `apps/baton`, `flutter analyze`, the formatting check and `flutter test` must pass there. [apps/baton/README.md](apps/baton/README.md) has the commands for both, and CI builds the Mac app and the iPhone app for the changes that can affect them (see [What CI runs](#what-ci-runs)).

Code that parses input from the network also has fuzz targets. They are not part of the normal build and need a nightly toolchain and [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz); [crates/baton-proto/fuzz/README.md](crates/baton-proto/fuzz/README.md) says how to run them. If you change a parser, let them run for a few minutes before you open the pull request.

## Making a change

1. Open an issue first for anything bigger than a small fix, so we can agree on the approach.
2. Keep each pull request to one logical change. A reviewer should be able to hold the whole thing in their head. Large features are built as a series of pull requests.
3. Say what changed, why, and how you tested it. The pull request template has the headings.
4. Add tests. Changes to the protocol, pairing or the trust store need tests for the failure cases too: malformed input, wrong keys, expired or replayed tickets.
5. Update the docs in the same pull request: [docs/protocol.md](docs/protocol.md) for any wire change and [docs/security.md](docs/security.md) for any trust change.

Commit messages: a short imperative summary (about 70 characters), a blank line, then a body that explains *why*.

## What CI runs

A pull request runs only the jobs that its change can affect, so a change to the Mac app does not also build the Rust workspace on three systems. The **Changes** job decides from the paths that the pull request touches, and the jobs it leaves out show as skipped.

| The change is only in | The jobs that run |
| --- | --- |
| Markdown files, `docs/`, the issue templates, `.github/CODEOWNERS`, `.github/dependabot.yml`, the Python script in `apps/baton/tool/` | None |
| `apps/baton/test/` | App (Flutter, Linux) |
| `apps/baton/ios/` | App (Flutter, Linux) and App (iPhone) |
| `apps/baton/macos/` | App (Flutter, Linux) and App (macOS) |
| Anything else in `apps/` | The three App jobs |
| Anything else | All of them |

The Rust jobs are Format, Clippy, Test, Minimum supported Rust version, Rustdoc, and Licenses and advisories. A change that is in several rows runs the jobs of all of them. A change to `crates/`, `Cargo.lock`, `.github/workflows` or the scripts next to it, and anything the rules do not know, such as a new folder, runs all of them. So does an empty list of paths, or a pull request whose paths cannot be listed. A push to `main` and a run started by hand always run everything, and they run Build too, which makes the downloads. A pull request does not.

The rules are in `.github/scripts/changed-paths.sh`. `changed-paths-test.sh` next to it holds them to the changes people really make, and CI runs it first, so a rule that skips a job it should not fails there.

### Why a run takes the time it does

GitHub runs five Mac jobs at a time for the whole repository, and a run on `main` needs four of them (App (macOS), App (iPhone), Test on macOS and Build for macOS). A pull request needs three, because it does not build the downloads. When several pull requests run together the Mac jobs wait their turn, and nothing is wrong. What CI does to ask less of them:

- The jobs that build Rust keep their dependencies from one run to the next, and the two app jobs also what Cargo builds for the library, in `apps/baton/build/cargokit` (`apps/baton/rust_builder/README.md` says how Cargokit is changed to build there). Only a run on `main` saves these caches, because GitHub keeps 10 GB for the whole repository and a set for every pull request pushed the others out. A pull request that changes `Cargo.lock` therefore builds the new dependencies in each of its runs, and the run on `main` after the merge saves them. The last step of each app job says how many of the dependencies it had to build, in a notice on the page of the run, so a cache that stopped helping shows.
- A push to a pull request cancels the run of the one before it. A run on `main` is not cancelled by the next merge, so that it gets to save its cache.
- Rust is built without debug information (`CARGO_PROFILE_DEV_DEBUG` in `ci.yml`, and a file that the app jobs write for the builds that Xcode starts). It makes a build a third faster and a cache a third of the size, and a failed test still names its file and line.
- The iPhone job builds its simulator for the processor of the Mac only. If Flutter never finds the app in the simulator, which has happened in about one job of eight, `.github/scripts/hang-guard.py` stops the test after two minutes and starts it again once. The log says so in a warning, and prints what the simulator logged about the app, so that a run that passes on the second start still shows what went wrong in the first. A test that fails is not started again.
- The downloads (the `baton` program for macOS, Windows and Linux, and the Mac app) are made by a run on `main` and by a run started by hand, not for a pull request. To get the downloads of a branch before it is merged, open **Actions**, choose **CI**, click **Run workflow** and pick the branch. A break that shows only in a release build, in the program for Intel Macs or when the downloads are packed is found by the run on `main` after the merge.

## Code style

- `cargo fmt` and `cargo clippy -D warnings` must be clean.
- No `unsafe` in the core crates (`unsafe_code` is forbidden workspace-wide). If a platform layer needs it, put it in its own crate with the reason written down. `baton-ffi` is that crate for the Flutter bridge: `unsafe` is denied everywhere in it except the module that `flutter_rust_bridge_codegen` writes. `baton-stream-abi` is that crate for the C interface to the platform code of the app: `unsafe` is denied everywhere in it except the two modules that cross the interface (`platform`, where Rust calls the platform, and `exports`, where the platform calls Rust), each `unsafe` block there says why it is sound, and a test fails if its list of lints drifts from the workspace's. `baton-fs` is that crate for the few system calls that the standard library lacks (moving a file without replacing one, marking a received file, hiding a folder, asking for free space, and making an open file wait for its data): `unsafe` is denied everywhere in it except its `sys` module, which holds one short function for each call and platform, and each `unsafe` block there says why it is sound.
- Input from the network is untrusted. Parsing code must not panic: return an error. Avoid `unwrap()` outside tests, do not index or slice with `[]` (`baton-proto` turns on `clippy::indexing_slicing`; use `get`, `split_first_chunk` and the like), and bound every length and count before allocating.
- Errors in library crates use `thiserror`; the command-line program turns them into messages written for the person who ran the command, with the likeliest cause and what to do about it (`CliError`).
- Start new source files with `// SPDX-License-Identifier: Apache-2.0`.

## Licensing

Baton is licensed under the [Apache License 2.0](LICENSE). By submitting a contribution you agree it is licensed under the same terms (section 5 of the license).

Do not copy or translate code from projects whose licenses are incompatible with Apache-2.0, such as the GPL and AGPL. That includes RustDesk, KDE Connect and Sunshine. Reading their designs and documentation is fine and encouraged. If you are unsure whether something is safe to reuse, ask in the pull request.

New dependencies must have a permissive license. `cargo deny check` enforces this in CI.

## Security

Please do not report vulnerabilities in public issues. See [SECURITY.md](SECURITY.md).

## Conduct

Be kind and assume good faith. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
