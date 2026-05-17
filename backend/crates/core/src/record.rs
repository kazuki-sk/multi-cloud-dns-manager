use serde::{Deserialize, Serialize};

/// DNS レコード種別。
/// MVP 対象: A / AAAA / CNAME / MX / TXT / NS（設計書 §7.1）。
/// SRV / CAA 等は adapter の supported_record_types() で宣言した場合のみ使用可能。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecordType {
    A,
    Aaaa,
    Cname,
    Mx,
    Txt,
    Ns,
    Srv,
    Caa,
}

/// プロバイダーから取得した DNS レコード（Actual State の単位）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    /// ゾーン相対名（例: "www", "@", "*"）
    pub name: String,
    pub record_type: RecordType,
    pub ttl: u32,
    /// レコード値のリスト（MX なら "10 mail.example.com." のように priority 込み）
    pub values: Vec<String>,
}

impl ProviderRecord {
    /// Observe Phase の Desired/Actual 比較用に values を正規化して返す。
    ///
    /// 共通: 末尾ドット除去・小文字化。
    /// MX:   priority を u16 として数値ソート。
    /// TXT:  複数 value をソートして順序非依存に。
    /// その他: ソートして順序非依存に。
    pub fn normalize_for_comparison(&self) -> Vec<String> {
        match self.record_type {
            RecordType::Mx => {
                // Separate parseable from unparseable values.
                // Unparseable values are kept as opaque strings rather than
                // silently dropped, so a misconfigured record is not hidden
                // from the diff engine.
                let mut parsed: Vec<(u16, String)> = Vec::new();
                let mut opaque: Vec<String> = Vec::new();
                for v in &self.values {
                    let trimmed = v.trim();
                    match parse_mx(trimmed) {
                        Some(entry) => parsed.push(entry),
                        None => opaque.push(trimmed.to_string()),
                    }
                }
                parsed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                opaque.sort();
                let mut result: Vec<String> = parsed
                    .into_iter()
                    .map(|(priority, exchange)| format!("{priority} {exchange}"))
                    .collect();
                result.extend(opaque);
                result
            }
            RecordType::Txt => {
                // TXT values are case-sensitive opaque strings; only trim
                // whitespace, never lowercase.
                let mut normalized: Vec<String> =
                    self.values.iter().map(|v| v.trim().to_string()).collect();
                normalized.sort();
                normalized
            }
            _ => {
                let mut normalized: Vec<String> = self
                    .values
                    .iter()
                    .map(|v| normalize_rdata(v.trim()))
                    .collect();
                normalized.sort();
                normalized
            }
        }
    }
}

/// MX RDATA `"10 mail.example.com."` を (priority, exchange) に分解して正規化する。
fn parse_mx(s: &str) -> Option<(u16, String)> {
    let (prio_str, exchange) = s.split_once(' ')?;
    let priority = prio_str.trim().parse::<u16>().ok()?;
    Some((priority, normalize_rdata(exchange.trim())))
}

/// 末尾ドットを除去して小文字化する。
fn normalize_rdata(s: &str) -> String {
    let lower = s.to_lowercase();
    match lower.strip_suffix('.') {
        Some(stripped) => stripped.to_string(),
        None => lower,
    }
}

/// actual と desired のレコードに差分があれば true を返す。
///
/// name / record_type が一致する前提で TTL と values を比較する。
/// values 比較には `normalize_for_comparison` を使い、順序・表記の揺れを吸収する。
pub fn diff(actual: &ProviderRecord, desired: &ProviderRecord) -> bool {
    actual.name != desired.name
        || actual.record_type != desired.record_type
        || actual.ttl != desired.ttl
        || actual.normalize_for_comparison() != desired.normalize_for_comparison()
}

/// ゾーン内でレコードを一意に識別するキー（name + type）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordKey {
    pub name: String,
    pub record_type: RecordType,
}

impl From<&ProviderRecord> for RecordKey {
    fn from(r: &ProviderRecord) -> Self {
        Self {
            name: r.name.clone(),
            record_type: r.record_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(record_type: RecordType, values: &[&str]) -> ProviderRecord {
        ProviderRecord {
            name: "@".to_string(),
            record_type,
            ttl: 300,
            values: values.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ── MX ───────────────────────────────────────────────────────────────────

    #[test]
    fn mx_sorts_by_priority_numerically() {
        // 辞書順では "9" > "10" になるが数値ソートで正しく並ぶことを確認
        let r = make(RecordType::Mx, &["20 mail2.example.com.", "9 mail1.example.com."]);
        assert_eq!(
            r.normalize_for_comparison(),
            vec!["9 mail1.example.com", "20 mail2.example.com"],
        );
    }

    #[test]
    fn mx_normalizes_case_and_trailing_dot() {
        let actual = make(RecordType::Mx, &["10 MAIL.EXAMPLE.COM."]);
        let desired = make(RecordType::Mx, &["10 mail.example.com"]);
        assert!(!diff(&actual, &desired));
    }

    #[test]
    fn mx_detects_priority_change() {
        let actual = make(RecordType::Mx, &["10 mail.example.com."]);
        let desired = make(RecordType::Mx, &["20 mail.example.com."]);
        assert!(diff(&actual, &desired));
    }

    // ── MX: unparseable values are retained ───────────────────────────────────

    #[test]
    fn mx_retains_unparseable_value() {
        let r = make(RecordType::Mx, &["10 mail.example.com.", "not-an-mx-value"]);
        let normalized = r.normalize_for_comparison();
        assert!(
            normalized.contains(&"not-an-mx-value".to_string()),
            "unparseable MX value must not be silently dropped; got: {normalized:?}",
        );
    }

    #[test]
    fn mx_opaque_value_causes_diff() {
        let with_opaque = make(RecordType::Mx, &["10 mail.example.com.", "not-an-mx-value"]);
        let without_opaque = make(RecordType::Mx, &["10 mail.example.com."]);
        assert!(diff(&with_opaque, &without_opaque));
    }

    // ── TXT ──────────────────────────────────────────────────────────────────

    #[test]
    fn txt_preserves_uppercase() {
        let r = make(RecordType::Txt, &["v=DKIM1; k=rsa; p=MIIBIjAN"]);
        assert_eq!(
            r.normalize_for_comparison(),
            vec!["v=DKIM1; k=rsa; p=MIIBIjAN"],
        );
    }

    #[test]
    fn txt_case_sensitive_comparison() {
        let actual = make(RecordType::Txt, &["v=DKIM1; p=ABCdef"]);
        let desired = make(RecordType::Txt, &["v=DKIM1; p=abcdef"]);
        assert!(diff(&actual, &desired));
    }

    #[test]
    fn txt_order_independent() {
        let actual = make(
            RecordType::Txt,
            &["v=spf1 include:example.com ~all", "google-site-verification=abc"],
        );
        let desired = make(
            RecordType::Txt,
            &["google-site-verification=abc", "v=spf1 include:example.com ~all"],
        );
        assert!(!diff(&actual, &desired));
    }

    #[test]
    fn txt_detects_value_change() {
        let actual = make(RecordType::Txt, &["v=spf1 ~all"]);
        let desired = make(RecordType::Txt, &["v=spf1 -all"]);
        assert!(diff(&actual, &desired));
    }

    // ── A ────────────────────────────────────────────────────────────────────

    #[test]
    fn a_order_independent() {
        let actual = make(RecordType::A, &["192.0.2.1", "192.0.2.2"]);
        let desired = make(RecordType::A, &["192.0.2.2", "192.0.2.1"]);
        assert!(!diff(&actual, &desired));
    }

    #[test]
    fn a_detects_missing_address() {
        let actual = make(RecordType::A, &["192.0.2.1", "192.0.2.2"]);
        let desired = make(RecordType::A, &["192.0.2.1"]);
        assert!(diff(&actual, &desired));
    }

    // ── TTL ──────────────────────────────────────────────────────────────────

    #[test]
    fn detects_ttl_change() {
        let actual = ProviderRecord {
            name: "@".to_string(),
            record_type: RecordType::A,
            ttl: 300,
            values: vec!["192.0.2.1".to_string()],
        };
        let desired = ProviderRecord { ttl: 600, ..actual.clone() };
        assert!(diff(&actual, &desired));
    }
}
