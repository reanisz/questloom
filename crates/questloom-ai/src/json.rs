//! CLI の応答テキストから JSON を頑健に取り出すヘルパ。
//!
//! 「JSON のみを出力せよ」と指示しても、CLI は前後に説明文を付けたり
//! ```` ```json ```` のコードフェンスで包んだりする。ここでは
//! **最初にパースできた JSON 値** を採用することで、そうした揺れを吸収する。

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{AiError, AiResult};

/// テキスト中から最初の JSON 値(オブジェクトまたは配列)を取り出す。
///
/// 走査は `{` / `[` の出現位置ごとに行い、そこから 1 つの JSON 値として
/// パースできたら採用する(後続のゴミは無視する)。これにより
/// 前後の説明文・コードフェンスは自然に剥がれる。
///
/// # Errors
/// パースできる JSON 値が 1 つも見つからない場合。
pub fn extract_first_json(text: &str) -> AiResult<Value> {
    for (index, ch) in text.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }
        // StreamDeserializer は「1 値だけ読んで残りを無視する」ため、
        // 末尾の説明文やコードフェンスの閉じがあっても成立する。
        let mut stream = serde_json::Deserializer::from_str(&text[index..]).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next() {
            return Ok(value);
        }
    }
    Err(AiError::NoJson {
        snippet: snippet(text),
    })
}

/// テキスト中の最初の JSON 値を、指定の型へデシリアライズする。
///
/// # Errors
/// JSON が見つからない、または型が合わない場合。
pub fn parse_first_json<T: DeserializeOwned>(text: &str) -> AiResult<T> {
    let value = extract_first_json(text)?;
    serde_json::from_value(value).map_err(|source| AiError::Json {
        message: source.to_string(),
        snippet: snippet(text),
    })
}

/// エラーメッセージに載せる応答の抜粋(長い応答は切り詰める)。
pub(crate) fn snippet(text: &str) -> String {
    const LIMIT: usize = 300;
    let trimmed = text.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(LIMIT).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json_array() {
        let value = extract_first_json(r#"[{"title":"買い物"}]"#).unwrap();
        assert_eq!(value[0]["title"], "買い物");
    }

    #[test]
    fn strips_prose_and_code_fences() {
        let text = "了解しました。以下が結果です。\n\n```json\n[\n  {\"title\": \"A\"}\n]\n```\n\n以上です。";
        let value = extract_first_json(text).unwrap();
        assert!(value.is_array());
        assert_eq!(value[0]["title"], "A");
    }

    #[test]
    fn skips_braces_that_are_not_json() {
        // 説明文に紛れた `{...}` は JSON としてパースできないので読み飛ばす。
        let text = "形式は {title, description} です。\n{\"title\":\"本命\"}";
        let value = extract_first_json(text).unwrap();
        assert_eq!(value["title"], "本命");
    }

    #[test]
    fn handles_nested_structures_and_escaped_braces_in_strings() {
        let text = r#"出力:
```
{"tasks":[{"title":"a {b}","description":"改行\nあり","nested":{"k":[1,2]}}]}
```"#;
        let value = extract_first_json(text).unwrap();
        assert_eq!(value["tasks"][0]["title"], "a {b}");
        assert_eq!(value["tasks"][0]["nested"]["k"][1], 2);
    }

    #[test]
    fn multibyte_prefix_does_not_break_slicing() {
        let text = "はい、こちらです → [1, 2, 3]";
        let value = extract_first_json(text).unwrap();
        assert_eq!(value[2], 3);
    }

    #[test]
    fn reports_missing_json() {
        let error = extract_first_json("JSON はありません").unwrap_err();
        assert!(matches!(error, AiError::NoJson { .. }));
        assert!(error.to_string().contains("JSON"));
    }

    #[test]
    fn parse_first_json_maps_to_typed_values() {
        #[derive(Debug, serde::Deserialize)]
        struct Item {
            title: String,
        }
        // 閉じていない JSON は「見つからなかった」扱い。
        assert!(parse_first_json::<Vec<Item>>("ここから [").is_err());

        let items: Vec<Item> = parse_first_json(r#"```json[{"title":"x"}]```"#).unwrap();
        assert_eq!(items[0].title, "x");

        // 型が合わない場合はエラー。
        let error = parse_first_json::<Vec<Item>>(r#"{"title":"x"}"#).unwrap_err();
        assert!(matches!(error, AiError::Json { .. }));
    }

    #[test]
    fn snippet_truncates_long_text() {
        let long = "あ".repeat(400);
        let cut = snippet(&long);
        assert!(cut.ends_with('…'));
        assert_eq!(cut.chars().count(), 301);
    }
}
