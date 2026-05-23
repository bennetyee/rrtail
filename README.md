# rrtail -- Remote Resilient Tail

The `rrtail` program fetches the contents of an append-only log file in
a resilient way.  It takes as a source file specifier
`[user@]host:path` in much the same way as `scp` and runs `ssh -l user
host tail --bytes=+0 path` to fetch the data.  If/when the `ssh`
process dies, e.g., because you're running `rrtail` on your laptop and
you're suspending it as you leave the cafe, `rrtail` will pause for a
few seconds, then run `ssh -l user host tail --bytes=+COUNT path` with
`COUNT` being the number of bytes transfered earlier.  This resumes
the log transfer process transparently from the point of view of the
consumer of `rrtail`'s output.

The duration by which `rrtail` sleeps after a network problem is
subject to exponential backoff up to an upper limit.  There are
command line arguments that control the initial value, the base value
for the exponent, and the upper bound.

## BUGS

There is no attempt to detect if the source file is not append-only.
If it is subject to log rotation (e.g., `logrotate`) and might be
renamed daily, etc, `rrtail` would probably just stop sending output
since `tail` defaults to `--follow=descriptor`.  We don't use
`--follow=name` and `--retry` (or equiv `-F`) because `rrtail` won't
know what `COUNT` value to use for the new file.

It might be possible to `ssh` into the source host, run `tail -F` with
its output going to a tool like `nc` such that these processes don't
get SIGHUP when the `ssh ` dies, except `nc -l` only accepts a single
connection and then dies, rather than provide single-client resumption
semantics.

This is the "initial" release of `rrtail`.  The network error message
matching is not well tested.

## Disclaimer

The code in this repo is entirely vibe coded using
http://aistudio.google.com/.  The only manual thing done other than
cutting-and-pasting the AI Studio generated code into the repository
is occasionally remembering to run `rustfmt`; the Studio generated
code sometimes has extraneous trailing spaces and the like.
