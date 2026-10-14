# Locked-device research (2026-10-14)

Three research passes behind [locked-devices.md](../../locked-devices.md). They are working notes, written from documentation, source code and reports, and they are committed so that the labels on that page can be checked against the evidence behind them.

Nothing in them was tested on a device. Every claim carries the confidence label its author gave it: documented (`[C]`, `[DOC]`, `[DOCS]`), reported (`[S]`, `[REPORT]`, `[CODE]`), inferred (`[I]`, `[INFER]`), unknown, and in one note `[IDEA]` for a design proposal. A `*` marks a page that was read through a summarising tool. Some pages reached the researchers only that way, and the search budget ran out before every question was covered, so re-read a source before relying on a claim that matters.

| File | Covers |
| --- | --- |
| [windows.md](windows.md) | Windows: sessions, Modern Standby and S3, capture and input on the secure desktop, detection APIs, tests T01 onwards |
| [apple.md](apple.md) | macOS, iOS and iPadOS: locked session behaviour, sleep and Dark Wake, Local Network privacy, push, file protection, tests T1 onwards |
| [linux-android-tools.md](linux-android-tools.md) | Linux (GNOME, KDE, X11), Android, existing remote tools (RustDesk, KDE Connect, Chrome Remote Desktop and others), mailbox and wake options, tests V-L, V-A onwards |
| [review-additions.md](review-additions.md) | Facts added while the page was reviewed, each with its source and the way it was read |

The last file is not a research pass. It holds the facts that came in while the page was reviewed, in the same format, so that their labels can be checked too.

Where a note and the page disagree, the page rules. It is stricter than some notes propose: Baton has no system-level service by default, and nothing touches a lock screen at all.

The notes number their tests T (Windows and Apple) and V (Linux and Android). Section 8 of the page uses its own ids (W, M, L, A, I, D) and names the original id where a test comes from a note.

Left out or changed in this copy:

- Step-level procedures for reaching or typing into a lock screen: a SYSTEM-level helper on Windows, input posted at a lock screen, and shell-level input on Android. Baton never unlocks or types into a lock screen (section 5.5 of the page), so each is replaced by a short stub that says what it covered.
- Entries about other file-sharing tools that Baton does not follow, and wording about who asked for the research and how the research sessions were run.
- Punctuation in one quotation from Apple (the PushKit entry in apple.md). The original has long dashes and the copy has commas. The words are unchanged.

Corrections made during review are lines that begin "Correction (review, 2026-10-14)", placed under the text they correct. The original text stays.
