use std::io;
use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::{env, thread};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestStatus {
    pub number: u64,
    pub state: PullRequestState,
    pub merged_at: Option<String>,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub head_ref_oid: Option<String>,
    pub is_draft: bool,
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
    pub display_url_in_dagger: bool,
}

#[derive(Debug)]
struct GhCommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl GhCommandOutput {
    fn combined_output(&self) -> String {
        combine_command_output(&self.stdout, &self.stderr)
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

#[derive(Debug, Deserialize)]
struct PullRequestStatusRecord {
    number: u64,
    state: PullRequestState,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid", default)]
    head_ref_oid: Option<String>,
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
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
    let args = build_create_pull_request_args(options);
    let display_url_in_dagger = options.title.is_some() || options.body.is_some();
    let output = if display_url_in_dagger {
        run_gh_capture_output(&args)?
    } else {
        run_gh_with_live_output(&args)?
    };
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
        return Ok(CreatedPullRequest {
            number,
            url,
            display_url_in_dagger,
        });
    }

    let mut created_pull_request = view_pull_request_by_url(&url)?;
    created_pull_request.display_url_in_dagger = display_url_in_dagger;
    Ok(created_pull_request)
}

pub fn view_pull_request(number: u64) -> io::Result<PullRequestStatus> {
    let output = run_gh_capture_output(&[
        "pr".to_string(),
        "view".to_string(),
        number.to_string(),
        "--json".to_string(),
        "number,state,mergedAt,baseRefName,headRefName,headRefOid,isDraft,url".to_string(),
    ])?;

    parse_pull_request_status(&output.stdout)
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

pub fn reopen_pull_request(number: u64) -> io::Result<()> {
    run_gh_command(
        "gh pr reopen",
        &["pr".to_string(), "reopen".to_string(), number.to_string()],
    )
}

pub fn mark_pull_request_as_draft(number: u64) -> io::Result<()> {
    run_gh_command(
        "gh pr ready --undo",
        &[
            "pr".to_string(),
            "ready".to_string(),
            number.to_string(),
            "--undo".to_string(),
        ],
    )
}

pub fn retarget_pull_request_base(number: u64, base_branch_name: &str) -> io::Result<()> {
    run_gh_command(
        "gh pr edit --base",
        &[
            "pr".to_string(),
            "edit".to_string(),
            number.to_string(),
            "--base".to_string(),
            base_branch_name.to_string(),
        ],
    )
}

pub fn edit_pull_request_base(number: u64, base_branch_name: &str) -> io::Result<()> {
    retarget_pull_request_base(number, base_branch_name)
}

pub fn merge_pull_request(number: u64) -> io::Result<()> {
    run_gh_command(
        "gh pr merge",
        &[
            "pr".to_string(),
            "merge".to_string(),
            number.to_string(),
            "--squash".to_string(),
            "--delete-branch".to_string(),
        ],
    )
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
        display_url_in_dagger: false,
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

fn parse_pull_request_status(stdout: &str) -> io::Result<PullRequestStatus> {
    let record: PullRequestStatusRecord = serde_json::from_str(stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(PullRequestStatus {
        number: record.number,
        state: record.state,
        merged_at: record.merged_at,
        base_ref_name: record.base_ref_name,
        head_ref_name: record.head_ref_name,
        head_ref_oid: record.head_ref_oid,
        is_draft: record.is_draft,
        url: record.url,
    })
}

fn build_create_pull_request_args(options: &CreatePullRequestOptions) -> Vec<String> {
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

    args
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

/// Returns the program name used to invoke the GitHub CLI.
///
/// Defaults to `"gh"` but can be overridden via the `DAGGER_GH_BIN` environment
/// variable, which is useful for testing on platforms where `Command::new("gh")`
/// does not resolve non-`.exe` scripts (e.g. `.cmd` wrappers on Windows).
fn gh_program() -> String {
    env::var("DAGGER_GH_BIN").unwrap_or_else(|_| "gh".to_string())
}

fn run_gh_capture_output(args: &[String]) -> io::Result<GhCommandOutput> {
    let output = Command::new(gh_program())
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
    let mut child = Command::new(gh_program())
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
    let combined = combine_command_output(stdout, stderr);

    if looks_like_auth_error(&combined) {
        return io::Error::other("gh authentication failed; run 'gh auth login'");
    }

    let sanitized = sanitize_gh_failure_output(&combined);
    if sanitized.is_empty() {
        io::Error::other(format!("{command_name} failed"))
    } else {
        io::Error::other(format!("{command_name} failed: {sanitized}"))
    }
}

fn combine_command_output(stdout: &str, stderr: &str) -> String {
    let stdout = stdout.trim();
    let stderr = stderr.trim();

    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

fn sanitize_gh_failure_output(output: &str) -> String {
    let mut lines = Vec::new();

    for line in output.lines() {
        if line.trim_start().starts_with("Usage:") {
            break;
        }
        lines.push(line);
    }

    lines.join("\n").trim().to_string()
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
        CreatePullRequestOptions, PullRequestState, build_create_pull_request_args,
        find_pull_request_url, parse_open_pull_request_details, parse_open_pull_requests,
        parse_pull_request_status, pull_request_number_from_url, sanitize_gh_failure_output,
    };

    #[test]
    fn parses_open_pull_request_list_output() {
        let pull_requests = parse_open_pull_requests(
            r#"[{"number":123,"baseRefName":"main","url":"https://github.com/oneirosoft/dagger/pull/123"}]"#,
        )
        .unwrap();

        assert_eq!(pull_requests.len(), 1);
        assert_eq!(pull_requests[0].number, 123);
        assert_eq!(pull_requests[0].base_ref_name, "main");
    }

    #[test]
    fn parses_open_pull_request_details_output() {
        let pull_requests = parse_open_pull_request_details(
            r#"[{"number":123,"title":"Auth PR","url":"https://github.com/oneirosoft/dagger/pull/123"}]"#,
        )
        .unwrap();

        assert_eq!(pull_requests[0].title, "Auth PR");
    }

    #[test]
    fn parses_pull_request_status_output() {
        let pull_request = parse_pull_request_status(
            r#"{"number":123,"state":"CLOSED","mergedAt":null,"baseRefName":"main","headRefName":"feat/auth","isDraft":false,"url":"https://github.com/oneirosoft/dagger/pull/123"}"#,
        )
        .unwrap();

        assert_eq!(pull_request.number, 123);
        assert_eq!(pull_request.state, PullRequestState::Closed);
        assert_eq!(pull_request.base_ref_name, "main");
        assert_eq!(pull_request.head_ref_name, "feat/auth");
        assert_eq!(pull_request.head_ref_oid, None);
        assert!(!pull_request.is_draft);
        assert_eq!(pull_request.merged_at, None);
    }

    #[test]
    fn extracts_pull_request_url_and_number_from_create_output() {
        let url = find_pull_request_url(
            "Creating pull request for feat/auth into main in oneirosoft/dagger.\nhttps://github.com/oneirosoft/dagger/pull/456\n",
        )
        .unwrap();

        assert_eq!(url, "https://github.com/oneirosoft/dagger/pull/456");
        assert_eq!(pull_request_number_from_url(&url), Some(456));
    }

    #[test]
    fn ignores_non_pull_request_urls_when_extracting_number() {
        assert_eq!(
            pull_request_number_from_url("https://github.com/oneirosoft/dagger/issues/456"),
            None
        );
    }

    #[test]
    fn builds_create_pull_request_args_from_options() {
        let args = build_create_pull_request_args(&CreatePullRequestOptions {
            base_branch_name: "main".into(),
            title: Some("feat: auth".into()),
            body: Some("feat: auth".into()),
            draft: true,
        });

        assert_eq!(
            args,
            vec![
                "pr",
                "create",
                "--base",
                "main",
                "--title",
                "feat: auth",
                "--body",
                "feat: auth",
                "--draft",
            ]
        );
    }

    #[test]
    fn strips_usage_block_from_gh_failure_output() {
        let sanitized = sanitize_gh_failure_output(
            "must provide `--title` and `--body`\n\nUsage:  gh pr create [flags]\n\nFlags:\n  -b, --body string\n",
        );

        assert_eq!(sanitized, "must provide `--title` and `--body`");
    }
}
