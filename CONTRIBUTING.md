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

If you change the functions under `crates/baton-ffi/src/api`, write the Dart side of the bridge again and commit the result. CI does the same and fails when the files differ. If you change the app in `apps/baton`, `flutter analyze`, the formatting check and `flutter test` must pass there. [apps/baton/README.md](apps/baton/README.md) has the commands for both, and CI builds the Mac app and the iPhone app on every change.

Code that parses input from the network also has fuzz targets. They are not part of the normal build and need a nightly toolchain and [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz); [crates/baton-proto/fuzz/README.md](crates/baton-proto/fuzz/README.md) says how to run them. If you change a parser, let them run for a few minutes before you open the pull request.

## Making a change

1. Open an issue first for anything bigger than a small fix, so we can agree on the approach.
2. Keep each pull request to one logical change. A reviewer should be able to hold the whole thing in their head. Large features are built as a series of pull requests.
3. Say what changed, why, and how you tested it. The pull request template has the headings.
4. Add tests. Changes to the protocol, pairing or the trust store need tests for the failure cases too: malformed input, wrong keys, expired or replayed tickets.
5. Update the docs in the same pull request: [docs/protocol.md](docs/protocol.md) for any wire change and [docs/security.md](docs/security.md) for any trust change.

Commit messages: a short imperative summary (about 70 characters), a blank line, then a body that explains *why*.

## Code style

- `cargo fmt` and `cargo clippy -D warnings` must be clean.
- No `unsafe` in the core crates (`unsafe_code` is forbidden workspace-wide). If a platform layer needs it, put it in its own crate with the reason written down. `baton-ffi` is that crate for the Flutter bridge: `unsafe` is denied everywhere in it except the module that `flutter_rust_bridge_codegen` writes.
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
