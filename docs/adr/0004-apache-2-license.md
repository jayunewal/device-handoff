# 0004: Apache-2.0, and no code copied from copyleft projects

Status: accepted

## Context

Baton is meant to be adopted widely and shipped through app stores. The projects closest to it have mixed licenses: RustDesk is AGPL-3.0, KDE Connect and Sunshine are GPL, LocalSend and scrcpy are Apache-2.0, iroh is MIT or Apache-2.0.

## Decision

License Baton under Apache-2.0. It is permissive, includes an explicit patent grant, and is compatible with our main dependencies.

Learn from RustDesk, KDE Connect, Sunshine and other copyleft projects by reading their designs and documentation, but do not copy or translate their code. Contributors must not paste code that cannot be relicensed under Apache-2.0. CI runs `cargo deny` so a dependency with an incompatible license fails the build.

## Alternatives considered

- **MIT.** Similar, without the patent grant.
- **GPL or AGPL.** Would let us reuse more existing code, but would complicate distribution through app stores and discourage commercial and embedded use.
- **MPL-2.0.** A reasonable middle ground, but less familiar to contributors and companies.

## Consequences

- Anyone can use, modify and redistribute Baton, including in closed-source products.
- We cannot reuse code from the GPL and AGPL projects above.
- Dependencies are restricted to permissive licenses, with MPL-2.0 allowed when used unmodified.
