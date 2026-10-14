# Security policy

Baton moves clipboard contents and links between your devices, so security problems matter here.

## Supported versions

Baton is pre-alpha. Only the `main` branch is supported, and the protocol may change without notice.

## Reporting a vulnerability

Please report vulnerabilities **privately**, using GitHub's "Report a vulnerability" button on the repository's Security tab. Do not put details in a public issue.

If there is no such button (the repository owner has to turn private reporting on), open an issue that says only that you have a security report, with no details, and a maintainer will arrange a private channel.

Include what you found, how to reproduce it, and what an attacker could do with it. We aim to acknowledge a report within seven days and will tell you what we plan to do about it. Please give us reasonable time to fix a problem before you disclose it.

## What is in scope

- The wire protocol, pairing and the trust store.
- How Baton uses cryptography and the transport.
- Anything that lets an unpaired device read, send or trigger something on a paired device.
- Anything that leaks clipboard contents, links or device keys.

## What is out of scope

- Vulnerabilities in dependencies. Report those upstream (we also run `cargo deny` against the RustSec advisory database).
- Attacks that need control of a paired device or of the unlocked machine the keys are stored on.
- Relay servers run by third parties.

The threat model is written down in [docs/security.md](docs/security.md).
