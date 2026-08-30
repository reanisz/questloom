//! `sort_order` 用の fractional indexing。
//!
//! 2 つのキーの間に入るキーを生成する純粋関数を提供する。生成されるキーは
//! 「整数部 + 小数部」からなる base62 文字列で、**バイト列としての辞書順が
//! そのまま並び順になる**。これによりドラッグ&ドロップの並び替えを 1 行の
//! UPDATE で済ませられる(docs/data-model.md 参照)。
//!
//! アルゴリズムは David Greenspan の "Implementing Fractional Indexing" と
//! 同じもの(npm `fractional-indexing` 互換)。整数部の先頭 1 文字が整数部の
//! 長さを表すため、末尾への連続追加でもキー長が線形に伸びない。

use std::fmt;

/// キーに使う文字集合。ASCII コード順が文字集合の順序と一致している必要がある。
const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// 表現可能な最小の整数部。これ自体はキーとして使えない。
const SMALLEST_INTEGER: &str = "A00000000000000000000000000";

/// 最初のキー(前後に何も無いときに生成される)。
pub const FIRST_KEY: &str = "a0";

/// fractional key の生成・検証に失敗したことを表す。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SortKeyError {
    /// キーの形式が不正。
    #[error("並び順キーの形式が不正です: {0:?}")]
    InvalidKey(String),
    /// `a < b` が成り立っていない。
    #[error("並び順キーの順序が不正です: {a:?} >= {b:?}")]
    OutOfOrder {
        /// 前のキー。
        a: String,
        /// 後のキー。
        b: String,
    },
    /// 整数部がこれ以上増減できない(実用上到達しない)。
    #[error("並び順キーの整数部がこれ以上{0}できません")]
    Exhausted(Exhausted),
}

/// [`SortKeyError::Exhausted`] の方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exhausted {
    /// 増加方向。
    Increment,
    /// 減少方向。
    Decrement,
}

impl fmt::Display for Exhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Increment => f.write_str("増加"),
            Self::Decrement => f.write_str("減少"),
        }
    }
}

type Result<T> = std::result::Result<T, SortKeyError>;

fn digit_index(c: u8) -> Option<usize> {
    DIGITS.iter().position(|d| *d == c)
}

/// 整数部の先頭文字から整数部の長さを求める。
fn integer_length(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

fn integer_part(key: &str) -> Result<&str> {
    let bytes = key.as_bytes();
    let head = *bytes.first().ok_or_else(|| invalid(key))?;
    let len = integer_length(head).ok_or_else(|| invalid(key))?;
    if len > bytes.len() {
        return Err(invalid(key));
    }
    Ok(&key[..len])
}

fn invalid(key: &str) -> SortKeyError {
    SortKeyError::InvalidKey(key.to_owned())
}

/// キーが正しい形式かどうかを検証する。
///
/// # Errors
/// 整数部の長さが合わない、小数部が `'0'` で終わる、`DIGITS` 以外の文字を含む場合にエラー。
pub fn validate_key(key: &str) -> Result<()> {
    if key == SMALLEST_INTEGER {
        return Err(invalid(key));
    }
    let int = integer_part(key)?;
    if int.bytes().skip(1).any(|c| digit_index(c).is_none()) {
        return Err(invalid(key));
    }
    let fraction = &key[int.len()..];
    if fraction.bytes().any(|c| digit_index(c).is_none()) {
        return Err(invalid(key));
    }
    if fraction.ends_with('0') {
        return Err(invalid(key));
    }
    Ok(())
}

fn validate_integer(int: &str) -> Result<()> {
    let head = *int.as_bytes().first().ok_or_else(|| invalid(int))?;
    if integer_length(head) != Some(int.len()) {
        return Err(invalid(int));
    }
    Ok(())
}

/// 整数部を 1 つ増やす。オーバーフローしたら `None`。
fn increment_integer(int: &str) -> Result<Option<String>> {
    validate_integer(int)?;
    let bytes = int.as_bytes();
    let head = bytes[0];
    let mut digits: Vec<u8> = bytes[1..].to_vec();

    let mut carry = true;
    for digit in digits.iter_mut().rev() {
        if !carry {
            break;
        }
        let index = digit_index(*digit).ok_or_else(|| invalid(int))? + 1;
        if index == DIGITS.len() {
            *digit = DIGITS[0];
        } else {
            *digit = DIGITS[index];
            carry = false;
        }
    }

    if !carry {
        return Ok(Some(build(head, &digits)));
    }
    match head {
        b'Z' => Ok(Some(FIRST_KEY.to_owned())),
        b'z' => Ok(None),
        _ => {
            let next = head + 1;
            if next > b'a' {
                digits.push(DIGITS[0]);
            } else {
                digits.pop();
            }
            Ok(Some(build(next, &digits)))
        }
    }
}

/// 整数部を 1 つ減らす。アンダーフローしたら `None`。
fn decrement_integer(int: &str) -> Result<Option<String>> {
    validate_integer(int)?;
    let bytes = int.as_bytes();
    let head = bytes[0];
    let mut digits: Vec<u8> = bytes[1..].to_vec();
    let last_digit = DIGITS[DIGITS.len() - 1];

    let mut borrow = true;
    for digit in digits.iter_mut().rev() {
        if !borrow {
            break;
        }
        let index = digit_index(*digit).ok_or_else(|| invalid(int))?;
        if index == 0 {
            *digit = last_digit;
        } else {
            *digit = DIGITS[index - 1];
            borrow = false;
        }
    }

    if !borrow {
        return Ok(Some(build(head, &digits)));
    }
    match head {
        b'a' => Ok(Some(format!("Z{}", last_digit as char))),
        b'A' => Ok(None),
        _ => {
            let next = head - 1;
            if next < b'Z' {
                digits.push(last_digit);
            } else {
                digits.pop();
            }
            Ok(Some(build(next, &digits)))
        }
    }
}

fn build(head: u8, digits: &[u8]) -> String {
    // 構成要素はすべて ASCII なので `as char` で 1 バイト文字になる。
    let mut key = String::with_capacity(digits.len() + 1);
    key.push(head as char);
    for digit in digits {
        key.push(*digit as char);
    }
    key
}

/// 小数部 `a` と `b` の中間の小数部を返す。`a < b`、かつ両者は `'0'` で終わらないこと。
fn midpoint(a: &str, b: Option<&str>) -> String {
    if let Some(b) = b {
        // 共通接頭辞を切り出して再帰する。
        let mut n = 0;
        let a_bytes = a.as_bytes();
        let b_bytes = b.as_bytes();
        while n < b_bytes.len() {
            let ac = a_bytes.get(n).copied().unwrap_or(DIGITS[0]);
            if ac != b_bytes[n] {
                break;
            }
            n += 1;
        }
        if n > 0 {
            let rest = midpoint(a.get(n..).unwrap_or(""), Some(&b[n..]));
            return format!("{}{}", &b[..n], rest);
        }
    }

    let digit_a = a
        .as_bytes()
        .first()
        .and_then(|c| digit_index(*c))
        .unwrap_or(0);
    let digit_b = match b {
        Some(b) => b
            .as_bytes()
            .first()
            .and_then(|c| digit_index(*c))
            .unwrap_or(DIGITS.len()),
        None => DIGITS.len(),
    };

    if digit_b > digit_a + 1 {
        let mid = (digit_a + digit_b).div_ceil(2);
        return (DIGITS[mid] as char).to_string();
    }
    match b {
        Some(b) if b.len() > 1 => b[..1].to_owned(),
        _ => {
            let rest = midpoint(a.get(1..).unwrap_or(""), None);
            format!("{}{}", DIGITS[digit_a] as char, rest)
        }
    }
}

/// `a` と `b` の間に入るキーを生成する。
///
/// - `a` が `None` なら「先頭に挿入」、`b` が `None` なら「末尾に追加」を意味する。
/// - 両方 `None` なら [`FIRST_KEY`] を返す。
///
/// # Errors
/// キーの形式が不正な場合、または `a >= b` の場合にエラーを返す。
pub fn generate_key_between(a: Option<&str>, b: Option<&str>) -> Result<String> {
    if let Some(a) = a {
        validate_key(a)?;
    }
    if let Some(b) = b {
        validate_key(b)?;
    }
    if let (Some(a), Some(b)) = (a, b) {
        if a >= b {
            return Err(SortKeyError::OutOfOrder {
                a: a.to_owned(),
                b: b.to_owned(),
            });
        }
    }

    match (a, b) {
        (None, None) => Ok(FIRST_KEY.to_owned()),
        (None, Some(b)) => {
            let int_b = integer_part(b)?;
            let fraction_b = &b[int_b.len()..];
            if int_b == SMALLEST_INTEGER {
                return Ok(format!("{int_b}{}", midpoint("", Some(fraction_b))));
            }
            if int_b < b {
                return Ok(int_b.to_owned());
            }
            decrement_integer(int_b)?.ok_or(SortKeyError::Exhausted(Exhausted::Decrement))
        }
        (Some(a), None) => {
            let int_a = integer_part(a)?;
            let fraction_a = &a[int_a.len()..];
            Ok(match increment_integer(int_a)? {
                Some(next) => next,
                None => format!("{int_a}{}", midpoint(fraction_a, None)),
            })
        }
        (Some(a), Some(b)) => {
            let int_a = integer_part(a)?;
            let fraction_a = &a[int_a.len()..];
            let int_b = integer_part(b)?;
            let fraction_b = &b[int_b.len()..];
            if int_a == int_b {
                return Ok(format!("{int_a}{}", midpoint(fraction_a, Some(fraction_b))));
            }
            let next =
                increment_integer(int_a)?.ok_or(SortKeyError::Exhausted(Exhausted::Increment))?;
            if next.as_str() < b {
                Ok(next)
            } else {
                Ok(format!("{int_a}{}", midpoint(fraction_a, None)))
            }
        }
    }
}

/// `a` と `b` の間に `n` 個のキーを昇順で生成する。
///
/// # Errors
/// [`generate_key_between`] と同じ条件でエラーを返す。
pub fn generate_keys_between(a: Option<&str>, b: Option<&str>, n: usize) -> Result<Vec<String>> {
    let mut keys = Vec::with_capacity(n);
    let mut lower = a.map(str::to_owned);
    for _ in 0..n {
        let key = generate_key_between(lower.as_deref(), b)?;
        lower = Some(key.clone());
        keys.push(key);
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn between(a: Option<&str>, b: Option<&str>) -> String {
        generate_key_between(a, b).expect("キー生成に成功する")
    }

    #[test]
    fn first_key_when_list_is_empty() {
        assert_eq!(between(None, None), "a0");
    }

    #[test]
    fn append_at_the_end_keeps_keys_short() {
        assert_eq!(between(Some("a0"), None), "a1");
        assert_eq!(between(Some("a1"), None), "a2");
        assert_eq!(between(Some("az"), None), "b00");
        assert_eq!(between(Some("b00"), None), "b01");
        assert_eq!(between(Some("Zz"), None), "a0");
        // 整数部が上限に達したら小数部で伸ばす。
        let largest_integer = "z".repeat(27);
        assert_eq!(
            between(Some(&largest_integer), None),
            format!("{largest_integer}V")
        );
    }

    #[test]
    fn prepend_at_the_front() {
        assert_eq!(between(None, Some("a0")), "Zz");
        assert_eq!(between(None, Some("Zz")), "Zy");
        assert_eq!(between(None, Some("a1")), "a0");
        assert_eq!(between(None, Some("Z0")), "Yzz");
    }

    #[test]
    fn insert_in_the_middle() {
        assert_eq!(between(Some("a0"), Some("a1")), "a0V");
        assert_eq!(between(Some("a0V"), Some("a1")), "a0l");
        assert_eq!(between(Some("a0"), Some("a0V")), "a0G");
        assert_eq!(between(Some("a0"), Some("a2")), "a1");
    }

    #[test]
    fn generated_key_is_always_strictly_between() {
        // ランダムではなく決定的に、中央へ 200 回連続で挿入し続ける。
        let mut low = between(None, None);
        let mut high = between(Some(&low), None);
        for i in 0..200 {
            let mid = between(Some(&low), Some(&high));
            assert!(low < mid, "step {i}: {low} !< {mid}");
            assert!(mid < high, "step {i}: {mid} !< {high}");
            validate_key(&mid).expect("生成されたキーは妥当");
            if i % 2 == 0 {
                low = mid;
            } else {
                high = mid;
            }
        }
    }

    #[test]
    fn repeated_append_stays_ordered_and_compact() {
        let mut keys: Vec<String> = Vec::new();
        for _ in 0..2000 {
            let key = between(keys.last().map(String::as_str), None);
            keys.push(key);
        }
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        let longest = keys.iter().map(String::len).max().unwrap();
        assert!(longest <= 4, "末尾追加でキーが伸びすぎている: {longest}");
    }

    #[test]
    fn repeated_prepend_stays_ordered_and_compact() {
        let mut keys: Vec<String> = Vec::new();
        for _ in 0..2000 {
            let key = between(None, keys.first().map(String::as_str));
            keys.insert(0, key);
        }
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        let longest = keys.iter().map(String::len).max().unwrap();
        assert!(longest <= 4, "先頭挿入でキーが伸びすぎている: {longest}");
    }

    #[test]
    fn generate_keys_between_is_ascending() {
        let keys = generate_keys_between(Some("a0"), Some("a1"), 10).unwrap();
        assert_eq!(keys.len(), 10);
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert!(keys.first().unwrap().as_str() > "a0");
        assert!(keys.last().unwrap().as_str() < "a1");
    }

    #[test]
    fn rejects_out_of_order_arguments() {
        assert!(matches!(
            generate_key_between(Some("a1"), Some("a0")),
            Err(SortKeyError::OutOfOrder { .. })
        ));
        assert!(matches!(
            generate_key_between(Some("a0"), Some("a0")),
            Err(SortKeyError::OutOfOrder { .. })
        ));
    }

    #[test]
    fn rejects_malformed_keys() {
        for bad in ["", "0", "a", "a0V0", "!!", SMALLEST_INTEGER] {
            assert!(
                validate_key(bad).is_err(),
                "{bad:?} は不正として弾かれるべき"
            );
            assert!(generate_key_between(Some(bad), None).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn integer_length_encoding() {
        assert_eq!(integer_length(b'a'), Some(2));
        assert_eq!(integer_length(b'b'), Some(3));
        assert_eq!(integer_length(b'Z'), Some(2));
        assert_eq!(integer_length(b'Y'), Some(3));
        assert_eq!(integer_length(b'0'), None);
    }

    #[test]
    fn increment_and_decrement_are_inverse() {
        for key in ["a0", "a1", "az", "b00", "Zz", "Yzz"] {
            let up = increment_integer(key).unwrap().unwrap();
            let down = decrement_integer(&up).unwrap().unwrap();
            assert_eq!(down, key, "{key} -> {up} -> {down}");
        }
    }
}
