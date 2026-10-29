package commands

import (
	"fmt"

	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/output"
)

// `tt sync status` and `tt sync`.
//
// OpenTodo exposes no sync state through ot (docs/opentodo-compatibility.md
// §6), and SPEC.md §6.7 forbids inferring "synchronized" from a command that
// merely succeeded. So this reports three separate things — sync state (always
// unavailable), whether the server could be reached, and whether local changes
// could be pending (they cannot) — and never collapses them into one
// reassuring word.

const unavailable = "OpenTodo's ot client does not report device sync state: it has no " +
	"documented sync-status command, and the sync indicator exists only in the OpenTodo app"

const noQueue = "none possible: the ot client is online-only and keeps no local copy, so " +
	"every tt write either reached the server or failed"

// Sync is `tt sync` with no subcommand: there is no documented operation to
// call, so the adapter says so rather than inventing one.
func Sync(app *App) error {
	return app.OT.Sync()
}

// SyncStatus reports what OpenTodo discloses about synchronisation.
func SyncStatus(app *App, asJSON bool) error {
	state, err := app.OT.ClientState()
	if err != nil {
		return err
	}

	var code, summary string
	var reachable *bool
	yes, no := true, false

	switch state.Kind {
	case ot.StateConfigured:
		code = "unavailable"
		summary = "Sync status: unavailable (the server was reached, which is not the same as synced)"
		reachable = &yes
	case ot.StateUnreachable:
		code = "offline"
		summary = "Sync status: offline — the OpenTodo server could not be reached"
		reachable = &no
	case ot.StateAuthFailure:
		code = "signed_out"
		summary = "Sync status: signed out — OpenTodo rejected the stored credentials"
		reachable = &no
	default:
		code = "unknown"
		summary = "Sync status: unknown — the ot client did not report its state"
	}

	if asJSON {
		var detail *string
		if state.Kind != ot.StateConfigured && state.Detail != "" {
			detail = &state.Detail
		}
		return output.PrintJSON(map[string]any{
			"syncStatus":                code,
			"syncStatusKnown":           false,
			"reason":                    unavailable,
			"serverReachable":           reachable,
			"server":                    nullable(state.Server),
			"account":                   nullable(state.Account),
			"tokenScope":                nullable(state.Scope),
			"detail":                    detail,
			"pendingLocalChanges":       0,
			"pendingLocalChangesReason": noQueue,
		})
	}

	fmt.Println(summary)
	fmt.Printf("  reason: %s\n", unavailable)
	if state.Kind == ot.StateConfigured {
		server := state.Server
		if server == "" {
			server = "an OpenTodo server"
		}
		account := ""
		if state.Account != "" {
			account = " as " + state.Account
		}
		scope := ""
		if state.Scope != "" {
			scope = " (token scope " + state.Scope + ")"
		}
		fmt.Printf("  client: configured for %s%s%s\n", server, account, scope)
	} else {
		fmt.Printf("  client: %s\n", state.Detail)
	}
	fmt.Printf("  pending local changes: %s\n", noQueue)
	fmt.Println("  to see real sync state, open the OpenTodo app: it shows Synced, Syncing, " +
		"pending counts, and errors")
	return nil
}

func nullable(value string) *string {
	if value == "" {
		return nil
	}
	return &value
}
