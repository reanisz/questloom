//! 時刻の抽象。テストで「今日」を固定できるようにする。

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};

/// 現在時刻・今日の日付を供給する。
pub trait Clock: Send + Sync + 'static {
    /// 現在時刻 (UTC)。タイムスタンプの記録に使う。
    fn now(&self) -> DateTime<Utc>;

    /// ローカルタイムゾーンにおける今日の日付。バケット導出に使う。
    fn today(&self) -> NaiveDate;
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
}

/// テスト用の固定クロック。
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: DateTime<Utc>,
    today: NaiveDate,
}

impl FixedClock {
    /// 指定した日付の 12:00 UTC を「現在」とするクロックを作る。
    ///
    /// # Panics
    /// `today` が表現できない時刻を含む場合(通常起こらない)。
    #[must_use]
    pub fn at(today: NaiveDate) -> Self {
        let now = Utc.from_utc_datetime(&today.and_hms_opt(12, 0, 0).expect("12:00 は常に有効"));
        Self { now, today }
    }

    /// 現在時刻と今日の日付を個別に指定する。
    #[must_use]
    pub const fn new(now: DateTime<Utc>, today: NaiveDate) -> Self {
        Self { now, today }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn today(&self) -> NaiveDate {
        self.today
    }
}
