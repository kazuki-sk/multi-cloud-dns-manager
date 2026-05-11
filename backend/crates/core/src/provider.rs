use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::record::{ProviderRecord, RecordKey, RecordType};

/// プロバイダーに渡す復号済み認証情報。
/// 形式はプロバイダーごとに異なるため汎用マップで保持する。
/// DB には平文保存せず Envelope Encryption 経由で注入される（設計書 §8.6）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials(pub HashMap<String, String>);

/// プロバイダーが返すゾーン一覧エントリ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderZone {
    /// プロバイダー側のゾーン ID（例: Route53 の "Z1234ABC"）
    pub provider_zone_id: String,
    /// ゾーン名（例: "example.com"）
    pub name: String,
}

/// ゾーン詳細情報。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderZoneDetail {
    pub provider_zone_id: String,
    pub name: String,
    pub record_count: Option<u64>,
}

/// プロバイダーが課す制約（設計書 §5.8, §8.7, §8.10）。
/// 全アクティブプロバイダーの制約の最小公倍数（LCD）をコアが動的に計算するために使用する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConstraints {
    pub min_ttl: Option<u32>,
    pub max_ttl: Option<u32>,
    pub name_max_length: Option<usize>,
    /// API レート制限（秒あたりリクエスト数）。None = 制限なし（§8.7）。
    pub rate_limit_rps: Option<f64>,
    /// DNSSEC サポートフラグ（§8.10）。
    pub dnssec_supported: bool,
}

/// レコードバリデーション結果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl ValidationResult {
    pub fn ok() -> Self {
        Self { valid: true, errors: vec![] }
    }

    pub fn err(errors: Vec<String>) -> Self {
        Self { valid: false, errors }
    }
}

/// プロバイダー操作エラー。
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication failed: {0}")]
    AuthError(String),

    #[error("zone not found: {0}")]
    ZoneNotFound(String),

    #[error("record not found: {0}")]
    RecordNotFound(String),

    #[error("rate limit exceeded")]
    RateLimitExceeded,

    #[error("provider API error: {0}")]
    ApiError(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// DNS プロバイダーアダプタープラグインインターフェース（設計書 §5.8）。
///
/// - 新規プロバイダー追加はこの trait を実装するだけでよく、コア改修は不要（設計原則 #7）。
/// - `dyn ProviderAdapter` として取り扱うため `async-trait` を使用。
/// - 全ての書き込み操作（upsert / delete）は冪等でなければならない。
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    // ── メタデータ ──────────────────────────────────────────────────────────

    /// プロバイダー識別子（例: "route53", "cloudflare", "gcloud"）。
    fn provider_id(&self) -> &str;

    /// このプロバイダーが処理できるレコード種別の一覧。
    fn supported_record_types(&self) -> &[RecordType];

    /// プロバイダー固有の制約。LCD 計算とレート制限に使用する。
    fn constraints(&self) -> ProviderConstraints;

    // ── Zone 操作 ───────────────────────────────────────────────────────────

    async fn list_zones(&self, creds: &Credentials) -> ProviderResult<Vec<ProviderZone>>;

    async fn get_zone(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
    ) -> ProviderResult<ProviderZoneDetail>;

    // ── Record 操作（すべて冪等） ────────────────────────────────────────────

    async fn list_records(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
    ) -> ProviderResult<Vec<ProviderRecord>>;

    /// レコードを作成または更新する（冪等）。
    async fn upsert_record(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
        record: &ProviderRecord,
    ) -> ProviderResult<()>;

    /// レコードを削除する（冪等: 存在しない場合もエラーにしない）。
    async fn delete_record(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
        key: &RecordKey,
    ) -> ProviderResult<()>;

    // ── バリデーション ──────────────────────────────────────────────────────

    /// レコードがこのプロバイダーの制約を満たすか検証する。
    fn validate_record(&self, record: &ProviderRecord) -> ValidationResult;
}
