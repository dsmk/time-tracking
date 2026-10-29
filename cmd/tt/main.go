// Command tt is a terminal client for OpenTodo time tracking.
//
// OpenTodo is the system of record. tt runs the documented `ot` client, reads
// its documented JSON, and keeps no time-entry storage of its own.
package main

import (
	"errors"
	"fmt"
	"os"

	// The IANA database is embedded so that day and week boundaries are the
	// same on a host with no tzdata installed — a scratch container, most
	// often — as they are on a developer's machine.
	_ "time/tzdata"

	"github.com/dsmk/time-tracking/internal/cli"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// version is stamped at build time with -ldflags="-X main.version=...".
var version = "dev"

func main() {
	cli.Version = version
	err := cli.New().Execute()
	if err == nil {
		return
	}
	fmt.Fprintf(os.Stderr, "tt: %v\n", err)

	// Everything tt raises itself is a *tterr.Error carrying its own exit
	// code. Anything else came from the argument parser, which only ever
	// reports a usage problem.
	var ttError *tterr.Error
	if errors.As(err, &ttError) {
		os.Exit(ttError.ExitCode())
	}
	os.Exit(tterr.KindUsage.ExitCode())
}
