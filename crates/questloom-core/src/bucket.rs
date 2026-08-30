//! 時間バケットの導出。docs/data-model.md の規則をそのまま実装した純粋関数群。
//!
//! バケットは DB に保存せず、`scheduled_*` と「今日の日付」「週の開始曜日設定」から
//! 表示のたびに導出する。これにより日付が変わっても自動的に正しい列へ移動し、
//! アプリ停止中のロールオーバー処理が不要になる。

use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::model::{Scheduled, TaskStatus, WeekKey};
use crate::settings::WeekStart;

/// Todo タスクの表示バケット。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Bucket {
    /// 今日やる(過ぎた予定もここへ繰り上がる)。
    Today,
    /// 明日やる。
    Tomorrow,
    /// 今週中にやる。
    ThisWeek,
    /// 来週やる。
    NextWeek,
    /// それ以降 / いつか。
    Future,
}

/// ボードの列。ドラッグ&ドロップ先の指定に用いる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoardColumn {
    /// 受信箱。
    New,
    /// Todo / Today。
    Today,
    /// Todo / Tomorrow。
    Tomorrow,
    /// Todo / ThisWeek。
    ThisWeek,
    /// Todo / NextWeek。
    NextWeek,
    /// Todo / Future。
    Future,
    /// 着手中。
    Doing,
    /// 完了。
    Done,
}

impl BoardColumn {
    /// 対応する時間バケット。Todo 列以外は `None`。
    #[must_use]
    pub const fn bucket(self) -> Option<Bucket> {
        match self {
            Self::Today => Some(Bucket::Today),
            Self::Tomorrow => Some(Bucket::Tomorrow),
            Self::ThisWeek => Some(Bucket::ThisWeek),
            Self::NextWeek => Some(Bucket::NextWeek),
            Self::Future => Some(Bucket::Future),
            Self::New | Self::Doing | Self::Done => None,
        }
    }

    /// この列へドロップしたときの `(status, scheduled)` を求める。
    ///
    /// New / Doing / Done 列は予定を変更しない意味を持たせるため、
    /// 呼び出し側が保持したい既存の予定を `current` に渡す。
    #[must_use]
    pub fn resolve(
        self,
        current: Scheduled,
        today: NaiveDate,
        week_start: WeekStart,
    ) -> (TaskStatus, Scheduled) {
        match self.bucket() {
            Some(bucket) => (
                TaskStatus::Todo,
                scheduled_for_bucket(bucket, today, week_start),
            ),
            None => {
                let status = match self {
                    Self::New => TaskStatus::New,
                    Self::Doing => TaskStatus::Doing,
                    _ => TaskStatus::Done,
                };
                (status, current)
            }
        }
    }
}

/// `date` を含む週の開始日を返す。
#[must_use]
pub fn week_start_date(date: NaiveDate, week_start: WeekStart) -> NaiveDate {
    let offset = match week_start {
        WeekStart::Monday => date.weekday().num_days_from_monday(),
        WeekStart::Sunday => date.weekday().num_days_from_sunday(),
    };
    date - Days::new(u64::from(offset))
}

/// `date` を含む週の週キーを返す。
///
/// 週の 4 日目(月曜始まりなら木曜、日曜始まりなら水曜)が属する ISO 週を
/// ラベルとして採用する。月曜始まりでは ISO 8601 の週番号と完全に一致し、
/// どちらの設定でも `(year, week)` の順序は日付順と一致する。
#[must_use]
pub fn week_key_of(date: NaiveDate, week_start: WeekStart) -> WeekKey {
    let anchor = week_start_date(date, week_start) + Days::new(3);
    let iso = anchor.iso_week();
    WeekKey::new(iso.year(), iso.week())
}

/// docs/data-model.md のバケット導出規則。
///
/// - `date(d)`: `d <= today` → Today / `d == today+1` → Tomorrow /
///   今週内 → ThisWeek / 来週内 → NextWeek / それ以降 → Future
/// - `week(w)`: `w < 今週` → Today / `w == 今週` → ThisWeek /
///   `w == 来週` → NextWeek / それ以降 → Future
/// - `none` → Future
#[must_use]
pub fn derive_bucket(scheduled: &Scheduled, today: NaiveDate, week_start: WeekStart) -> Bucket {
    let this_week = week_key_of(today, week_start);
    let next_week = week_key_of(today + Days::new(7), week_start);

    match *scheduled {
        Scheduled::None => Bucket::Future,
        Scheduled::Date(date) => {
            if date <= today {
                Bucket::Today
            } else if date == today + Days::new(1) {
                Bucket::Tomorrow
            } else {
                let key = week_key_of(date, week_start);
                if key == this_week {
                    Bucket::ThisWeek
                } else if key == next_week {
                    Bucket::NextWeek
                } else {
                    Bucket::Future
                }
            }
        }
        Scheduled::Week(key) => {
            if key < this_week {
                Bucket::Today
            } else if key == this_week {
                Bucket::ThisWeek
            } else if key == next_week {
                Bucket::NextWeek
            } else {
                Bucket::Future
            }
        }
    }
}

/// 列 → 予定のマッピング(ドラッグ&ドロップ時に使う)。
///
/// Today → `date(today)` / Tomorrow → `date(today+1)` / ThisWeek → `week(今週)` /
/// NextWeek → `week(来週)` / Future → `none`。
#[must_use]
pub fn scheduled_for_bucket(bucket: Bucket, today: NaiveDate, week_start: WeekStart) -> Scheduled {
    match bucket {
        Bucket::Today => Scheduled::Date(today),
        Bucket::Tomorrow => Scheduled::Date(today + Days::new(1)),
        Bucket::ThisWeek => Scheduled::Week(week_key_of(today, week_start)),
        Bucket::NextWeek => Scheduled::Week(week_key_of(today + Days::new(7), week_start)),
        Bucket::Future => Scheduled::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("有効な日付")
    }

    fn bucket_of_date(date: NaiveDate, today: NaiveDate, ws: WeekStart) -> Bucket {
        derive_bucket(&Scheduled::Date(date), today, ws)
    }

    fn bucket_of_week(key: WeekKey, today: NaiveDate, ws: WeekStart) -> Bucket {
        derive_bucket(&Scheduled::Week(key), today, ws)
    }

    // ---- 週の開始日 ----

    #[test]
    fn week_start_date_monday() {
        // 2026-08-31 は月曜。
        assert_eq!(d(2026, 8, 31).weekday(), chrono::Weekday::Mon);
        for (offset, day) in [(0, 31), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6)] {
            let date = d(2026, 8, 31) + Days::new(offset);
            assert_eq!(date.day(), day, "offset {offset}");
            assert_eq!(week_start_date(date, WeekStart::Monday), d(2026, 8, 31));
        }
        // 週をまたぐと開始日も動く。
        assert_eq!(
            week_start_date(d(2026, 9, 7), WeekStart::Monday),
            d(2026, 9, 7)
        );
    }

    #[test]
    fn week_start_date_sunday() {
        // 2026-08-30 は日曜。
        assert_eq!(d(2026, 8, 30).weekday(), chrono::Weekday::Sun);
        for offset in 0..7 {
            let date = d(2026, 8, 30) + Days::new(offset);
            assert_eq!(week_start_date(date, WeekStart::Sunday), d(2026, 8, 30));
        }
        assert_eq!(
            week_start_date(d(2026, 9, 6), WeekStart::Sunday),
            d(2026, 9, 6)
        );
    }

    // ---- 週キー ----

    #[test]
    fn monday_week_key_matches_iso_week() {
        // 月曜始まりでは ISO 週番号と完全一致する。年初・年末を含めて総当たりで確認。
        let mut date = d(2019, 12, 1);
        while date < d(2031, 1, 31) {
            let iso = date.iso_week();
            assert_eq!(
                week_key_of(date, WeekStart::Monday),
                WeekKey::new(iso.year(), iso.week()),
                "{date}"
            );
            date = date + Days::new(1);
        }
    }

    #[test]
    fn week_key_is_constant_within_a_week_and_changes_across_weeks() {
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            let mut date = d(2024, 1, 1);
            while date < d(2028, 1, 1) {
                let start = week_start_date(date, ws);
                let key = week_key_of(date, ws);
                // 同じ週の全日で同じキー。
                for offset in 0..7 {
                    assert_eq!(
                        week_key_of(start + Days::new(offset), ws),
                        key,
                        "{date} {ws:?}"
                    );
                }
                // 前後の週とは異なるキーで、順序も日付順と一致する。
                let prev = week_key_of(start - Days::new(1), ws);
                let next = week_key_of(start + Days::new(7), ws);
                assert!(prev < key, "{date} {ws:?}: {prev} !< {key}");
                assert!(key < next, "{date} {ws:?}: {key} !< {next}");
                date = date + Days::new(7);
            }
        }
    }

    #[test]
    fn week_key_ordering_is_monotonic_over_a_decade() {
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            let mut date = d(2019, 12, 1);
            let mut previous = week_key_of(date, ws);
            while date < d(2031, 1, 31) {
                let key = week_key_of(date, ws);
                assert!(key >= previous, "{date} {ws:?}: {key} < {previous}");
                previous = key;
                date = date + Days::new(1);
            }
        }
    }

    #[test]
    fn week_key_new_year_boundaries() {
        // 2026-01-01 は木曜 → ISO 2026-W01。
        assert_eq!(d(2026, 1, 1).weekday(), chrono::Weekday::Thu);
        assert_eq!(
            week_key_of(d(2026, 1, 1), WeekStart::Monday),
            WeekKey::new(2026, 1)
        );
        // 2025-12-29(月)は既に ISO 2026-W01。
        assert_eq!(
            week_key_of(d(2025, 12, 29), WeekStart::Monday),
            WeekKey::new(2026, 1)
        );
        // 2027-01-01 は金曜 → ISO 2026-W53。
        assert_eq!(d(2027, 1, 1).weekday(), chrono::Weekday::Fri);
        assert_eq!(
            week_key_of(d(2027, 1, 1), WeekStart::Monday),
            WeekKey::new(2026, 53)
        );
        // 2021-01-01 は金曜 → ISO 2020-W53。
        assert_eq!(
            week_key_of(d(2021, 1, 1), WeekStart::Monday),
            WeekKey::new(2020, 53)
        );
    }

    // ---- date() のバケット導出 ----

    #[test]
    fn past_dates_roll_up_to_today() {
        let today = d(2026, 8, 31);
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            assert_eq!(bucket_of_date(today, today, ws), Bucket::Today);
            assert_eq!(
                bucket_of_date(today - Days::new(1), today, ws),
                Bucket::Today
            );
            assert_eq!(
                bucket_of_date(today - Days::new(400), today, ws),
                Bucket::Today
            );
        }
    }

    #[test]
    fn tomorrow_wins_over_week_membership() {
        // 月曜始まりで今日が日曜(週の最終日)。明日は次の週だが Tomorrow が優先される。
        let today = d(2026, 9, 6);
        assert_eq!(today.weekday(), chrono::Weekday::Sun);
        assert_eq!(
            bucket_of_date(today + Days::new(1), today, WeekStart::Monday),
            Bucket::Tomorrow
        );
        // 日曜始まりで今日が土曜(週の最終日)。
        let today = d(2026, 9, 5);
        assert_eq!(today.weekday(), chrono::Weekday::Sat);
        assert_eq!(
            bucket_of_date(today + Days::new(1), today, WeekStart::Sunday),
            Bucket::Tomorrow
        );
    }

    #[test]
    fn date_buckets_monday_start_midweek() {
        // 今日 = 2026-09-02 (水), 月曜始まり → 今週 = 08-31(月)〜09-06(日)。
        let today = d(2026, 9, 2);
        let ws = WeekStart::Monday;
        assert_eq!(bucket_of_date(d(2026, 9, 2), today, ws), Bucket::Today);
        assert_eq!(bucket_of_date(d(2026, 9, 3), today, ws), Bucket::Tomorrow);
        assert_eq!(bucket_of_date(d(2026, 9, 4), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 6), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 7), today, ws), Bucket::NextWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 13), today, ws), Bucket::NextWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 14), today, ws), Bucket::Future);
    }

    #[test]
    fn date_buckets_sunday_start_midweek() {
        // 今日 = 2026-09-02 (水), 日曜始まり → 今週 = 08-30(日)〜09-05(土)。
        let today = d(2026, 9, 2);
        let ws = WeekStart::Sunday;
        assert_eq!(bucket_of_date(d(2026, 9, 2), today, ws), Bucket::Today);
        assert_eq!(bucket_of_date(d(2026, 9, 3), today, ws), Bucket::Tomorrow);
        assert_eq!(bucket_of_date(d(2026, 9, 4), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 5), today, ws), Bucket::ThisWeek);
        // 09-06 は日曜 = 次の週の初日。
        assert_eq!(bucket_of_date(d(2026, 9, 6), today, ws), Bucket::NextWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 12), today, ws), Bucket::NextWeek);
        assert_eq!(bucket_of_date(d(2026, 9, 13), today, ws), Bucket::Future);
    }

    #[test]
    fn week_start_setting_shifts_the_boundary() {
        // 今日 = 2026-09-02 (水)。09-06 (日) は月曜始まりでは今週、日曜始まりでは来週。
        let today = d(2026, 9, 2);
        assert_eq!(
            bucket_of_date(d(2026, 9, 6), today, WeekStart::Monday),
            Bucket::ThisWeek
        );
        assert_eq!(
            bucket_of_date(d(2026, 9, 6), today, WeekStart::Sunday),
            Bucket::NextWeek
        );
    }

    #[test]
    fn date_buckets_across_new_year() {
        // 今日 = 2026-12-30 (水), 月曜始まり → 今週 = 12-28(月)〜01-03(日)。
        let today = d(2026, 12, 30);
        let ws = WeekStart::Monday;
        assert_eq!(today.weekday(), chrono::Weekday::Wed);
        assert_eq!(bucket_of_date(d(2026, 12, 30), today, ws), Bucket::Today);
        assert_eq!(bucket_of_date(d(2026, 12, 31), today, ws), Bucket::Tomorrow);
        assert_eq!(bucket_of_date(d(2027, 1, 1), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2027, 1, 3), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2027, 1, 4), today, ws), Bucket::NextWeek);
        assert_eq!(bucket_of_date(d(2027, 1, 10), today, ws), Bucket::NextWeek);
        assert_eq!(bucket_of_date(d(2027, 1, 11), today, ws), Bucket::Future);
    }

    #[test]
    fn date_buckets_across_month_end() {
        // 今日 = 2026-01-29 (木), 月曜始まり → 今週 = 01-26(月)〜02-01(日)。
        let today = d(2026, 1, 29);
        let ws = WeekStart::Monday;
        assert_eq!(today.weekday(), chrono::Weekday::Thu);
        assert_eq!(bucket_of_date(d(2026, 1, 30), today, ws), Bucket::Tomorrow);
        assert_eq!(bucket_of_date(d(2026, 1, 31), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2026, 2, 1), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2026, 2, 2), today, ws), Bucket::NextWeek);
    }

    #[test]
    fn date_buckets_across_leap_day() {
        // 2028 はうるう年。今日 = 2028-02-28 (月), 月曜始まり。
        let today = d(2028, 2, 28);
        let ws = WeekStart::Monday;
        assert_eq!(today.weekday(), chrono::Weekday::Mon);
        assert_eq!(bucket_of_date(d(2028, 2, 29), today, ws), Bucket::Tomorrow);
        assert_eq!(bucket_of_date(d(2028, 3, 1), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2028, 3, 5), today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_date(d(2028, 3, 6), today, ws), Bucket::NextWeek);
    }

    /// 週の各曜日を「今日」として、date 予定のバケット遷移が
    /// Today → Tomorrow → ThisWeek → NextWeek → Future の順に単調であることを確認する。
    #[test]
    fn date_bucket_progression_is_monotonic_for_every_weekday() {
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            let mut today = d(2026, 8, 24);
            // 2 週間分、すべての曜日を「今日」として試す。
            for _ in 0..14 {
                let mut previous = Bucket::Today;
                for offset in 0..30 {
                    let bucket = bucket_of_date(today + Days::new(offset), today, ws);
                    assert!(
                        bucket >= previous,
                        "today={today} offset={offset} {ws:?}: {bucket:?} < {previous:?}"
                    );
                    previous = bucket;
                }
                // 最終日(29 日後)は必ず Future。
                assert_eq!(
                    bucket_of_date(today + Days::new(29), today, ws),
                    Bucket::Future
                );
                today = today + Days::new(1);
            }
        }
    }

    /// 週内のどの曜日でも、ThisWeek と判定される日は必ず「今週の週キー」を持つ。
    #[test]
    fn this_week_and_next_week_membership_agrees_with_week_key() {
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            let mut today = d(2026, 12, 20);
            for _ in 0..30 {
                for offset in 0..25 {
                    let date = today + Days::new(offset);
                    match bucket_of_date(date, today, ws) {
                        Bucket::ThisWeek => assert_eq!(
                            week_key_of(date, ws),
                            week_key_of(today, ws),
                            "today={today} date={date} {ws:?}"
                        ),
                        Bucket::NextWeek => assert_eq!(
                            week_key_of(date, ws),
                            week_key_of(today + Days::new(7), ws),
                            "today={today} date={date} {ws:?}"
                        ),
                        _ => {}
                    }
                }
                today = today + Days::new(1);
            }
        }
    }

    // ---- week() のバケット導出 ----

    #[test]
    fn week_buckets_monday_start() {
        let today = d(2026, 9, 2);
        let ws = WeekStart::Monday;
        let this_week = week_key_of(today, ws);
        let next_week = week_key_of(today + Days::new(7), ws);
        let prev_week = week_key_of(today - Days::new(7), ws);
        assert_eq!(bucket_of_week(prev_week, today, ws), Bucket::Today);
        assert_eq!(bucket_of_week(this_week, today, ws), Bucket::ThisWeek);
        assert_eq!(bucket_of_week(next_week, today, ws), Bucket::NextWeek);
        assert_eq!(
            bucket_of_week(week_key_of(today + Days::new(14), ws), today, ws),
            Bucket::Future
        );
    }

    #[test]
    fn overdue_week_rolls_up_to_today() {
        let today = d(2026, 1, 5);
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            // 前年末の週。
            assert_eq!(
                bucket_of_week(week_key_of(d(2025, 12, 20), ws), today, ws),
                Bucket::Today
            );
        }
    }

    #[test]
    fn week_buckets_across_new_year() {
        // 今日 = 2026-12-30 (水)。今週 = ISO 2026-W53、来週 = ISO 2026-W53+1 → 2027-W01。
        let today = d(2026, 12, 30);
        let ws = WeekStart::Monday;
        assert_eq!(week_key_of(today, ws), WeekKey::new(2026, 53));
        assert_eq!(week_key_of(today + Days::new(7), ws), WeekKey::new(2027, 1));
        assert_eq!(
            bucket_of_week(WeekKey::new(2026, 53), today, ws),
            Bucket::ThisWeek
        );
        assert_eq!(
            bucket_of_week(WeekKey::new(2027, 1), today, ws),
            Bucket::NextWeek
        );
        assert_eq!(
            bucket_of_week(WeekKey::new(2027, 2), today, ws),
            Bucket::Future
        );
        assert_eq!(
            bucket_of_week(WeekKey::new(2026, 52), today, ws),
            Bucket::Today
        );
    }

    /// 年末年始をまたぐ全日について、week 予定のバケットが
    /// Today / ThisWeek / NextWeek / Future のいずれかに正しく振り分けられる。
    #[test]
    fn week_bucket_progression_across_year_boundaries() {
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            for start in [d(2020, 12, 20), d(2025, 12, 20), d(2026, 12, 20)] {
                let mut today = start;
                for _ in 0..40 {
                    let this_week = week_key_of(today, ws);
                    let next_week = week_key_of(today + Days::new(7), ws);
                    assert_eq!(bucket_of_week(this_week, today, ws), Bucket::ThisWeek);
                    assert_eq!(bucket_of_week(next_week, today, ws), Bucket::NextWeek);
                    assert_eq!(
                        bucket_of_week(week_key_of(today - Days::new(7), ws), today, ws),
                        Bucket::Today
                    );
                    assert_eq!(
                        bucket_of_week(week_key_of(today + Days::new(14), ws), today, ws),
                        Bucket::Future
                    );
                    today = today + Days::new(1);
                }
            }
        }
    }

    #[test]
    fn none_is_always_future() {
        assert_eq!(
            derive_bucket(&Scheduled::None, d(2026, 9, 2), WeekStart::Monday),
            Bucket::Future
        );
    }

    // ---- 列 → 予定のマッピング ----

    #[test]
    fn column_mapping_roundtrips_through_bucket_derivation() {
        for ws in [WeekStart::Monday, WeekStart::Sunday] {
            let mut today = d(2026, 12, 25);
            for _ in 0..30 {
                for bucket in [
                    Bucket::Today,
                    Bucket::Tomorrow,
                    Bucket::ThisWeek,
                    Bucket::NextWeek,
                    Bucket::Future,
                ] {
                    let scheduled = scheduled_for_bucket(bucket, today, ws);
                    assert_eq!(
                        derive_bucket(&scheduled, today, ws),
                        bucket,
                        "today={today} {ws:?} {bucket:?}"
                    );
                }
                today = today + Days::new(1);
            }
        }
    }

    #[test]
    fn column_mapping_values() {
        let today = d(2026, 9, 2);
        let ws = WeekStart::Monday;
        assert_eq!(
            scheduled_for_bucket(Bucket::Today, today, ws),
            Scheduled::Date(d(2026, 9, 2))
        );
        assert_eq!(
            scheduled_for_bucket(Bucket::Tomorrow, today, ws),
            Scheduled::Date(d(2026, 9, 3))
        );
        assert_eq!(
            scheduled_for_bucket(Bucket::ThisWeek, today, ws),
            Scheduled::Week(WeekKey::new(2026, 36))
        );
        assert_eq!(
            scheduled_for_bucket(Bucket::NextWeek, today, ws),
            Scheduled::Week(WeekKey::new(2026, 37))
        );
        assert_eq!(
            scheduled_for_bucket(Bucket::Future, today, ws),
            Scheduled::None
        );
    }

    #[test]
    fn board_column_resolve() {
        let today = d(2026, 9, 2);
        let ws = WeekStart::Monday;
        let current = Scheduled::Date(d(2026, 9, 10));
        assert_eq!(
            BoardColumn::Today.resolve(current, today, ws),
            (TaskStatus::Todo, Scheduled::Date(today))
        );
        // New / Doing / Done は予定を保持する。
        assert_eq!(
            BoardColumn::New.resolve(current, today, ws),
            (TaskStatus::New, current)
        );
        assert_eq!(
            BoardColumn::Doing.resolve(current, today, ws),
            (TaskStatus::Doing, current)
        );
        assert_eq!(
            BoardColumn::Done.resolve(current, today, ws),
            (TaskStatus::Done, current)
        );
    }
}
