# Containers for running and testing `tt`

Two images from one [`Containerfile`](Containerfile), sharing a build of `tt`
and of the `ot` client it drives.

| Target | What it is for |
|---|---|
| `toolbox` | An interactive [Toolbox](https://containertoolbx.org/)/[Distrobox](https://distrobox.it/) with `tt` and `ot` on `PATH` — a place to use the CLI without installing a Rust or Go toolchain on the host |
| `test` | A non-interactive image that runs `cargo test`, and the opt-in live tests in [`../testing/`](../testing/), against a mounted working tree |

```bash
cd containers
./build.sh              # both
./build.sh toolbox      # or just one
```

The build context is the repository root — the Rust build needs the source —
and `build.sh` handles that wherever you run it from.

## The toolbox

```bash
./build.sh toolbox
toolbox create --image localhost/tt-toolbox:latest tt
toolbox enter tt

[tt]$ tt status
```

Distrobox works from the same image:

```bash
distrobox create --image localhost/tt-toolbox:latest --name tt
distrobox enter tt
```

Two things both tools do that matter here:

- **Your home directory is bind-mounted.** `ot`'s configuration lives in
  `~/.config/opentodo`, so credentials set up on the host with
  [`../deploy/bin/ot-credentials.sh`](../deploy/bin/ot-credentials.sh) work
  inside the toolbox with no further setup. One token, both places.
- **The host's network is shared.** A server on the host's own `localhost` —
  `testing/docker-compose.yml` on `:8080`, say — is reachable at the same URL
  you would use outside.

The image carries `rust` and `cargo` so you can rebuild `tt` inside it, which
is most of the point of a toolbox. That adds about 500 MB; leave it out
with:

```bash
./build.sh toolbox --build-arg TOOLBOX_EXTRA_PACKAGES=""
```

Note that Fedora's `rustc` is whatever that release ships, not the 1.88 in
`rust-version`. For a build that matches CI exactly, use the `test` image.

## The test image

```bash
./build.sh test
./run-tests.sh unit            # cargo test; no server, no network
```

That is the suite CI runs, in a pinned toolchain, without installing Rust on
the host. The repository is *mounted*, not copied, so it tests your working
tree; `CARGO_TARGET_DIR` points at a volume so a container run never fights
the host's `./target`.

For the live tests you need a throwaway server and a token:

```bash
export OPENTODO_URL=https://todo.kind.test
export OPENTODO_TOKEN=...        # ../deploy/bin/ot-credentials.sh --help
./run-tests.sh seed              # sample tasks and 3h45m of known time
./run-tests.sh live              # the nine opt-in tests in ../tests/live.rs
./run-tests.sh reset
```

**These write data.** Everything in [`../testing/README.md`](../testing/README.md)
about using a throwaway instance applies here unchanged — the scripts are the
same ones, just run inside a container.

`./run-tests.sh shell` drops you in if you would rather drive it by hand, and
`./run-tests.sh findings` runs `capture-findings.sh` for updating
[`../docs/opentodo-compatibility.md`](../docs/opentodo-compatibility.md).

## Two things about installing `ot`

**The module path moved.** Upstream's repository is now `OpenTodoApp/OpenTodo`
and its `go.mod` declares `github.com/OpenTodoApp/opentodo`. Go refuses a
module whose declared path differs from the one it was asked for, so the older
`github.com/pixari/opentodo/cmd/ot` command does not redirect, it fails:

```
module declares its path as: github.com/OpenTodoApp/opentodo
        but was required as: github.com/pixari/opentodo
```

This repository used to document the old path in four places; they were
corrected along with this directory.

The *container image* reference (`ghcr.io/pixari/opentodo`) is unaffected —
that package is still published under the old name, and the deployments in
[`../deploy/`](../deploy/) use it as-is.

**The version has to be stamped in.** `cmd/ot/main.go` declares
`var version = "dev"` and upstream's Makefile overrides it with
`-X main.version=`. A plain `go install` leaves it alone, so
`ot version --json` reports `{"output_version":1,"version":"dev"}` — which
`tt` cannot read as a version number, making it print *"could not determine
the OpenTodo client version"* before every command. The `ot` stage passes the
ldflag, and reports `v0.16.0`.

## What has been checked

Both images were built and run on podman 5.8.9 / Fedora 44.

| | |
|---|---|
| Scripts | `bash -n` clean, and wired into the CI shell-parse step |
| `ot` | Built and run. `ot version --json` returns `{"output_version":1,"version":"v0.16.0"}` — `output_version` 1 is what `tt` supports, and the leading `v` is stripped by `tt`'s semver parser. |
| The old module path | Confirmed broken by building it: Go reports the path conflict quoted above. |
| `toolbox` | `tt`, `ot`, `jq` and `cargo` all on `PATH`. `tt 0.1.0` runs and drives the real `ot` — the first time anything in this repository has done so against a genuine client rather than the mock. |
| `test` | `./run-tests.sh unit` passes: 117 tests, 9 live tests still `#[ignore]`d, the same numbers as a host `cargo test`. Build artefacts landed in the volume, the host's `./target` was untouched, and `--userns=keep-id` left the working tree owned by the invoking user. |
| **A live run against a server** | **Not done.** No image has been pointed at an OpenTodo instance, so `./run-tests.sh live`, `seed` and `findings` are unexercised. |

If upstream renames the module again, or tags a release without `cmd/ot`, the
`ot` stage is where it shows up, and `--build-arg OT_VERSION=...` is the knob.
