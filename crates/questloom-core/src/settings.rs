//! コア設定モデル。`settings` テーブルの `core` 名前空間に JSON で保存される。

use chrono::Weekday;
use serde::{Deserialize, Serialize};

/// 設定の名前空間名(コア設定)。
pub const CORE_NAMESPACE: &str = "core";

/// 週の開始曜日。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeekStart {
    /// 月曜始まり(既定)。ISO 8601 と一致する。
    #[default]
    Monday,
    /// 日曜始まり。
    Sunday,
}

impl WeekStart {
    /// [`chrono::Weekday`] へ変換する。
    #[must_use]
    pub const fn weekday(self) -> Weekday {
        match self {
            Self::Monday => Weekday::Mon,
            Self::Sunday => Weekday::Sun,
        }
    }
}

/// コア設定。未知フィールドは無視し、欠けたフィールドは既定値で補う(前方互換)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CoreSettings {
    /// 週の開始曜日。バケット導出に用いる。
    pub week_start: WeekStart,
    /// バックアップの保持世代数。
    pub backup_generations: u32,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            week_start: WeekStart::Monday,
            backup_generations: 14,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_docs() {
        let settings = CoreSettings::default();
        assert_eq!(settings.week_start, WeekStart::Monday);
        assert_eq!(settings.backup_generations, 14);
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let parsed: CoreSettings =
            serde_json::from_str(r#"{"weekStart":"sunday","futureField":123}"#).unwrap();
        assert_eq!(parsed.week_start, WeekStart::Sunday);
        assert_eq!(parsed.backup_generations, 14);
    }

    #[test]
    fn empty_object_yields_defaults() {
        let parsed: CoreSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, CoreSettings::default());
    }
}
