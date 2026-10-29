//! `tt sync status` and `tt sync`.
//!
//! OpenTodo exposes no sync state through `ot` (docs/opentodo-compatibility.md
//! §6), and `SPEC.md` §6.7 forbids inferring "synchronized" from a command that
//! merely succeeded. So this reports three separate things — sync state (always
//! unavailable), whether the server could be reached, and whether local changes
//! could be pending (they cannot) — and never collapses them into one
//! reassuring word.

use super::App;
use crate::cli::{JsonFlag, SyncArgs, SyncCommand};
use crate::error::Result;
use crate::ot::ClientState;
use crate::output;

const UNAVAILABLE: &str =
    "OpenTodo's ot client does not report device sync state: it has no documented sync-status \
     command, and the sync indicator exists only in the OpenTodo app";
const NO_QUEUE: &str =
    "none possible: the ot client is online-only and keeps no local copy, so every tt write \
     either reached the server or failed";

pub fn run(app: &App, args: &SyncArgs) -> Result<()> {
    match &args.command {
        Some(SyncCommand::Status(flag)) => status(app, flag),
        // No documented operation to call; the adapter says so rather than
        // inventing one.
        None => app.ot.sync(),
    }
}

fn status(app: &App, args: &JsonFlag) -> Result<()> {
    let state = app.ot.client_state()?;

    let (code, summary, reachable) = match &state {
        ClientState::Configured { .. } => (
            "unavailable",
            "Sync status: unavailable (the server was reached, which is not the same as synced)",
            Some(true),
        ),
        ClientState::Unreachable(_) => (
            "offline",
            "Sync status: offline — the OpenTodo server could not be reached",
            Some(false),
        ),
        ClientState::AuthFailure(_) => (
            "signed_out",
            "Sync status: signed out — OpenTodo rejected the stored credentials",
            Some(false),
        ),
        ClientState::Unknown(_) => (
            "unknown",
            "Sync status: unknown — the ot client did not report its state",
            None,
        ),
    };

    if args.json {
        let (server, account, scope) = match &state {
            ClientState::Configured { server, account, scope } => {
                (server.clone(), account.clone(), scope.clone())
            }
            _ => (None, None, None),
        };
        let detail = match &state {
            ClientState::Unreachable(d) | ClientState::AuthFailure(d) | ClientState::Unknown(d) => {
                Some(d.clone())
            }
            ClientState::Configured { .. } => None,
        };
        return output::print_json(&serde_json::json!({
            "syncStatus": code,
            "syncStatusKnown": false,
            "reason": UNAVAILABLE,
            "serverReachable": reachable,
            "server": server,
            "account": account,
            "tokenScope": scope,
            "detail": detail,
            "pendingLocalChanges": 0,
            "pendingLocalChangesReason": NO_QUEUE,
        }));
    }

    println!("{summary}");
    println!("  reason: {UNAVAILABLE}");
    match &state {
        ClientState::Configured { server, account, scope } => {
            println!(
                "  client: configured for {}{}{}",
                server.as_deref().unwrap_or("an OpenTodo server"),
                account.as_deref().map(|a| format!(" as {a}")).unwrap_or_default(),
                scope.as_deref().map(|s| format!(" (token scope {s})")).unwrap_or_default()
            );
        }
        ClientState::Unreachable(detail)
        | ClientState::AuthFailure(detail)
        | ClientState::Unknown(detail) => println!("  client: {detail}"),
    }
    println!("  pending local changes: {NO_QUEUE}");
    println!("  to see real sync state, open the OpenTodo app: it shows Synced, Syncing, pending counts, and errors");
    Ok(())
}
