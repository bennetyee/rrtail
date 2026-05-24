# rrtail -- Remote Resilient Tail

The `rrtail` program fetches the contents of an append-only log file
in a resilient way, so that temporary networking problems are masked.
This means that you can `rrtail` a log file from a remote machine
(accessible via `ssh`) while at work, close your laptop, go to a cafe,
home, the airport, etc, and after waking up the laptop and having it
get a new network address, the data from the log file will just
continue to stream from where you left off, as if no interruption had
occurred.

`Rrtail` takes as a source file specifier `[user@]host:path` in much
the same way as `scp` and runs `ssh -l user host tail --bytes=+0 path`
to fetch the data.  If/when the `ssh` process dies, e.g., because
you're running `rrtail` on your laptop and you're suspending it as you
leave the cafe, `rrtail` will pause for a few seconds, then run `ssh
-o TcpKeepAlive=Yes -l user host tail --bytes=+COUNT path` with
`COUNT` being the number of bytes transfered earlier.  This resumes
the log transfer process transparently from the point of view of the
consumer of `rrtail`'s output.  This _masks the network failure_.

The duration by which `rrtail` sleeps after a network problem is
subject to exponential backoff up to an upper limit.  There are
command line arguments that control the initial value, the base value
for the exponent, and the upper bound.

It is assumed that the necessary `ssh` credentials are stored in a
keyring, so that no password interaction is necessary when
re-establishing the remote `tail` process.

## EXAMPLE

### Remote Monitoring

```sh
$ rrtail hostname:/var/log/auth.log
```

```sh
$ rrtail hostname:/var/log/syslog
```

See BUGS below.

### Visualization

```sh
$ rrtail hostname:span-data/all-circuits.log | eval live_plotter --timestamp --labels $(cat span-data/labels) -v $(expr 60 \* 60 \* 24 \* 2)
```

Here, we use `rrtail` to grab log data that is used to generate a
bunch of time-series plots that allows us to visualize energy usage
according to the SPAN smart breaker panel.

See https://github.com/bennetyee/live_plotter for details of that program.

## BUGS

There is no attempt to detect if the source file is not actually
append-only.

If the source file is subject to log rotation (e.g., `logrotate`) and
might be renamed daily, etc, `rrtail` would probably just stop sending
output since `tail` defaults to `--follow=descriptor`.  We don't use
`--follow=name` and `--retry` (or equiv `-F`) because `rrtail` won't
know what `COUNT` value to use for the new file.

It might be possible to `ssh` into the source host, run `tail -F` with
its output going to a tool like `nc` such that these processes don't
get SIGHUP when the `ssh ` dies, except `nc -l` only accepts a single
connection and then exits, rather than provide single-client
resumption semantics.  Alternatively, we could also run a more
stateful streaming server program on the source machine than just
`tail`, and use an actual protocol between its client (a new version
of `rrtail`) and that server to handle resumption.  Garbage collecting
that server process -- distinguishing between pending resumption and
the client exiting (including via hard reboots) would be difficult.

This is the "initial" release of `rrtail`.  The network error message
matching is not well tested.

## Disclaimer

The code in this repo is entirely vibe coded using
http://aistudio.google.com/.  The only manual thing done other than
cutting-and-pasting the AI Studio generated code into the repository
is occasionally remembering to run `rustfmt`; the Studio generated
code sometimes has extraneous trailing spaces and the like.
