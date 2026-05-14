# WASI surface

> **TODO** — write this after M1 ships, before M2 expands the import
> surface. Should specify exactly which WASI calls the host exposes to
> plugins, which are stubbed, and which are forbidden at audio-thread
> time. The riskiest part of the design — get it on paper before code.
>
> Suggested sections:
> - Allowed-at-init vs. allowed-on-audio-thread call lists
> - Stubs (e.g. random_get returns deterministic bytes? real entropy?)
> - Filesystem policy (probably: deny in v1, revisit if a plugin needs
>   it)
> - Time/clock policy (monotonic only? wall clock?)
> - Logging path for `fd_write` to stdout/stderr
