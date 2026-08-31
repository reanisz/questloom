//! AI CLI プロセスの起動・出力取り込み・タイムアウト・キャンセル。
//!
//! Windows では以下に注意している。
//!
//! - `claude` / `codex` は npm が生成する `.cmd` シム経由で入っていることがあり、
//!   `CreateProcessW` は `.exe` しか探さない。そのため PATH と `PATHEXT` を自前で
//!   走査して実体を解決し、`.cmd` / `.bat` だった場合は `cmd.exe /C` 経由で起動する。
//! - シム経由の引数は cmd.exe とバッチの `%*` で 2 度パースされる。引用符を `""`
//!   で表す([`quote_for_cmd`])ことで引用符の対応を崩さず、`&` `|` `>` などを
//!   引用符の内側に閉じ込める。
//! - ただし **改行を含む引数は cmd.exe を通せない**(改行が行末とみなされる)。
//!   複数行のプロンプトはシム経由のとき標準入力から渡す([`PromptDelivery`])。
//! - `CREATE_NO_WINDOW` を付け、コンソールウィンドウを出さない。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::{AiError, AiResult};

/// コンソールウィンドウを出さずにプロセスを起動する Windows のフラグ。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `stderr` をエラーメッセージへ載せるときの最大文字数。
const STDERR_LIMIT: usize = 400;

/// プロンプトを CLI へ渡す方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptDelivery {
    /// 引数として渡す(`{prompt}` を置換)。既定。
    #[default]
    Argument,
    /// 標準入力から渡し、`{prompt}` を含む引数は落とす。
    ///
    /// cmd.exe 経由で起動せざるを得ない `.cmd` シムでは、改行を含む引数を
    /// 渡せないためこちらを使う。
    Stdin,
}

impl PromptDelivery {
    /// コマンド名から、使える渡し方を判定する。
    ///
    /// 解決できないコマンドは [`Self::Argument`](PromptDelivery::Argument) を返す
    /// (どのみち起動時に [`AiError::CommandNotFound`] になる)。
    #[must_use]
    pub fn detect(command: &str) -> Self {
        match resolve_program(command) {
            Some(path) if needs_shell(&path) => Self::Stdin,
            _ => Self::Argument,
        }
    }
}

/// 1 回の CLI 実行の指定。
#[derive(Debug, Clone)]
pub struct AiRequest {
    /// 実行ファイル名(PATH から解決する)。
    pub command: String,
    /// 引数(プレースホルダ置換済み)。シェルを介さず配列のまま渡す。
    pub args: Vec<String>,
    /// 標準入力へ流す内容。`None` なら標準入力は閉じる。
    pub stdin: Option<String>,
    /// これを超えたらプロセスを kill する。
    pub timeout: Duration,
}

/// CLI の実行結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiOutput {
    /// 終了コード。シグナル等で取得できない場合は `None`。
    pub code: Option<i32>,
    /// 標準出力(UTF-8 として lossy に解釈)。
    pub stdout: String,
    /// 標準エラー出力(UTF-8 として lossy に解釈)。
    pub stderr: String,
}

impl AiOutput {
    /// 正常終了(終了コード 0)か。
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// 正常終了なら標準出力を返し、そうでなければ [`AiError::Failed`] にする。
    ///
    /// # Errors
    /// 終了コードが 0 以外の場合。
    pub fn into_stdout(self, command: &str) -> AiResult<String> {
        if self.success() {
            return Ok(self.stdout);
        }
        Err(AiError::Failed {
            command: command.to_owned(),
            code: self
                .code
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
            stderr: truncate(self.stderr.trim(), STDERR_LIMIT),
        })
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars()
        .take(limit)
        .chain(std::iter::once('…'))
        .collect()
}

/// CLI を起動し、終了まで待って出力を返す。
///
/// `cancel` が発火するか `request.timeout` を超えた場合はプロセスを kill し、
/// [`AiError::Cancelled`] / [`AiError::Timeout`] を返す。
///
/// # Errors
/// 実行ファイルが見つからない・起動に失敗した・タイムアウト・キャンセルの各場合。
pub async fn run(request: &AiRequest, cancel: &CancellationToken) -> AiResult<AiOutput> {
    let mut command = build_command(&request.command, &request.args)?;
    command
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    tracing::info!(
        command = request.command,
        timeout_secs = request.timeout.as_secs(),
        stdin = request.stdin.is_some(),
        "AI CLI を起動します"
    );

    let mut child = command.spawn().map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            AiError::CommandNotFound {
                command: request.command.clone(),
            }
        } else {
            AiError::Spawn {
                command: request.command.clone(),
                source,
            }
        }
    })?;

    // パイプが詰まって子プロセスがブロックしないよう、待機と並行して読み書きする。
    if let (Some(mut pipe), Some(text)) = (child.stdin.take(), request.stdin.clone()) {
        tokio::spawn(async move {
            let _ = pipe.write_all(text.as_bytes()).await;
            // ここで drop して EOF を伝える(閉じないと CLI が入力待ちのまま止まる)。
            let _ = pipe.shutdown().await;
        });
    }
    let mut stdout = child.stdout.take().expect("stdout は piped");
    let mut stderr = child.stderr.take().expect("stderr は piped");
    let stdout_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer).await;
        buffer
    });
    let stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer).await;
        buffer
    });

    let status = tokio::select! {
        result = child.wait() => result.map_err(|source| AiError::Io {
            command: request.command.clone(),
            source,
        })?,
        () = tokio::time::sleep(request.timeout) => {
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(AiError::Timeout {
                command: request.command.clone(),
                secs: request.timeout.as_secs(),
            });
        }
        () = cancel.cancelled() => {
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(AiError::Cancelled);
        }
    };

    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    Ok(AiOutput {
        code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// 実体を解決したうえで [`Command`] を組み立てる。
fn build_command(program: &str, args: &[String]) -> AiResult<Command> {
    let resolved = resolve_program(program).ok_or_else(|| AiError::CommandNotFound {
        command: program.to_owned(),
    })?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;

        let mut command = if needs_shell(&resolved) {
            if args.iter().any(|arg| arg.contains(['\n', '\r', '\0'])) {
                return Err(AiError::ShimArgument {
                    command: program.to_owned(),
                });
            }
            // `.cmd` / `.bat` は実行イメージではないので cmd.exe に解釈させる。
            let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned());
            let mut command = Command::new(comspec);
            // /D: AutoRun を無効化(レジストリ設定に左右されない)
            // /S: 直後の最初と最後の `"` だけを剥がし、残りをそのままコマンドとして扱う
            command.args(["/D", "/S", "/C"]);
            command
                .as_std_mut()
                .raw_arg(command_line_for_shim(&resolved, args));
            command
        } else {
            let mut command = Command::new(&resolved);
            command.args(args);
            command
        };
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
        Ok(command)
    }

    #[cfg(not(windows))]
    {
        let mut command = Command::new(&resolved);
        command.args(args);
        Ok(command)
    }
}

/// cmd.exe 経由での起動が必要か(`.exe` / `.com` 以外)。
fn needs_shell(path: &Path) -> bool {
    if !cfg!(windows) {
        return false;
    }
    !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com"))
}

/// `cmd.exe /D /S /C` の後ろに verbatim で渡す文字列を組み立てる。
///
/// 全体をさらに `"` で囲むのは `/S` の規則に合わせるため。`/S` は「最初と最後の
/// `"` だけを取り除いて残りをそのままコマンドとして扱う」ので、内側のトークンごとの
/// 引用がそのまま生きる(囲まないと cmd の引用符除去で先頭トークンが壊れる)。
#[must_use]
pub fn command_line_for_shim(program: &Path, args: &[String]) -> String {
    let mut line = String::from("\"");
    line.push_str(&quote_for_cmd(&program.display().to_string()));
    for arg in args {
        line.push(' ');
        line.push_str(&quote_for_cmd(arg));
    }
    line.push('"');
    line
}

/// 1 トークンを `"` で括る。内側の `"` は `""`、`"` の直前のバックスラッシュは倍加。
///
/// - `""` は MSVCRT / `CommandLineToArgvW` ではリテラルの `"`(引用符の内側のまま)
/// - cmd.exe から見ても引用符の数が偶数のままなので、`&` `|` `>` などが
///   引用符の外に出てしまうことがない
///
/// バッチシムが `%*` で引数列を再展開しても、この形なら意味が変わらない。
#[must_use]
pub fn quote_for_cmd(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
                quoted.push_str("\"\"");
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// PATH(と Windows では `PATHEXT`)から実行ファイルの実体を探す。
///
/// パス区切りを含む場合はそのまま存在確認する。
#[must_use]
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    if program.contains('/') || program.contains('\\') {
        return with_extensions(Path::new(program));
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| with_extensions(&dir.join(program)))
}

/// 拡張子候補を試して、実在するファイルを返す。
fn with_extensions(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    #[cfg(windows)]
    {
        // 既定値は cmd.exe が使うものと同じ。`.ps1` は含まれない(cmd から直接
        // 起動できないため、PowerShell 専用シムしか無い CLI は未対応)。
        let pathext = std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .to_ascii_lowercase();
        for ext in pathext.split(';').filter(|ext| !ext.is_empty()) {
            let candidate = PathBuf::from(format!("{}{ext}", base.display()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用に、確実に存在する「何かを出力して即終了する」リクエストを作る。
    ///
    /// cmd の `echo` はコンソールのコードページで出力するため、ここでは ASCII を使う
    /// (実際の CLI は Node 製で、パイプへは UTF-8 を書く)。
    fn echo(text: &str) -> AiRequest {
        #[cfg(windows)]
        let (command, args) = (
            "cmd".to_owned(),
            vec!["/C".to_owned(), "echo".to_owned(), text.to_owned()],
        );
        #[cfg(not(windows))]
        let (command, args) = ("echo".to_owned(), vec![text.to_owned()]);
        AiRequest {
            command,
            args,
            stdin: None,
            timeout: Duration::from_secs(30),
        }
    }

    /// 標準入力をそのまま標準出力へ流すリクエスト。
    fn cat() -> AiRequest {
        #[cfg(windows)]
        let (command, args) = ("cmd".to_owned(), vec!["/C".to_owned(), "more".to_owned()]);
        #[cfg(not(windows))]
        let (command, args) = ("cat".to_owned(), Vec::new());
        AiRequest {
            command,
            args,
            stdin: None,
            timeout: Duration::from_secs(30),
        }
    }

    #[tokio::test]
    async fn runs_a_command_and_captures_stdout() {
        let output = run(&echo("hello questloom"), &CancellationToken::new())
            .await
            .unwrap();
        assert!(output.success());
        assert!(output.stdout.contains("hello questloom"), "{output:?}");
        assert_eq!(output.clone().into_stdout("cmd").unwrap(), output.stdout);
    }

    #[tokio::test]
    async fn feeds_stdin_and_closes_it() {
        let request = AiRequest {
            stdin: Some("first line\nsecond line\n".to_owned()),
            ..cat()
        };
        let output = run(&request, &CancellationToken::new()).await.unwrap();
        assert!(output.success(), "{output:?}");
        assert!(output.stdout.contains("second line"), "{output:?}");
    }

    #[tokio::test]
    async fn missing_commands_report_a_helpful_error() {
        let request = AiRequest {
            command: "questloom-nonexistent-cli".to_owned(),
            args: Vec::new(),
            stdin: None,
            timeout: Duration::from_secs(5),
        };
        let error = run(&request, &CancellationToken::new()).await.unwrap_err();
        assert!(matches!(error, AiError::CommandNotFound { .. }));
        let message = error.to_string();
        assert!(message.contains("見つかりません"), "{message}");
        assert!(message.contains("PATH"), "{message}");
    }

    /// 2 秒以上かかるコマンドを作る。
    fn slow(timeout: Duration) -> AiRequest {
        #[cfg(windows)]
        let (command, args) = (
            "cmd".to_owned(),
            vec!["/C".to_owned(), "ping -n 6 127.0.0.1 > NUL".to_owned()],
        );
        #[cfg(not(windows))]
        let (command, args) = ("sleep".to_owned(), vec!["5".to_owned()]);
        AiRequest {
            command,
            args,
            stdin: None,
            timeout,
        }
    }

    #[tokio::test]
    async fn a_timeout_kills_the_process() {
        let error = run(&slow(Duration::from_millis(300)), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(error, AiError::Timeout { .. }), "{error}");
        assert!(error.to_string().contains("中断"));
    }

    #[tokio::test]
    async fn cancellation_stops_the_run() {
        let cancel = CancellationToken::new();
        let handle = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            handle.cancel();
        });
        let error = run(&slow(Duration::from_secs(30)), &cancel)
            .await
            .unwrap_err();
        assert!(matches!(error, AiError::Cancelled), "{error}");
    }

    #[tokio::test]
    async fn a_non_zero_exit_becomes_an_error() {
        #[cfg(windows)]
        let (command, args) = ("cmd".to_owned(), vec!["/C".to_owned(), "exit 3".to_owned()]);
        #[cfg(not(windows))]
        let (command, args) = ("sh".to_owned(), vec!["-c".to_owned(), "exit 3".to_owned()]);
        let request = AiRequest {
            command,
            args,
            stdin: None,
            timeout: Duration::from_secs(10),
        };
        let output = run(&request, &CancellationToken::new()).await.unwrap();
        assert_eq!(output.code, Some(3));
        let error = output.into_stdout("cmd").unwrap_err();
        assert!(matches!(error, AiError::Failed { .. }));
        assert!(error.to_string().contains("3"));
    }

    #[test]
    fn resolves_commands_from_path() {
        #[cfg(windows)]
        let name = "cmd";
        #[cfg(not(windows))]
        let name = "sh";
        let resolved = resolve_program(name).expect("PATH から解決できる");
        assert!(resolved.is_file(), "{resolved:?}");
        assert_eq!(PromptDelivery::detect(name), PromptDelivery::Argument);

        assert!(resolve_program("questloom-nonexistent-cli").is_none());
        assert_eq!(
            PromptDelivery::detect("questloom-nonexistent-cli"),
            PromptDelivery::Argument
        );
    }

    #[cfg(windows)]
    #[test]
    fn resolves_cmd_shims_through_pathext_and_switches_to_stdin() {
        // npm が作る `.cmd` シムを模したファイルを、拡張子なしの名前で解決させる。
        // (PATH を書き換えると他のテストに影響するため、パス指定で確認する。)
        let dir = std::env::temp_dir().join(format!("questloom-ai-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("questloom-fake-cli.cmd");
        std::fs::write(&shim, "@echo off\r\necho %1\r\n").unwrap();

        let base = dir.join("questloom-fake-cli").display().to_string();
        let resolved = resolve_program(&base).expect(".cmd シムを解決できる");
        let delivery = PromptDelivery::detect(&base);
        std::fs::remove_file(&shim).ok();

        assert_eq!(resolved, shim);
        assert!(needs_shell(&resolved), "cmd.exe 経由で起動する");
        assert!(!needs_shell(Path::new(r"C:\Windows\System32\cmd.exe")));
        // 改行を含むプロンプトを cmd 経由で渡せないので、標準入力に切り替える。
        assert_eq!(delivery, PromptDelivery::Stdin);
    }

    #[test]
    fn quoting_keeps_quotes_balanced_for_cmd() {
        // 引用符は `""`。cmd から見た引用符の数は常に偶数になる。
        assert_eq!(quote_for_cmd("plain"), r#""plain""#);
        assert_eq!(quote_for_cmd("a & b | c > d"), r#""a & b | c > d""#);
        assert_eq!(quote_for_cmd(r#"{"k":"v"}"#), r#""{""k"":""v""}""#);
        assert_eq!(quote_for_cmd(r"C:\path\"), r#""C:\path\\""#);
        assert_eq!(quote_for_cmd(r#"a\"b"#), r#""a\\""b""#);

        for arg in [
            "plain",
            "a & b | c > d",
            r#"{"k":"v"}"#,
            r"C:\path\",
            r#"a\"b"#,
        ] {
            let quoted = quote_for_cmd(arg);
            assert_eq!(
                quoted.matches('"').count() % 2,
                0,
                "引用符の数が奇数: {quoted}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn shim_command_line_joins_quoted_tokens() {
        let line = command_line_for_shim(
            Path::new(r"C:\npm\claude.cmd"),
            &["-p".to_owned(), "a & b".to_owned()],
        );
        // 外側の 1 対は /S が剥がす。
        assert_eq!(line, r#"""C:\npm\claude.cmd" "-p" "a & b"""#);
    }

    /// `.cmd` シムを実際に cmd.exe 経由で起動し、メタ文字入りの引数がそのまま届くか確認する。
    #[cfg(windows)]
    #[tokio::test]
    async fn runs_a_cmd_shim_with_tricky_arguments() {
        let dir = std::env::temp_dir().join(format!("questloom-ai-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("questloom-echo-shim.cmd");
        // npm のシムと同じく `%*` で引数列をそのまま転送する形を模す。
        // (`%*` は引用符付きのまま展開されるので、メタ文字が引用符の外に出ない。)
        std::fs::write(&shim, "@echo off\r\necho.%*\r\n").unwrap();

        let request = AiRequest {
            command: shim.display().to_string(),
            args: vec!["-p".to_owned(), "a & b | c > d".to_owned()],
            stdin: None,
            timeout: Duration::from_secs(20),
        };
        let output = run(&request, &CancellationToken::new()).await.unwrap();
        std::fs::remove_file(&shim).ok();

        // cmd.exe とバッチの再展開を通っても、引用済みトークンが崩れない。
        assert!(output.success(), "{output:?}");
        assert_eq!(
            output.stdout.trim(),
            r#""-p" "a & b | c > d""#,
            "{output:?}"
        );
    }

    /// シム経由では改行を含む引数を渡せないので、はっきりエラーにする。
    #[cfg(windows)]
    #[tokio::test]
    async fn a_cmd_shim_rejects_multiline_arguments() {
        let dir = std::env::temp_dir().join(format!("questloom-ai-nl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("questloom-nl-shim.cmd");
        std::fs::write(&shim, "@echo off\r\necho.%~1\r\n").unwrap();

        let request = AiRequest {
            command: shim.display().to_string(),
            args: vec!["1 行目\n2 行目".to_owned()],
            stdin: None,
            timeout: Duration::from_secs(20),
        };
        let error = run(&request, &CancellationToken::new()).await.unwrap_err();
        std::fs::remove_file(&shim).ok();
        assert!(matches!(error, AiError::ShimArgument { .. }), "{error}");
    }
}
