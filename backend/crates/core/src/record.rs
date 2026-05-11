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
