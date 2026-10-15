// SPDX-License-Identifier: Apache-2.0

//! The command line: every command and option, with the help text people read.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

/// Hand off links and text between your own devices.
///
/// Pair two devices once, then send a link or some text from one to the other. Everything
/// between devices is end to end encrypted, and works on the same Wi-Fi or through the
/// internet.
#[derive(Debug, Parser)]
#[command(name = "baton", version, arg_required_else_help = true)]
#[command(after_help = "Try it:\n  \
    1. on the first device:   baton listen, then type `pair offer` at its prompt\n  \
    2. on the second device:  baton pair join <ticket>\n  \
    3. on the second device:  baton send link https://example.com")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "FOLDER",
        help = "Where Baton keeps its state [default: $BATON_HOME, or a folder in your user profile]"
    )]
    pub home: Option<PathBuf>,

    /// Say more about what Baton is doing; repeat for more
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

/// A line typed at the `baton listen` prompt: the same commands, without the program name.
#[derive(Debug, Parser)]
#[command(no_binary_name = true, disable_version_flag = true)]
#[command(override_usage = "<COMMAND>")]
pub struct Prompt {
    #[command(subcommand)]
    pub command: Command,
}

/// What to do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show this device's name and fingerprint
    Id,
    /// Pair this device with another one
    #[command(subcommand)]
    Pair(PairCommand),
    /// List the devices this one is paired with
    Devices,
    /// Stop trusting a paired device
    Unpair(UnpairArgs),
    /// Check that a paired device can be reached
    Ping(PingArgs),
    /// Receive links and text until you press Ctrl+C; commands can be typed here too
    Listen,
    /// Send a link or text to a paired device
    #[command(subcommand)]
    Send(SendCommand),
    /// Read, open or remove what other devices have sent
    Inbox(InboxArgs),
}

/// How two devices are paired: one offers, the other joins.
#[derive(Debug, Subcommand)]
pub enum PairCommand {
    /// Show a ticket for the other device, and wait for it to join
    ///
    /// The ticket works once and for a few minutes. When the other device joins, both
    /// devices show a six-digit code. If they match, type yes here to finish pairing.
    ///
    /// It is better typed at the prompt of `baton listen` than run on its own: the ticket
    /// holds the addresses of the program that made it, and `listen` keeps them.
    Offer,
    /// Pair with the device that showed you a ticket
    Join {
        #[arg(help = "The ticket, which starts with BATON1:. Leave it out to be asked for it")]
        ticket: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct UnpairArgs {
    /// The device to forget: its name, or the start of its fingerprint
    pub device: String,
    /// Do not ask first
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct PingArgs {
    /// The device to check: its name, or the start of its fingerprint [default: the only paired one]
    pub device: Option<String>,
}

/// What to send.
#[derive(Debug, Subcommand)]
pub enum SendCommand {
    /// Send a web link
    Link {
        #[arg(help = "The address, such as https://example.com/page")]
        url: String,
        /// A title to show beside it
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        to: Target,
    },
    /// Send text for the other device's clipboard
    ///
    /// The words after the command are sent as they are, with a single space between
    /// them. Put the text in quotes to keep its spacing. With no words, text is read
    /// from standard input.
    Text {
        /// The text to send
        words: Vec<String>,
        #[command(flatten)]
        to: Target,
    },
    /// Send the text that is on this device's clipboard
    Clipboard {
        #[command(flatten)]
        to: Target,
    },
}

#[derive(Debug, Args)]
pub struct Target {
    /// The device to send to: its name, or the start of its fingerprint [default: the only paired one]
    #[arg(long, value_name = "DEVICE")]
    pub to: Option<String>,
}

#[derive(Debug, Args)]
pub struct InboxArgs {
    #[command(subcommand)]
    pub command: Option<InboxCommand>,
}

/// What to do with the inbox.
#[derive(Debug, Subcommand)]
pub enum InboxCommand {
    /// List what has arrived, oldest first (the default)
    List,
    /// Show one item in full
    Show {
        /// The item's number, from the list
        number: u64,
    },
    /// Open a received link in your web browser
    Open {
        /// The item's number, from the list
        number: u64,
    },
    /// Copy a received item to this device's clipboard
    Copy {
        /// The item's number, from the list
        number: u64,
    },
    /// Remove items from the inbox
    Rm {
        /// The numbers of the items to remove
        #[arg(required = true)]
        numbers: Vec<u64>,
    },
    /// Remove everything from the inbox
    Clear {
        /// Do not ask first
        #[arg(short, long)]
        yes: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
        Prompt::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(std::iter::once("baton").chain(args.iter().copied()))
            .unwrap()
            .command
    }

    #[test]
    fn sending_a_link_takes_a_title_and_a_target() {
        let Command::Send(SendCommand::Link { url, title, to }) = parse(&[
            "send",
            "link",
            "https://example.com/",
            "--title",
            "Example",
            "--to",
            "Laptop",
        ]) else {
            panic!("not a link");
        };
        assert_eq!(url, "https://example.com/");
        assert_eq!(title.as_deref(), Some("Example"));
        assert_eq!(to.to.as_deref(), Some("Laptop"));
    }

    #[test]
    fn text_is_the_words_after_the_command() {
        let Command::Send(SendCommand::Text { words, to }) =
            parse(&["send", "text", "hello", "there"])
        else {
            panic!("not text");
        };
        assert_eq!(words, ["hello", "there"]);
        assert_eq!(to.to, None);
    }

    #[test]
    fn the_inbox_lists_when_no_subcommand_is_given() {
        let Command::Inbox(args) = parse(&["inbox"]) else {
            panic!("not the inbox");
        };
        assert!(args.command.is_none());
    }

    #[test]
    fn removing_items_needs_a_number() {
        assert!(Cli::try_parse_from(["baton", "inbox", "rm"]).is_err());
        let Command::Inbox(InboxArgs {
            command: Some(InboxCommand::Rm { numbers }),
        }) = parse(&["inbox", "rm", "3", "4"])
        else {
            panic!("not a removal");
        };
        assert_eq!(numbers, [3, 4]);
    }

    #[test]
    fn the_home_folder_can_be_given_anywhere() {
        let cli = Cli::try_parse_from(["baton", "devices", "--home", "/tmp/x", "-vv"]).unwrap();
        assert_eq!(cli.home.as_deref(), Some(std::path::Path::new("/tmp/x")));
        assert_eq!(cli.verbose, 2);
    }

    #[test]
    fn a_prompt_line_has_no_program_name() {
        let prompt = Prompt::try_parse_from(["send", "link", "https://example.com/"]).unwrap();
        assert!(matches!(
            prompt.command,
            Command::Send(SendCommand::Link { .. })
        ));
        assert!(Prompt::try_parse_from(["--home", "/tmp/x", "devices"]).is_err());
    }

    #[test]
    fn nothing_at_all_asks_for_help() {
        let error = Cli::try_parse_from(["baton"]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }
}
