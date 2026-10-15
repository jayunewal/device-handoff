// SPDX-License-Identifier: Apache-2.0

//! `baton pair offer` and `baton pair join`.

use baton_core::{PairApprovalRequest, PairedDevice};
use baton_proto::ticket::PairTicket;

use super::{Mode, listen};
use crate::{console::Console, context::Context, error::CliError, select, term, util::is_yes};

/// Shows a ticket and waits for a device to join with it.
pub async fn offer(ctx: &mut Context, console: &Console, mode: Mode) -> Result<(), CliError> {
    let node = ctx.node().await?;
    let minutes = ctx.config().pairing.offer_ttl_secs / 60;
    let approver = {
        let console = console.clone();
        move |request: PairApprovalRequest| {
            let console = console.clone();
            async move { approve(&console, request).await }
        }
    };
    let offer = node.offer_pairing(approver).await?;

    println!("On the other device, run:\n");
    println!("    baton pair join {}\n", offer.ticket_text());
    println!(
        "The ticket works once and for {minutes} {}. Whoever holds it can ask to pair, and you \
         still get to say no, but send it to no one else.",
        if minutes == 1 { "minute" } else { "minutes" }
    );
    println!("Waiting for the other device...");

    match mode {
        Mode::OneShot => {
            // Anything typed while waiting is not an answer: the question comes with the
            // request, once the codes are on the screen.
            console.ignore_early_typing();
            let paired = tokio::select! {
                paired = offer.wait() => paired.map_err(CliError::from)?,
                () = crate::signal::interrupted() => return Err(CliError::new("Cancelled.")),
            };
            println!("\n{}", paired_message(&paired, mode));
            Ok(())
        }
        Mode::InSession => {
            // Back to the prompt now: the request arrives whenever the other device joins.
            let interactive = console.is_interactive();
            tokio::spawn(async move {
                match offer.wait().await {
                    Ok(paired) => println!("\n{}", paired_message(&paired, mode)),
                    Err(error) => eprintln!("\nbaton: {}", CliError::from(error)),
                }
                listen::show_prompt(interactive);
            });
            Ok(())
        }
    }
}

/// What the person is told once a device is paired, and what to do next.
fn paired_message(paired: &PairedDevice, mode: Mode) -> String {
    let who = select::describe(paired);
    match mode {
        Mode::OneShot => format!(
            "Paired with {who}.\nRun `baton listen` on one device and \
             `baton send link <address>` on the other."
        ),
        Mode::InSession => format!(
            "Paired with {who}.\nType `send link <address>` to send it something. What it sends \
             arrives here."
        ),
    }
}

/// Puts the joining device's request to the person and waits for their answer.
async fn approve(console: &Console, request: PairApprovalRequest) -> bool {
    // Asked before anything is printed, so that an answer typed (or piped) at once reaches
    // this question and not the prompt. If the person is in the middle of another question,
    // this waits until that command is done, so that a `yes` meant for it cannot pair a
    // device.
    let mut question = console.ask().await;
    println!(
        "\nA device wants to pair with this one:\n\n    Name:         {}\n    System:       {}\n    \
         Fingerprint:  {}\n",
        term::line(&request.device.name),
        term::platform(request.device.platform),
        request.fingerprint()
    );
    println!(
        "If it is yours, it is showing this code:\n\n    {}\n",
        request.confirmation
    );
    println!("Type yes to pair only if the codes are the same. Anything else says no.");
    let yes = is_yes(question.answer().await.as_deref());
    if !yes {
        println!("Declined.");
    }
    yes
}

/// Joins the offer a ticket stands for.
pub async fn join(
    ctx: &mut Context,
    console: &Console,
    mode: Mode,
    ticket: Option<String>,
) -> Result<(), CliError> {
    let text = if let Some(ticket) = ticket.filter(|ticket| ticket != "-") {
        ticket
    } else {
        let mut question = console.ask().await;
        if console.is_interactive() {
            println!("Paste the ticket from the other device, then press Enter:");
        }
        question
            .answer()
            .await
            .ok_or_else(|| CliError::new("No ticket was given."))?
    };
    let ticket: PairTicket = text
        .parse()
        .map_err(|e: baton_proto::ticket::TicketError| {
            CliError::new(format!("That ticket does not work: {e}."))
        })?;

    let node = ctx.node().await?;
    println!(
        "Contacting the device with fingerprint {}...",
        ticket.offerer.fingerprint()
    );
    let paired = node
        .join_pairing(&ticket, |code| {
            println!("\nThe code for this pairing is:\n\n    {code}\n");
            println!(
                "Check that the other device shows the same code, then type yes there. \
                 Waiting for it..."
            );
        })
        .await?;
    println!("\n{}", paired_message(&paired, mode));
    Ok(())
}
