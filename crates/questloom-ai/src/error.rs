//! AI CLI 呼び出しのエラー。メッセージはそのまま UI に出せる日本語にする。

/// [`AiError`] を返す結果型。
pub type AiResult<T> = Result<T, AiError>;

/// AI CLI の呼び出し・解釈に失敗したことを表す。
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// 指定のプロバイダが設定に無い、または無効化されている。
    #[error("AI プロバイダ {id:?} が見つからないか無効になっています")]
    UnknownProvider {
        /// 要求されたプロバイダ ID。
        id: String,
    },
    /// 実行ファイルを PATH から解決できなかった。
    #[error("{command} CLI が見つかりません。インストールと PATH を確認してください")]
    CommandNotFound {
        /// 実行しようとしたコマンド名。
        command: String,
    },
    /// プロセスの起動に失敗した。
    #[error("{command} の起動に失敗しました: {source}")]
    Spawn {
        /// 実行しようとしたコマンド名。
        command: String,
        /// 元エラー。
        #[source]
        source: std::io::Error,
    },
    /// 起動後の入出力に失敗した。
    #[error("{command} の実行中にエラーが発生しました: {source}")]
    Io {
        /// 実行したコマンド名。
        command: String,
        /// 元エラー。
        #[source]
        source: std::io::Error,
    },
    /// `.cmd` シム経由では渡せない引数(改行を含むもの)があった。
    #[error("{command} は .cmd シム経由のため、改行を含む引数を渡せません")]
    ShimArgument {
        /// 実行しようとしたコマンド名。
        command: String,
    },
    /// タイムアウトしたためプロセスを kill した。
    #[error("{command} が {secs} 秒以内に終了しなかったため中断しました")]
    Timeout {
        /// 実行したコマンド名。
        command: String,
        /// タイムアウト秒数。
        secs: u64,
    },
    /// ユーザー操作で中断した。
    #[error("AI の実行をキャンセルしました")]
    Cancelled,
    /// 非 0 終了。
    #[error("{command} が異常終了しました (終了コード {code}): {stderr}")]
    Failed {
        /// 実行したコマンド名。
        command: String,
        /// 終了コード(シグナル等で不明なら `unknown`)。
        code: String,
        /// 標準エラー出力(切り詰め済み)。
        stderr: String,
    },
    /// 応答に JSON が含まれていなかった。
    #[error("AI の応答から JSON を取り出せませんでした: {snippet}")]
    NoJson {
        /// 応答の抜粋。
        snippet: String,
    },
    /// JSON の形が期待と違った。
    #[error("AI の応答 JSON を解釈できませんでした ({message}): {snippet}")]
    Json {
        /// serde のエラーメッセージ。
        message: String,
        /// 応答の抜粋。
        snippet: String,
    },
    /// 抽出できたタスクが 0 件だった。
    #[error("AI はタスクを 1 件も抽出しませんでした")]
    NoTasks,
}
