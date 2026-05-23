use clap::Parser;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Parser, Debug)]
#[command(
    name = "rrtail",
    version = "0.1.0",
    about = "Remote Resilient Tail - Resilient remote tailing over SSH"
)]
struct Args {
    #[arg(short = 'P', help = "SSH port to use")]
    port: Option<u16>,

    #[arg(short = 'F', help = "Alternate SSH config file to use")]
    ssh_config: Option<String>,

    #[arg(short = 'i', help = "Identity file to use")]
    identity_file: Option<String>,

    #[arg(short = 'c', help = "Cipher specification to pass to ssh")]
    cipher_spec: Option<String>,

    #[arg(short = 'B', help = "Bind interface to pass to ssh")]
    bind_interface: Option<String>,

    #[arg(short = 'b', help = "Bind address to pass to ssh")]
    bind_address: Option<String>,

    #[arg(
        long,
        default_value_t = 0,
        help = "Starting byte offset (0-based) from which to tail"
    )]
    starting_byte: u64,

    #[arg(
        long,
        default_value = "1s",
        value_parser = parse_duration,
        help = "Initial pause after ssh failing (e.g., '1s', '500ms')"
    )]
    initial_retry_backoff: Duration,

    #[arg(
        long,
        default_value_t = 2.0,
        value_parser = validate_multiplier,
        help = "Exponential backoff multiplier for immediate network failures (must be >= 1.0)"
    )]
    retry_backoff_multiplier: f64,

    #[arg(
        long,
        default_value_t = -1,
        help = "Maximum number of retries before giving up (-1 for infinite)"
    )]
    max_retries: i32,

    #[arg(
        long,
        default_value = "10m",
        value_parser = parse_duration,
        help = "Upper-bound for the retry sleep (e.g., '10m')"
    )]
    max_retry_backoff: Duration,

    #[arg(
        long,
        help = "Pass SSH stderr output through to rrtail's stderr in real-time"
    )]
    show_ssh_errors: bool,

    #[arg(help = "Source specification in the format [user@]host:pathspec")]
    source: String,
}

fn parse_duration(s: &str) -> Result<Duration, humantime::DurationError> {
    humantime::parse_duration(s)
}

fn validate_multiplier(s: &str) -> Result<f64, String> {
    let val: f64 = s
        .parse()
        .map_err(|_| format!("'{}' is not a valid float", s))?;
    if val < 1.0 {
        return Err("Multiplier must be at least 1.0".to_string());
    }
    Ok(val)
}

fn parse_source_spec(source: &str) -> Result<(Option<String>, String, String), String> {
    let parts: Vec<&str> = source.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(
            "Source specification must be in the format '[user@]host:pathspec'".to_string(),
        );
    }

    let user_host = parts[0];
    let pathspec = parts[1].to_string();
    if pathspec.is_empty() {
        return Err("Path specification cannot be empty".to_string());
    }

    if let Some(at_idx) = user_host.rfind('@') {
        let user = user_host[..at_idx].to_string();
        let host = user_host[at_idx + 1..].to_string();
        if user.is_empty() || host.is_empty() {
            return Err("Invalid user or host in source specification".to_string());
        }
        Ok((Some(user), host, pathspec))
    } else {
        if user_host.is_empty() {
            return Err("Host cannot be empty".to_string());
        }
        Ok((None, user_host.to_string(), pathspec))
    }
}

fn is_immediate_network_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("could not resolve hostname")
        || s.contains("network is unreachable")
        || s.contains("connection refused")
        || s.contains("connection timed out")
        || s.contains("temporary failure in name resolution")
        || s.contains("no route to host")
        || s.contains("connection closed by remote host")
        || s.contains("connection reset")
        || s.contains("lost connection")
        || s.contains("ssh: connect to host")
        || s.contains("kex_exchange_identification")
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let (user, host, pathspec) = match parse_source_spec(&args.source) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Configuration Error: {}", e);
            std::process::exit(1);
        }
    };

    let mut bytes_transferred: u64 = 0;
    let mut retry_count = 0;
    let mut current_backoff = args.initial_retry_backoff;

    loop {
        let target_offset = args.starting_byte + bytes_transferred;
        let tail_bytes_arg = format!("+{}", target_offset + 1);

        let mut cmd = tokio::process::Command::new("ssh");

        if let Some(port) = args.port {
            cmd.arg("-p").arg(port.to_string());
        }
        if let Some(ref config) = args.ssh_config {
            cmd.arg("-F").arg(config);
        }
        if let Some(ref identity) = args.identity_file {
            cmd.arg("-i").arg(identity);
        }
        if let Some(ref cipher) = args.cipher_spec {
            cmd.arg("-c").arg(cipher);
        }
        if let Some(ref bind_interface) = args.bind_interface {
            cmd.arg("-B").arg(bind_interface);
        }
        if let Some(ref bind_addr) = args.bind_address {
            cmd.arg("-b").arg(bind_addr);
        }
        if let Some(ref u) = user {
            cmd.arg("-l").arg(u);
        }

        cmd.arg(&host);

        let tail_cmd = format!("tail -f --bytes={} {}", tail_bytes_arg, pathspec);
        cmd.arg(tail_cmd);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let start_time = Instant::now();

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error spawning ssh process: {}", e);
                if args.max_retries >= 0 && retry_count >= args.max_retries {
                    eprintln!("Reached maximum retries ({}), giving up.", args.max_retries);
                    std::process::exit(1);
                }
                retry_count += 1;
                let sleep_duration = args.initial_retry_backoff;
                eprintln!(
                    "Waiting {} before retrying...",
                    humantime::format_duration(sleep_duration)
                );
                tokio::time::sleep(sleep_duration).await;
                continue;
            }
        };

        let mut stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");

        // Background task reads stderr chunk-by-chunk. If show_ssh_errors is true,
        // it forwards the data to the local stderr stream in real-time.
        let show_ssh_errors = args.show_ssh_errors;
        let stderr_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut reader = stderr;
            let mut temp_buf = vec![0u8; 1024];
            let mut local_stderr = tokio::io::stderr();
            loop {
                match reader.read(&mut temp_buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&temp_buf[..n]);
                        if show_ssh_errors {
                            let _ = local_stderr.write_all(&temp_buf[..n]).await;
                            let _ = local_stderr.flush().await;
                        }
                    }
                    Err(_) => break,
                }
            }
            buf
        });

        let mut local_stdout = tokio::io::stdout();
        let mut stdout_buf = vec![0u8; 8192];
        let mut run_bytes_transferred = 0;

        loop {
            match stdout.read(&mut stdout_buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = local_stdout.write_all(&stdout_buf[..n]).await {
                        eprintln!("Error writing to standard output: {}", e);
                        break;
                    }
                    let _ = local_stdout.flush().await;
                    bytes_transferred += n as u64;
                    run_bytes_transferred += n as u64;
                }
                Err(e) => {
                    eprintln!("Error reading remote standard output: {}", e);
                    break;
                }
            }
        }

        let status_res = child.wait().await;
        let stderr_data = stderr_handle.await.unwrap_or_default();
        let stderr_str = String::from_utf8_lossy(&stderr_data);

        let run_duration = start_time.elapsed();

        if let Ok(status) = status_res {
            if !status.success() {
                eprintln!("ssh process exited with status: {}", status);
                // If standard error was not already streamed in real-time,
                // output the captured block on connection failure.
                if !show_ssh_errors && !stderr_str.is_empty() {
                    eprintln!("ssh stderr:\n{}", stderr_str.trim_end());
                }
            }
        } else if let Err(e) = status_res {
            eprintln!("Error waiting for ssh process: {}", e);
        }

        if run_bytes_transferred > 0 || run_duration > Duration::from_secs(10) {
            retry_count = 0;
            current_backoff = args.initial_retry_backoff;
        }

        let is_net_err = is_immediate_network_failure(&stderr_str);

        if args.max_retries >= 0 && retry_count >= args.max_retries {
            eprintln!("Reached maximum retries ({}), giving up.", args.max_retries);
            std::process::exit(1);
        }

        let sleep_duration = if is_net_err && run_duration < Duration::from_secs(5) {
            let dur = current_backoff;
            let next_backoff_secs = current_backoff.as_secs_f64() * args.retry_backoff_multiplier;
            current_backoff =
                Duration::from_secs_f64(next_backoff_secs).min(args.max_retry_backoff);
            dur
        } else {
            args.initial_retry_backoff
        };

        eprintln!(
            "Disconnected. Retrying in {} (Attempt {}/{})...",
            humantime::format_duration(sleep_duration),
            retry_count + 1,
            if args.max_retries >= 0 {
                args.max_retries.to_string()
            } else {
                "∞".to_string()
            }
        );

        tokio::time::sleep(sleep_duration).await;
        retry_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_spec() {
        assert_eq!(
            parse_source_spec("alice@example.com:/var/log/syslog"),
            Ok((
                Some("alice".to_string()),
                "example.com".to_string(),
                "/var/log/syslog".to_string()
            ))
        );
        assert_eq!(
            parse_source_spec("example.com:log.txt"),
            Ok((None, "example.com".to_string(), "log.txt".to_string()))
        );
        assert!(parse_source_spec("example.com").is_err());
    }
}
