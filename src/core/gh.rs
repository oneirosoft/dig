use std::io;
use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestSummary {
    pub number: u64,
    pub base_ref_name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestDetails {
    pub number: u64,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePullRequestOptions {
    pub base_branch_name: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedPullRequest {
    pub number: u64,
    pub url: String,
}

#[derive(Debug)]
struct GhCommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl GhCommandOutput {
    fn combined_output(&self) -> String {
        let stdout = self.stdout.trim();
        let stderr = self.stderr.trim();

        match (stdout.is_empty(), stderr.is_empty()) {
            (true, true) => String::new(),
            (false, true) => stdout.to_string(),
            (true, false) => stderr.to_string(),
            (false, false) => format!("{stdout}\n{stderr}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PullRequestSummaryRecord {
    number: u64,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestViewRecord {
    number: u64,
    url: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestDetailsRecord {
    number: u64,
    title: String,
    url: String,
}

pub fn list_open_pull_requests_for_head(branch_name: &str) -> io::Result<Vec<PullRequestSummary>> {
    let output = run_gh_capture_output(&[
        "pr".to_string(),
        "list".to_string(),
        "--head".to_string(),
        branch_name.to_string(),
        "--state".to_string(),
        "open".to_string(),
        "--json".to_string(),
        "number,baseRefName,url".to_string(),
    ])?;

    parse_open_pull_requests(&output.stdout)
}

pub fn create_pull_request(options: &CreatePullRequestOptions) -> io::Result<CreatedPullRequest> {
    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--base".to_string(),
        options.base_branch_name.clone(),
    ];

    if let Some(title) = &options.title {
        args.push("--title".to_string());
        args.push(title.clone());
    }

    if let Some(body) = &options.body {
        args.push("--body".to_string());
        args.push(body.clone());
    }

    if options.draft {
        args.push("--draft".to_string());
    }

    let output = run_gh_with_live_output(&args)?;
    if !output.status.success() {
        return Err(gh_command_failed(
            "gh pr create",
            &output.stdout,
            &output.stderr,
        ));
    }

    let url = find_pull_request_url(&output.combined_output()).ok_or_else(|| {
        io::Error::other("gh pr create succeeded but did not report a pull request URL")
    })?;

    if let Some(number) = pull_request_number_from_url(&url) {
        return Ok(CreatedPullRequest { number, url });
    }

    view_pull_request_by_url(&url)
}

pub fn list_open_pull_requests() -> io::Result<Vec<PullRequestDetails>> {
    let output = run_gh_capture_output(&[
        "pr".to_string(),
        "list".to_string(),
        "--state".to_string(),
        "open".to_string(),
        "--json".to_string(),
        "number,title,url".to_string(),
    ])?;

    parse_open_pull_request_details(&output.stdout)
}

pub fn open_current_pull_request_in_browser() -> io::Result<()> {
    run_gh_command(
        "gh pr view --web",
        &["pr".to_string(), "view".to_string(), "--web".to_string()],
    )
}

pub fn open_pull_request_in_browser(number: u64) -> io::Result<()> {
    run_gh_command(
        "gh pr view --web",
        &[
            "pr".to_string(),
            "view".to_string(),
            number.to_string(),
            "--web".to_string(),
        ],
    )
}

fn view_pull_request_by_url(url: &str) -> io::Result<CreatedPullRequest> {
    let output = run_gh_capture_output(&[
        "pr".to_string(),
        "view".to_string(),
        url.to_string(),
        "--json".to_string(),
        "number,url".to_string(),
    ])?;
    let record: PullRequestViewRecord = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(CreatedPullRequest {
        number: record.number,
        url: record.url,
    })
}

fn parse_open_pull_requests(stdout: &str) -> io::Result<Vec<PullRequestSummary>> {
    let records: Vec<PullRequestSummaryRecord> = serde_json::from_str(stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(records
        .into_iter()
        .map(|record| PullRequestSummary {
            number: record.number,
            base_ref_name: record.base_ref_name,
            url: record.url,
        })
        .collect())
}

fn parse_open_pull_request_details(stdout: &str) -> io::Result<Vec<PullRequestDetails>> {
    let records: Vec<PullRequestDetailsRecord> = serde_json::from_str(stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(records
        .into_iter()
        .map(|record| PullRequestDetails {
            number: record.number,
            title: record.title,
            url: record.url,
        })
        .collect())
}

fn find_pull_request_url(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|token| token.contains("/pull/"))
        .map(|token| {
            token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '(' | ')' | '[' | ']'))
        })
        .map(str::to_string)
}

fn pull_request_number_from_url(url: &str) -> Option<u64> {
    let (_, suffix) = url.rsplit_once("/pull/")?;
    let digits = suffix
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();

    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn run_gh_capture_output(args: &[String]) -> io::Result<GhCommandOutput> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(normalize_gh_spawn_error)?;

    output_to_gh_command_output(output)
}

fn run_gh_command(command_name: &str, args: &[String]) -> io::Result<()> {
    let output = run_gh_capture_output(args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(gh_command_failed(
            command_name,
            &output.stdout,
            &output.stderr,
        ))
    }
}

fn run_gh_with_live_output(args: &[String]) -> io::Result<GhCommandOutput> {
    let mut child = Command::new("gh")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(normalize_gh_spawn_error)?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("failed to capture gh stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("failed to capture gh stderr"))?;

    let stdout_handle = thread::spawn(move || stream_and_capture(stdout, false));
    let stderr_handle = thread::spawn(move || stream_and_capture(stderr, true));

    let status = child.wait()?;
    let stdout = join_capture_thread(stdout_handle, "stdout")?;
    let stderr = join_capture_thread(stderr_handle, "stderr")?;

    Ok(GhCommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn stream_and_capture<R>(mut reader: R, use_stderr: bool) -> io::Result<String>
where
    R: Read,
{
    let mut buffer = [0_u8; 4096];
    let mut captured = Vec::new();

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        if use_stderr {
            let mut stderr = io::stderr();
            stderr.write_all(&buffer[..read])?;
            stderr.flush()?;
        } else {
            let mut stdout = io::stdout();
            stdout.write_all(&buffer[..read])?;
            stdout.flush()?;
        }

        captured.extend_from_slice(&buffer[..read]);
    }

    String::from_utf8(captured).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn join_capture_thread(
    handle: thread::JoinHandle<io::Result<String>>,
    stream_name: &str,
) -> io::Result<String> {
    handle
        .join()
        .map_err(|_| io::Error::other(format!("failed to join gh {stream_name} capture thread")))?
}

fn output_to_gh_command_output(output: Output) -> io::Result<GhCommandOutput> {
    Ok(GhCommandOutput {
        status: output.status,
        stdout: String::from_utf8(output.stdout)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
        stderr: String::from_utf8(output.stderr)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
    })
}

fn normalize_gh_spawn_error(err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::NotFound {
        io::Error::other("gh CLI is not installed or not found on PATH")
    } else {
        err
    }
}

fn gh_command_failed(command_name: &str, stdout: &str, stderr: &str) -> io::Error {
    let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.trim().to_string(),
        (true, false) => stderr.trim().to_string(),
        (false, false) => format!("{}\n{}", stdout.trim(), stderr.trim()),
    };

    if looks_like_auth_error(&combined) {
        return io::Error::other("gh authentication failed; run 'gh auth login'");
    }

    if combined.is_empty() {
        io::Error::other(format!("{command_name} failed"))
    } else {
        io::Error::other(format!("{command_name} failed: {combined}"))
    }
}

fn looks_like_auth_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();

    normalized.contains("gh auth login")
        || normalized.contains("not logged into any github hosts")
        || normalized.contains("authentication")
}

#[cfg(test)]
mod tests {
    use super::{
        find_pull_request_url, parse_open_pull_request_details, parse_open_pull_requests,
        pull_request_number_from_url,
    };

    #[test]
    fn parses_open_pull_request_list_output() {
        let pull_requests = parse_open_pull_requests(
            r#"[{"number":123,"baseRefName":"main","url":"https://github.com/acme/dig/pull/123"}]"#,
        )
        .unwrap();

        assert_eq!(pull_requests.len(), 1);
        assert_eq!(pull_requests[0].number, 123);
        assert_eq!(pull_requests[0].base_ref_name, "main");
    }

    #[test]
    fn parses_open_pull_request_details_output() {
        let pull_requests = parse_open_pull_request_details(
            r#"[{"number":123,"title":"Auth PR","url":"https://github.com/acme/dig/pull/123"}]"#,
        )
        .unwrap();

        assert_eq!(pull_requests[0].title, "Auth PR");
    }

    #[test]
    fn extracts_pull_request_url_and_number_from_create_output() {
        let url = find_pull_request_url(
            "Creating pull request for feat/auth into main in acme/dig.\nhttps://github.com/acme/dig/pull/456\n",
        )
        .unwrap();

        assert_eq!(url, "https://github.com/acme/dig/pull/456");
        assert_eq!(pull_request_number_from_url(&url), Some(456));
    }

    #[test]
    fn ignores_non_pull_request_urls_when_extracting_number() {
        assert_eq!(
            pull_request_number_from_url("https://github.com/acme/dig/issues/456"),
            None
        );
    }
}
