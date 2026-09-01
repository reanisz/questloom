//! 時刻の抽象。テストで「今日」を固定できるようにする。

use chrono::{DateTime, Local, NaiveDate, TimeDelta, TimeZone, Utc};

/// 現在時刻・今日の日付を供給する。
///
/// # UTC とローカル日付
///
/// タイムスタンプ (`created_at` / `done_at` / …) はすべて UTC で持ち、
/// 表示上の「日」はローカルタイムゾーンで決まる。**その変換は Clock の責務**にしてある
/// ([`local_date`](Self::local_date) / [`local_day_start`](Self::local_day_start))。
/// サービス層が `chrono::Local` を直接触ると、テストで「今日」を固定しても
/// タイムゾーン絡みの判定だけ実機の設定に引きずられてしまうため。
///
/// 2 つは同じ変換の両方向で、次の関係を満たすこと(実装はどちらもこれを守る)。
///
/// ```text
/// local_date(at) < date  ⟺  at < local_day_start(date)
/// ```
pub trait Clock: Send + Sync + 'static {
    /// 現在時刻 (UTC)。タイムスタンプの記録に使う。
    fn now(&self) -> DateTime<Utc>;

    /// ローカルタイムゾーンにおける今日の日付。バケット導出に使う。
    fn today(&self) -> NaiveDate;

    /// UTC の時刻が、ローカルタイムゾーンでは何日か。
    ///
    /// 「前日以前に完了したタスクをボードの Done 列から外す」判定に使う。
    fn local_date(&self, at: DateTime<Utc>) -> NaiveDate;

    /// ローカル日付の始まり (0:00) を UTC で返す。
    ///
    /// 永続化層へ「この時刻より前」を渡すための境界。
    fn local_day_start(&self, date: NaiveDate) -> DateTime<Utc>;
}

/// 日付の 0:00。`NaiveDate` は必ず 0:00 を表現できる。
fn midnight(date: NaiveDate) -> chrono::NaiveDateTime {
    date.and_hms_opt(0, 0, 0).expect("0:00 は常に有効")
}

/// システム時刻を使う実装。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn today(&self) -> NaiveDate {
        Local::now().date_naive()
    }

    fn local_date(&self, at: DateTime<Utc>) -> NaiveDate {
        at.with_timezone(&Local).date_naive()
    }

    fn local_day_start(&self, date: NaiveDate) -> DateTime<Utc> {
        let start = midnight(date);
        if let Some(at) = Local.from_local_datetime(&start).earliest() {
            return at.with_timezone(&Utc);
        }
        // 夏時間の切り替えで 0:00 そのものが存在しない地域(例: ブラジル)。
        // その日の正午のオフセットで近似する(ずれても飛んだ 1 時間の内側に収まる)。
        Local
            .from_local_datetime(&(start + TimeDelta::hours(12)))
            .earliest()
            .map_or_else(
                || Utc.from_utc_datetime(&start),
                |noon| noon.with_timezone(&Utc) - TimeDelta::hours(12),
            )
    }
}

/// テスト用の固定クロック。
///
/// ローカルタイムゾーンは「UTC から固定オフセット」として持つ
/// ([`with_offset_hours`](Self::with_offset_hours))。既定は UTC なので、
/// タイムゾーン境界を試したいテストだけがオフセットを指定すればよい。
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: DateTime<Utc>,
    today: NaiveDate,
    /// ローカルタイムゾーンの UTC からのずれ(秒)。東が正。
    offset_secs: i32,
}

impl FixedClock {
    /// 指定した日付の 12:00 UTC を「現在」とするクロックを作る。
    ///
    /// # Panics
    /// `today` が表現できない時刻を含む場合(通常起こらない)。
    #[must_use]
    pub fn at(today: NaiveDate) -> Self {
        let now = Utc.from_utc_datetime(&today.and_hms_opt(12, 0, 0).expect("12:00 は常に有効"));
        Self::new(now, today)
    }

    /// 現在時刻と今日の日付を個別に指定する。
    #[must_use]
    pub const fn new(now: DateTime<Utc>, today: NaiveDate) -> Self {
        Self {
            now,
            today,
            offset_secs: 0,
        }
    }

    /// ローカルタイムゾーンを「UTC+`hours`」にする(例: JST なら 9)。
    #[must_use]
    pub const fn with_offset_hours(mut self, hours: i32) -> Self {
        self.offset_secs = hours * 3600;
        self
    }

    fn offset(self) -> TimeDelta {
        TimeDelta::seconds(i64::from(self.offset_secs))
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn today(&self) -> NaiveDate {
        self.today
    }

    fn local_date(&self, at: DateTime<Utc>) -> NaiveDate {
        (at + self.offset()).date_naive()
    }

    fn local_day_start(&self, date: NaiveDate) -> DateTime<Utc> {
        Utc.from_utc_datetime(&midnight(date)) - self.offset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("有効な日付")
    }

    fn at(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .expect("RFC3339")
            .with_timezone(&Utc)
    }

    #[test]
    fn a_fixed_clock_defaults_to_utc() {
        let clock = FixedClock::at(date(2026, 9, 2));
        assert_eq!(
            clock.local_date(at("2026-09-02T23:59:59Z")),
            date(2026, 9, 2)
        );
        assert_eq!(
            clock.local_date(at("2026-09-03T00:00:00Z")),
            date(2026, 9, 3)
        );
        assert_eq!(
            clock.local_day_start(date(2026, 9, 2)),
            at("2026-09-02T00:00:00Z")
        );
    }

    /// UTC では前日でも、ローカル(+9)では今日という境界。
    #[test]
    fn a_fixed_offset_shifts_the_local_date() {
        let clock = FixedClock::at(date(2026, 9, 2)).with_offset_hours(9);
        // 09-01 15:00Z = 09-02 00:00+09:00 がその日の始まり。
        assert_eq!(
            clock.local_day_start(date(2026, 9, 2)),
            at("2026-09-01T15:00:00Z")
        );
        assert_eq!(
            clock.local_date(at("2026-09-01T15:00:00Z")),
            date(2026, 9, 2)
        );
        assert_eq!(
            clock.local_date(at("2026-09-01T14:59:59Z")),
            date(2026, 9, 1)
        );
    }

    /// 西側(負のオフセット)でも同じ関係が成り立つ。
    #[test]
    fn a_negative_offset_shifts_the_other_way() {
        let clock = FixedClock::at(date(2026, 9, 2)).with_offset_hours(-5);
        assert_eq!(
            clock.local_day_start(date(2026, 9, 2)),
            at("2026-09-02T05:00:00Z")
        );
        assert_eq!(
            clock.local_date(at("2026-09-02T04:59:59Z")),
            date(2026, 9, 1)
        );
        assert_eq!(
            clock.local_date(at("2026-09-02T05:00:00Z")),
            date(2026, 9, 2)
        );
    }

    /// trait ドキュメントの不変条件 `local_date(at) < date ⟺ at < local_day_start(date)`。
    #[test]
    fn the_two_directions_agree() {
        for hours in [-11, -5, 0, 9, 14] {
            let clock = FixedClock::at(date(2026, 9, 2)).with_offset_hours(hours);
            let today = date(2026, 9, 2);
            let start = clock.local_day_start(today);
            for minutes in [-1, 0, 1, 60, -60] {
                let moment = start + TimeDelta::minutes(minutes);
                assert_eq!(
                    clock.local_date(moment) < today,
                    moment < start,
                    "offset={hours}h minutes={minutes}"
                );
            }
        }
    }

    /// システムクロックでも両方向が食い違わないこと(実行環境の TZ で確かめる)。
    #[test]
    fn the_system_clock_is_consistent_with_itself() {
        let clock = SystemClock;
        let today = clock.today();
        let start = clock.local_day_start(today);
        assert_eq!(clock.local_date(start), today);
        assert!(clock.local_date(start - TimeDelta::seconds(1)) < today);
    }
}
