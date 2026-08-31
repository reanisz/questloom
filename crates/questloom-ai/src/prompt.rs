//! 3 つの AI 機能のプロンプト設計と、応答 JSON の解釈。
//!
//! - [`create_tasks_prompt`] — 文章からタスクを抽出させる(構造化出力)
//! - [`split_task_prompt`] — タスクをサブタスクへ分割・詳細化させる(構造化出力)
//! - [`free_instruction_prompt`] — MCP ツール経由で自律的に操作させる(自然文の応答)
//!
//! 構造化出力の 2 つは、応答から [`parse_task_drafts`] で JSON 配列を取り出す。

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use questloom_core::service::TaskDetail;
use serde::{Deserialize, Serialize};

use crate::error::{AiError, AiResult};
use crate::json::parse_first_json;

/// 履歴が長いタスクでプロンプトが膨らみすぎないようにする上限。
const MAX_UPDATES_IN_PROMPT: usize = 10;

/// AI が提案した 1 件のタスク。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskDraft {
    /// タイトル(必須)。
    pub title: String,
    /// 詳細。
    pub description: String,
    /// 締切。RFC 3339 か `YYYY-MM-DD`。解釈できない場合は無視する。
    pub deadline: Option<String>,
}

impl TaskDraft {
    /// 締切を UTC の時刻へ解釈する。
    ///
    /// `YYYY-MM-DD` だけの場合はその日の終わり(23:59:59 UTC)とみなす。
    /// 解釈できない文字列は `None`(締切なし)として扱う。
    #[must_use]
    pub fn deadline_utc(&self) -> Option<DateTime<Utc>> {
        let raw = self.deadline.as_deref()?.trim();
        if raw.is_empty() {
            return None;
        }
        if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
            return Some(parsed.with_timezone(&Utc));
        }
        let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
        let end_of_day = date.and_hms_opt(23, 59, 59)?;
        Utc.from_local_datetime(&end_of_day).single()
    }
}

/// 応答 JSON からタスク案の配列を取り出す。
///
/// 配列そのものに加え、`{"tasks":[...]}` のようなラップも許容する。
/// タイトルが空の要素は捨てる。
///
/// # Errors
/// JSON が見つからない・解釈できない・有効な要素が 0 件の場合。
pub fn parse_task_drafts(text: &str) -> AiResult<Vec<TaskDraft>> {
    /// 配列でもオブジェクトラップでも受けるための入れ物。
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Drafts {
        List(Vec<TaskDraft>),
        Wrapped {
            #[serde(alias = "subtasks", alias = "items", alias = "results")]
            tasks: Vec<TaskDraft>,
        },
    }

    let drafts: Vec<TaskDraft> = match parse_first_json::<Drafts>(text)? {
        Drafts::List(list) => list,
        Drafts::Wrapped { tasks } => tasks,
    };
    let drafts: Vec<TaskDraft> = drafts
        .into_iter()
        .map(|draft| TaskDraft {
            title: draft.title.trim().to_owned(),
            description: draft.description.trim().to_owned(),
            deadline: draft.deadline,
        })
        .filter(|draft| !draft.title.is_empty())
        .collect();

    if drafts.is_empty() {
        return Err(AiError::NoTasks);
    }
    Ok(drafts)
}

/// 構造化出力を求める共通の指示。
fn json_contract(today: NaiveDate) -> String {
    format!(
        "# 出力形式\n\
         JSON 配列だけを出力してください。前置き・説明・コードフェンスは不要です。\n\
         各要素は次の形にしてください。\n\
         {{\"title\": \"タスクのタイトル\", \"description\": \"補足(不要なら空文字)\", \"deadline\": \"YYYY-MM-DD または RFC3339(不明なら null)\"}}\n\
         \n\
         # 制約\n\
         - title は必須。40 文字以内で、何をするかが分かる短い日本語にすること。\n\
         - 入力に書かれていない締切を推測しないこと。日付が読み取れないときは deadline を null にすること。\n\
         - 該当するものが無ければ空配列 [] を出力すること。\n\
         - 今日は {today} です。\n"
    )
}

/// 文章からタスクを抽出させるプロンプト。
#[must_use]
pub fn create_tasks_prompt(text: &str, today: NaiveDate) -> String {
    format!(
        "あなたはタスク管理アプリ questloom のアシスタントです。\n\
         次の文章から、実行すべきタスクを抽出してください。\n\
         \n\
         {contract}\n\
         # 対象の文章\n\
         <<<\n{text}\n>>>\n",
        contract = json_contract(today),
        text = text.trim(),
    )
}

/// タスクをサブタスクへ分割・詳細化させるプロンプト。
#[must_use]
pub fn split_task_prompt(
    detail: &TaskDetail,
    instruction: Option<&str>,
    today: NaiveDate,
) -> String {
    let mut prompt = format!(
        "あなたはタスク管理アプリ questloom のアシスタントです。\n\
         次のタスクを、実行可能な粒度のサブタスクへ分割・詳細化してください。\n\
         既にある子タスクと重複するものは出さないでください。\n\
         \n\
         {contract}",
        contract = json_contract(today),
    );

    if let Some(instruction) = instruction.map(str::trim).filter(|text| !text.is_empty()) {
        prompt.push_str(&format!("\n# 追加指示\n{instruction}\n"));
    }

    prompt.push_str(&format!(
        "\n# 対象のタスク\n\
         タイトル: {title}\n\
         状態: {status}\n\
         締切: {deadline}\n",
        title = detail.card.task.title,
        status = detail.card.task.status,
        deadline = detail
            .card
            .task
            .deadline
            .map_or_else(|| "なし".to_owned(), |at| at.to_rfc3339()),
    ));

    let description = detail.card.task.description.trim();
    if !description.is_empty() {
        prompt.push_str(&format!("詳細:\n<<<\n{description}\n>>>\n"));
    }

    if !detail.children.is_empty() {
        prompt.push_str("既存の子タスク:\n");
        for child in &detail.children {
            prompt.push_str(&format!("- {}\n", child.task.title));
        }
    }

    if !detail.updates.is_empty() {
        prompt.push_str("これまでのアップデート履歴(新しいものが下):\n");
        let skip = detail.updates.len().saturating_sub(MAX_UPDATES_IN_PROMPT);
        for update in detail.updates.iter().skip(skip) {
            prompt.push_str(&format!(
                "- [{}] {}\n",
                update.created_at.format("%Y-%m-%d"),
                update.body.replace('\n', " ")
            ));
        }
    }

    prompt
}

/// 自由指示のプロンプト。
///
/// `mcp_attached` が真なら、MCP ツール経由でタスクを操作するよう前置きする。
/// 偽の場合は MCP が使えない旨を伝え、テキストで答えさせる。
#[must_use]
pub fn free_instruction_prompt(text: &str, mcp_attached: bool) -> String {
    let preamble = if mcp_attached {
        "あなたはタスク管理アプリ questloom のアシスタントです。\n\
         questloom の MCP ツール(`mcp__questloom__*`)からタスクを直接操作できます。\n\
         list_tasks / get_task / create_task / update_task / move_task / complete_task /\n\
         promote_task / add_task_update / add_resource が使えます。\n\
         まず必要な情報を list_tasks などで確認し、そのうえで指示を実行してください。\n\
         削除に相当する操作はありません。取り消せない変更は最小限にとどめてください。\n\
         最後に、何をしたかを日本語 3 行以内で報告してください。"
    } else {
        "あなたはタスク管理アプリ questloom のアシスタントです。\n\
         いまは questloom のタスクを直接操作できません(MCP 未接続)。\n\
         指示に対して、日本語で簡潔に回答してください。"
    };
    format!(
        "{preamble}\n\n# ユーザーの指示\n<<<\n{}\n>>>\n",
        text.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::model::{
        Origin, Scheduled, Task, TaskId, TaskStatus, TaskUpdateEntry, UpdateId,
    };
    use questloom_core::service::TaskCard;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 2).unwrap()
    }

    fn card(title: &str, description: &str) -> TaskCard {
        let now = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
        TaskCard {
            task: Task {
                id: TaskId::new(),
                title: title.to_owned(),
                description: description.to_owned(),
                status: TaskStatus::New,
                scheduled: Scheduled::None,
                deadline: None,
                is_instant: false,
                origin: Origin::User,
                parent_id: None,
                sort_order: "a0".to_owned(),
                created_at: now,
                updated_at: now,
                done_at: None,
                deleted_at: None,
            },
            bucket: None,
            child_count: 0,
            resource_count: 0,
            primary_resource: None,
        }
    }

    #[test]
    fn parses_a_plain_array() {
        let drafts = parse_task_drafts(
            r#"```json
            [
              {"title": " 見積もりを出す ", "description": " 概算で可 ", "deadline": "2026-09-30"},
              {"title": "レビュー依頼"}
            ]
            ```"#,
        )
        .unwrap();
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].title, "見積もりを出す");
        assert_eq!(drafts[0].description, "概算で可");
        assert_eq!(
            drafts[0].deadline_utc().unwrap().to_rfc3339(),
            "2026-09-30T23:59:59+00:00"
        );
        assert_eq!(drafts[1].description, "");
        assert_eq!(drafts[1].deadline_utc(), None);
    }

    #[test]
    fn parses_wrapped_objects() {
        let drafts = parse_task_drafts(r#"{"tasks":[{"title":"a"}]}"#).unwrap();
        assert_eq!(drafts[0].title, "a");
        let drafts = parse_task_drafts(r#"{"subtasks":[{"title":"b"}]}"#).unwrap();
        assert_eq!(drafts[0].title, "b");
    }

    #[test]
    fn rejects_empty_results() {
        assert!(matches!(parse_task_drafts("[]"), Err(AiError::NoTasks)));
        assert!(matches!(
            parse_task_drafts(r#"[{"title":"   "}]"#),
            Err(AiError::NoTasks)
        ));
        assert!(matches!(
            parse_task_drafts("タスクはありません"),
            Err(AiError::NoJson { .. })
        ));
    }

    #[test]
    fn accepts_rfc3339_deadlines_and_ignores_garbage() {
        let draft = TaskDraft {
            title: "x".to_owned(),
            description: String::new(),
            deadline: Some("2026-09-30T09:00:00Z".to_owned()),
        };
        assert_eq!(
            draft.deadline_utc().unwrap().to_rfc3339(),
            "2026-09-30T09:00:00+00:00"
        );
        let draft = TaskDraft {
            deadline: Some("来週まで".to_owned()),
            ..draft.clone()
        };
        assert_eq!(draft.deadline_utc(), None);
        let draft = TaskDraft {
            deadline: Some("  ".to_owned()),
            ..draft
        };
        assert_eq!(draft.deadline_utc(), None);
    }

    #[test]
    fn create_prompt_states_the_contract() {
        let prompt = create_tasks_prompt("  水曜までに請求書を出す  ", today());
        assert!(prompt.contains("JSON 配列だけを出力"));
        assert!(prompt.contains("今日は 2026-09-02 です"));
        assert!(prompt.contains("水曜までに請求書を出す"));
        assert!(!prompt.contains("  水曜"), "前後の空白は落とす");
    }

    #[test]
    fn split_prompt_includes_task_context() {
        let parent = card("引っ越し", "9 月中に完了させる");
        let parent_id = parent.task.id;
        let detail = TaskDetail {
            card: parent,
            resources: Vec::new(),
            updates: vec![TaskUpdateEntry {
                id: UpdateId::new(),
                task_id: parent_id,
                body: "業者を 3 社\nリストアップ".to_owned(),
                origin: Origin::User,
                created_at: Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap(),
            }],
            parent: None,
            children: vec![card("見積もりを取る", "")],
        };

        let prompt = split_task_prompt(&detail, Some(" 3 つ以内で "), today());
        assert!(prompt.contains("引っ越し"));
        assert!(prompt.contains("9 月中に完了させる"));
        assert!(prompt.contains("既存の子タスク"));
        assert!(prompt.contains("見積もりを取る"));
        // 履歴の改行は 1 行に潰す。
        assert!(prompt.contains("- [2026-09-01] 業者を 3 社 リストアップ"));
        assert!(prompt.contains("# 追加指示\n3 つ以内で"));
        assert!(prompt.contains("JSON 配列だけを出力"));

        // 追加指示なしでも節ごと出ない。
        let prompt = split_task_prompt(&detail, Some("   "), today());
        assert!(!prompt.contains("# 追加指示"));
    }

    #[test]
    fn free_instruction_prompt_switches_on_mcp() {
        let with_mcp = free_instruction_prompt("今日のタスクを整理して", true);
        assert!(with_mcp.contains("mcp__questloom__*"));
        assert!(with_mcp.contains("今日のタスクを整理して"));

        let without = free_instruction_prompt("今日のタスクを整理して", false);
        assert!(without.contains("MCP 未接続"));
        assert!(!without.contains("mcp__questloom__*"));
    }
}
