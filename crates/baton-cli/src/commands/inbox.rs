// SPDX-License-Identifier: Apache-2.0

//! `baton inbox`.

use baton_core::{InboxEntry, InboxItem};

use crate::{
    browser,
    cli::{InboxArgs, InboxCommand},
    clipboard,
    console::Console,
    context::Context,
    error::CliError,
    term,
    util::{is_yes, unix_now},
};

/// How much of an item a list line shows.
const PREVIEW_CHARS: usize = 70;

/// Reads or changes the inbox.
pub async fn run(ctx: &mut Context, console: &Console, args: InboxArgs) -> Result<(), CliError> {
    match args.command.unwrap_or(InboxCommand::List) {
        InboxCommand::List => list(ctx),
        InboxCommand::Show { number } => show(ctx, number),
        InboxCommand::Open { number } => open(ctx, number),
        InboxCommand::Copy { number } => copy(ctx, number),
        InboxCommand::Rm { numbers } => remove(ctx, &numbers),
        InboxCommand::Clear { yes } => clear(ctx, console, yes).await,
    }
}

/// `link` or `text`.
fn kind(entry: &InboxEntry) -> &'static str {
    match entry.item {
        InboxItem::Link { .. } => "link",
        InboxItem::Text { .. } => "text",
    }
}

/// The start of what an item holds, on one line.
fn gist(entry: &InboxEntry, chars: usize) -> String {
    match &entry.item {
        InboxItem::Link { url, .. } => term::preview(url, chars),
        InboxItem::Text { text } => term::preview(text, chars),
    }
}

/// One line saying that `entry` has arrived, for `baton listen`.
pub fn announcement(entry: &InboxEntry) -> String {
    let from = term::line(&entry.from_name);
    let number = entry.number;
    match &entry.item {
        InboxItem::Link { url, title } => {
            let title = title
                .as_deref()
                .map(|title| format!(" (\"{}\")", term::preview(title, 40)))
                .unwrap_or_default();
            format!(
                "[#{number}] link from {from}: {}{title}",
                term::preview(url, 100)
            )
        }
        InboxItem::Text { text } => format!(
            "[#{number}] text from {from} ({}): {}",
            term::size(text.len()),
            term::preview(text, 100)
        ),
    }
}

fn list(ctx: &Context) -> Result<(), CliError> {
    let entries = ctx.inbox().list()?;
    if entries.is_empty() {
        println!("The inbox is empty.");
        return Ok(());
    }
    let now = unix_now();
    let mut rows = vec![vec![
        "#".to_owned(),
        "WHEN".to_owned(),
        "FROM".to_owned(),
        "KIND".to_owned(),
        "ITEM".to_owned(),
    ]];
    for entry in &entries {
        rows.push(vec![
            entry.number.to_string(),
            term::ago(now, entry.received_at),
            term::line(&entry.from_name),
            kind(entry).to_owned(),
            gist(entry, PREVIEW_CHARS),
        ]);
    }
    print!("{}", term::table(&rows));
    println!("\n`baton inbox show <number>` shows an item in full; `open` and `copy` use it.");
    Ok(())
}

/// The item `number`, or an error that says it is not there.
fn find(ctx: &Context, number: u64) -> Result<InboxEntry, CliError> {
    ctx.inbox().get(number)?.ok_or_else(|| {
        CliError::new(format!(
            "There is no item #{number} in the inbox. `baton inbox` lists what is there."
        ))
    })
}

fn show(ctx: &Context, number: u64) -> Result<(), CliError> {
    let entry = find(ctx, number)?;
    let sender = format!(
        "{} ({})",
        term::line(&entry.from_name),
        entry.from.fingerprint()
    );
    println!(
        "Item #{number}, {} from {sender}, {}",
        kind(&entry),
        term::ago(unix_now(), entry.received_at)
    );
    match &entry.item {
        InboxItem::Link { url, title } => {
            println!("\nAddress: {}", term::line(url));
            if let Some(title) = title {
                println!("Title:   {}", term::line(title));
            }
        }
        InboxItem::Text { text } => {
            println!("\n{}", term::size(text.len()));
            println!("----");
            println!("{}", term::block(text));
            println!("----");
        }
    }
    Ok(())
}

fn open(ctx: &Context, number: u64) -> Result<(), CliError> {
    let entry = find(ctx, number)?;
    let InboxItem::Link { url, .. } = &entry.item else {
        return Err(CliError::new(format!(
            "Item #{number} is text, not a link. `baton inbox copy {number}` puts it on the clipboard."
        )));
    };
    let opened = browser::open(url)?;
    println!("Opened {} in your browser.", term::line(opened.as_str()));
    Ok(())
}

fn copy(ctx: &Context, number: u64) -> Result<(), CliError> {
    let entry = find(ctx, number)?;
    let text = match &entry.item {
        InboxItem::Link { url, .. } => url,
        InboxItem::Text { text } => text,
    };
    clipboard::write_text(text, || {
        println!(
            "On Linux the text stays on the clipboard only while this command runs. Paste it \
             where you need it, then press Ctrl+C."
        );
    })?;
    println!("Copied item #{number} to the clipboard.");
    Ok(())
}

fn remove(ctx: &Context, numbers: &[u64]) -> Result<(), CliError> {
    let mut removed = 0;
    for &number in numbers {
        if ctx.inbox().remove(number)?.is_some() {
            println!("Removed #{number}.");
            removed += 1;
        } else {
            println!("There is no item #{number}.");
        }
    }
    if removed == 0 {
        return Err(CliError::new("Nothing was removed."));
    }
    Ok(())
}

async fn clear(ctx: &Context, console: &Console, yes: bool) -> Result<(), CliError> {
    let count = ctx.inbox().list()?.len();
    if count == 0 {
        println!("The inbox is already empty.");
        return Ok(());
    }
    // Kept until the result is printed, so that a pairing request that arrived meanwhile is
    // put to the person after it, not between their answer and it.
    let mut question = if yes { None } else { Some(console.ask().await) };
    if let Some(question) = question.as_mut() {
        println!(
            "Remove all {count} {} from the inbox? Type yes to remove them.",
            if count == 1 { "item" } else { "items" }
        );
        if !is_yes(question.answer().await.as_deref()) {
            println!("Nothing changed.");
            return Ok(());
        }
    }
    let removed = ctx.inbox().clear()?;
    println!(
        "Removed {removed} {}.",
        if removed == 1 { "item" } else { "items" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use baton_proto::DeviceId;

    use super::*;

    fn entry(item: InboxItem) -> InboxEntry {
        InboxEntry {
            number: 7,
            from: DeviceId::from_bytes([1; 32]),
            from_name: "Laptop".to_owned(),
            received_at: 1_800_000_000,
            item,
        }
    }

    #[test]
    fn a_link_is_announced_with_its_title() {
        let line = announcement(&entry(InboxItem::Link {
            url: "https://example.com/a".to_owned(),
            title: Some("Example".to_owned()),
        }));
        assert_eq!(
            line,
            "[#7] link from Laptop: https://example.com/a (\"Example\")"
        );
    }

    #[test]
    fn text_is_announced_with_its_size_and_start() {
        let line = announcement(&entry(InboxItem::Text {
            text: "hello there".to_owned(),
        }));
        assert_eq!(line, "[#7] text from Laptop (11 bytes): hello there");
    }

    #[test]
    fn what_is_announced_cannot_act_on_the_terminal() {
        let line = announcement(&entry(InboxItem::Text {
            text: "\u{1b}[2Jgone".to_owned(),
        }));
        assert!(!line.contains('\u{1b}'));
        let line = announcement(&entry(InboxItem::Link {
            url: "https://example.com/".to_owned(),
            title: Some("a\u{1b}]0;x\u{7}b".to_owned()),
        }));
        assert!(!line.contains('\u{1b}') && !line.contains('\u{7}'));
    }

    #[test]
    fn long_text_is_cut_in_an_announcement() {
        let line = announcement(&entry(InboxItem::Text {
            text: "x".repeat(500),
        }));
        assert!(line.ends_with('…'));
        assert!(line.chars().count() < 160);
    }

    #[test]
    fn the_list_names_the_kind() {
        assert_eq!(
            kind(&entry(InboxItem::Text {
                text: String::new()
            })),
            "text"
        );
        assert_eq!(
            kind(&entry(InboxItem::Link {
                url: String::new(),
                title: None
            })),
            "link"
        );
    }
}
