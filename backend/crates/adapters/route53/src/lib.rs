mod client;
mod convert;
mod error;

use async_trait::async_trait;
use aws_sdk_route53::{
    types::{Change, ChangeBatch, ChangeAction, ResourceRecord, ResourceRecordSet, RrType},
    Client,
};
use dns_manager_core::{
    Credentials, ProviderAdapter, ProviderConstraints, ProviderError, ProviderRecord,
    ProviderResult, ProviderZone, ProviderZoneDetail, RecordKey, RecordType, ValidationResult,
};

use convert::{
    add_txt_quotes, normalize_zone_id, record_type_to_rr_type, rr_type_to_record_type,
    strip_txt_quotes, to_fqdn, to_relative_name,
};
use error::{is_record_not_found, map_sdk_error};

// ── Route53 constraints (§5.8, §8.7) ─────────────────────────────────────────

/// Route53 accepts TTL ≥ 0, but we mandate 60 s as the practical minimum
/// per the design spec.
const MIN_TTL: u32 = 60;
const MAX_TTL: u32 = 2_147_483_647; // Route53 documented upper bound
const NAME_MAX_LEN: usize = 255;
/// Route53 default API rate limit: 5 requests / second (§8.7).
const RATE_LIMIT_RPS: f64 = 5.0;

static SUPPORTED: &[RecordType] = &[
    RecordType::A,
    RecordType::Aaaa,
    RecordType::Cname,
    RecordType::Mx,
    RecordType::Txt,
    RecordType::Ns,
    RecordType::Srv,
    RecordType::Caa,
];

// ── Adapter ───────────────────────────────────────────────────────────────────

/// Route53 implementation of [`ProviderAdapter`].
///
/// Stateless: a new AWS SDK client is constructed per call from the decrypted
/// [`Credentials`] provided by the worker. No secrets are retained beyond the
/// lifetime of each method call.
pub struct Route53Adapter;

impl Default for Route53Adapter {
    fn default() -> Self {
        Self
    }
}

impl Route53Adapter {
    pub fn new() -> Self {
        Self
    }

    fn sdk_client(&self, creds: &Credentials) -> ProviderResult<Client> {
        client::build_client(creds)
    }

    /// Return the FQDN of a hosted zone (e.g. `"example.com."`).
    /// Needed to convert between relative record names and FQDNs.
    async fn zone_fqdn(&self, client: &Client, zone_id: &str) -> ProviderResult<String> {
        let resp = client
            .get_hosted_zone()
            .id(zone_id)
            .send()
            .await
            .map_err(|e| {
                if e.to_string().contains("NoSuchHostedZone") {
                    ProviderError::ZoneNotFound(zone_id.to_string())
                } else {
                    map_sdk_error(e)
                }
            })?;

        // hosted_zone() returns Option<&HostedZone>; always Some in a
        // successful GetHostedZone response.
        let hz = resp
            .hosted_zone()
            .ok_or_else(|| ProviderError::ApiError("GetHostedZone returned empty body".into()))?;
        Ok(hz.name().to_string())
    }
}

// ── ProviderAdapter impl ──────────────────────────────────────────────────────

#[async_trait]
impl ProviderAdapter for Route53Adapter {
    // ── metadata ──────────────────────────────────────────────────────────────

    fn provider_id(&self) -> &str {
        "route53"
    }

    fn supported_record_types(&self) -> &[RecordType] {
        SUPPORTED
    }

    fn constraints(&self) -> ProviderConstraints {
        ProviderConstraints {
            min_ttl: Some(MIN_TTL),
            max_ttl: Some(MAX_TTL),
            name_max_length: Some(NAME_MAX_LEN),
            rate_limit_rps: Some(RATE_LIMIT_RPS),
            dnssec_supported: true,
        }
    }

    // ── Zone operations ───────────────────────────────────────────────────────

    async fn list_zones(&self, creds: &Credentials) -> ProviderResult<Vec<ProviderZone>> {
        let client = self.sdk_client(creds)?;
        let mut zones = Vec::new();

        // PaginationStream::next() is an inherent async method on aws-smithy-async's
        // PaginationStream<Item>; it does not require a StreamExt import.
        let mut paginator = client.list_hosted_zones().into_paginator().send();
        while let Some(page) = paginator.next().await {
            let page = page.map_err(map_sdk_error)?;
            for hz in page.hosted_zones() {
                zones.push(ProviderZone {
                    provider_zone_id: normalize_zone_id(hz.id()),
                    name: hz.name().trim_end_matches('.').to_string(),
                });
            }
        }

        Ok(zones)
    }

    async fn get_zone(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
    ) -> ProviderResult<ProviderZoneDetail> {
        let client = self.sdk_client(creds)?;
        let zone_id = normalize_zone_id(provider_zone_id);

        let resp = client
            .get_hosted_zone()
            .id(&zone_id)
            .send()
            .await
            .map_err(|e| {
                if e.to_string().contains("NoSuchHostedZone") {
                    ProviderError::ZoneNotFound(zone_id.clone())
                } else {
                    map_sdk_error(e)
                }
            })?;

        let hz = resp
            .hosted_zone()
            .ok_or_else(|| ProviderError::ApiError("GetHostedZone returned empty body".into()))?;

        Ok(ProviderZoneDetail {
            provider_zone_id: normalize_zone_id(hz.id()),
            name: hz.name().trim_end_matches('.').to_string(),
            record_count: hz
                .resource_record_set_count()
                .map(|n: i64| n.max(0) as u64),
        })
    }

    // ── Record operations (all idempotent per §5.8) ───────────────────────────

    async fn list_records(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
    ) -> ProviderResult<Vec<ProviderRecord>> {
        let client = self.sdk_client(creds)?;
        let zone_id = normalize_zone_id(provider_zone_id);

        // Zone FQDN is needed to convert Route53 FQDNs → relative names.
        let zone_fqdn = self.zone_fqdn(&client, &zone_id).await?;

        let mut records = Vec::new();

        // list_resource_record_sets uses a name/type cursor rather than a token,
        // so it does not expose a paginator helper – drive the loop manually.
        let mut next_name: Option<String> = None;
        let mut next_type: Option<RrType> = None;

        loop {
            let mut builder = client
                .list_resource_record_sets()
                .hosted_zone_id(&zone_id);
            if let Some(ref name) = next_name {
                builder = builder.start_record_name(name);
            }
            if let Some(ref rt) = next_type {
                builder = builder.start_record_type(rt.clone());
            }

            let resp = builder.send().await.map_err(|e| map_sdk_error(e))?;

            for rrs in resp.resource_record_sets() {
                // Skip Route53-only types (SOA, PTR, …) outside our set.
                let Some(record_type) = rr_type_to_record_type(rrs.r#type()) else {
                    continue;
                };
                // Skip alias records (no TTL, Route53 extension with no DNS equivalent).
                if rrs.alias_target().is_some() {
                    continue;
                }
                let Some(raw_ttl) = rrs.ttl() else {
                    continue;
                };

                let name = to_relative_name(rrs.name(), &zone_fqdn);
                let values: Vec<String> = rrs
                    .resource_records()
                    .iter()
                    .map(|rr: &ResourceRecord| {
                        let v = rr.value();
                        if record_type == RecordType::Txt {
                            strip_txt_quotes(v)
                        } else {
                            v.to_string()
                        }
                    })
                    .collect();

                records.push(ProviderRecord {
                    name,
                    record_type,
                    ttl: (raw_ttl as i64).max(0) as u32,
                    values,
                });
            }

            if resp.is_truncated() {
                next_name = resp.next_record_name().map(|s| s.to_string());
                next_type = resp.next_record_type().cloned();
            } else {
                break;
            }
        }

        Ok(records)
    }

    /// Idempotent upsert using Route53's `UPSERT` change action.
    ///
    /// Route53 creates the record when it does not exist, or replaces it
    /// in-place when it does – satisfying §5.8 with a single API call.
    async fn upsert_record(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
        record: &ProviderRecord,
    ) -> ProviderResult<()> {
        let client = self.sdk_client(creds)?;
        let zone_id = normalize_zone_id(provider_zone_id);
        let zone_fqdn = self.zone_fqdn(&client, &zone_id).await?;

        let fqdn = to_fqdn(&record.name, &zone_fqdn);
        let rr_type = record_type_to_rr_type(record.record_type);

        let resource_records = record
            .values
            .iter()
            .map(|v| {
                let wire_value = if record.record_type == RecordType::Txt {
                    add_txt_quotes(v)
                } else {
                    v.clone()
                };
                ResourceRecord::builder()
                    .value(wire_value)
                    .build()
                    .map_err(|e| ProviderError::ApiError(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let rrs = ResourceRecordSet::builder()
            .name(&fqdn)
            .r#type(rr_type)
            .ttl(record.ttl as i64)
            .set_resource_records(Some(resource_records))
            .build()
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let change = Change::builder()
            .action(ChangeAction::Upsert)
            .resource_record_set(rrs)
            .build()
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let batch = ChangeBatch::builder()
            .comment("managed by dns-manager")
            .changes(change)
            .build()
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        client
            .change_resource_record_sets()
            .hosted_zone_id(&zone_id)
            .change_batch(batch)
            .send()
            .await
            .map_err(|e| map_sdk_error(e))?;

        Ok(())
    }

    /// Idempotent delete (§5.8): returns `Ok(())` when the record is already absent.
    ///
    /// Route53's DELETE action requires the exact current TTL and values, so the
    /// record is first fetched with `list_resource_record_sets`.  A narrow race
    /// window exists (deletion between list and delete); the resulting service
    /// error is also treated as success.
    async fn delete_record(
        &self,
        creds: &Credentials,
        provider_zone_id: &str,
        key: &RecordKey,
    ) -> ProviderResult<()> {
        let client = self.sdk_client(creds)?;
        let zone_id = normalize_zone_id(provider_zone_id);
        let zone_fqdn = self.zone_fqdn(&client, &zone_id).await?;

        let fqdn = to_fqdn(&key.name, &zone_fqdn);
        let rr_type = record_type_to_rr_type(key.record_type);

        // Fetch the record to obtain its exact TTL and values for the DELETE payload.
        let resp = client
            .list_resource_record_sets()
            .hosted_zone_id(&zone_id)
            .start_record_name(&fqdn)
            .start_record_type(rr_type.clone())
            .max_items(1)
            .send()
            .await
            .map_err(|e| map_sdk_error(e))?;

        // Route53 returns records at-or-after (fqdn, rr_type) – verify exact match.
        let existing = resp
            .resource_record_sets()
            .iter()
            .find(|rrs| fqdns_equal(rrs.name(), &fqdn) && rrs.r#type() == &rr_type);

        let Some(rrs) = existing else {
            return Ok(()); // Already absent – idempotent
        };

        let change = Change::builder()
            .action(ChangeAction::Delete)
            .resource_record_set(rrs.clone())
            .build()
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let batch = ChangeBatch::builder()
            .changes(change)
            .build()
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        match client
            .change_resource_record_sets()
            .hosted_zone_id(&zone_id)
            .change_batch(batch)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                // Race: record deleted between our list and this delete.
                if is_record_not_found(&e.to_string()) {
                    Ok(())
                } else {
                    Err(map_sdk_error(e))
                }
            }
        }
    }

    // ── Validation ────────────────────────────────────────────────────────────

    /// Validate that `record` satisfies Route53-specific constraints.
    ///
    /// Checked rules:
    /// - TTL in `[60, 2_147_483_647]`
    /// - Name ≤ 255 characters
    /// - CNAME not at zone apex (`@`)
    /// - At least one value present
    /// - MX values: `"<priority> <hostname>"` format
    /// - CAA values: `"<flags> <tag> <value>"` format
    fn validate_record(&self, record: &ProviderRecord) -> ValidationResult {
        let mut errors = Vec::new();

        // ── TTL ───────────────────────────────────────────────────────────────
        if record.ttl < MIN_TTL {
            errors.push(format!(
                "TTL {} is below the Route53 minimum of {} seconds",
                record.ttl, MIN_TTL
            ));
        }
        if record.ttl > MAX_TTL {
            errors.push(format!(
                "TTL {} exceeds the Route53 maximum of {}",
                record.ttl, MAX_TTL
            ));
        }

        // ── Name length ───────────────────────────────────────────────────────
        if record.name.len() > NAME_MAX_LEN {
            errors.push(format!(
                "record name length {} exceeds DNS maximum of {} characters",
                record.name.len(),
                NAME_MAX_LEN
            ));
        }

        // ── CNAME at apex ─────────────────────────────────────────────────────
        if record.record_type == RecordType::Cname && record.name == "@" {
            errors.push("CNAME records cannot be placed at the zone apex (@)".to_string());
        }

        // ── Values present ────────────────────────────────────────────────────
        if record.values.is_empty() {
            errors.push("record must have at least one value".to_string());
        }

        // ── MX format: "<priority> <hostname>" ────────────────────────────────
        if record.record_type == RecordType::Mx {
            for v in &record.values {
                let mut parts = v.splitn(2, ' ');
                let priority_ok = parts.next().and_then(|p| p.parse::<u16>().ok()).is_some();
                let exchange_ok = parts.next().map(|s| !s.is_empty()).unwrap_or(false);
                if !priority_ok || !exchange_ok {
                    errors.push(format!(
                        "invalid MX value '{v}': expected \"<priority> <hostname>\" \
                         (e.g. \"10 mail.example.com.\")"
                    ));
                }
            }
        }

        // ── CAA format: "<flags> <tag> <value>" ──────────────────────────────
        if record.record_type == RecordType::Caa {
            for v in &record.values {
                let parts: Vec<&str> = v.splitn(3, ' ').collect();
                let flags_ok = parts.first().and_then(|f| f.parse::<u8>().ok()).is_some();
                let tag_ok = parts.get(1).map(|t| !t.is_empty()).unwrap_or(false);
                let val_ok = parts.get(2).map(|v| !v.is_empty()).unwrap_or(false);
                if !flags_ok || !tag_ok || !val_ok {
                    errors.push(format!(
                        "invalid CAA value '{v}': expected \"<flags> <tag> <value>\" \
                         (e.g. \"0 issue \\\"letsencrypt.org\\\"\")"
                    ));
                }
            }
        }

        if errors.is_empty() {
            ValidationResult::ok()
        } else {
            ValidationResult::err(errors)
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compare two FQDNs ignoring trailing dots and ASCII case.
fn fqdns_equal(a: &str, b: &str) -> bool {
    let a = a.strip_suffix('.').unwrap_or(a);
    let b = b.strip_suffix('.').unwrap_or(b);
    a.eq_ignore_ascii_case(b)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> Route53Adapter {
        Route53Adapter::new()
    }

    fn record(name: &str, rt: RecordType, ttl: u32, values: &[&str]) -> ProviderRecord {
        ProviderRecord {
            name: name.to_string(),
            record_type: rt,
            ttl,
            values: values.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn provider_id_is_route53() {
        assert_eq!(adapter().provider_id(), "route53");
    }

    #[test]
    fn constraints_min_ttl() {
        assert_eq!(adapter().constraints().min_ttl, Some(MIN_TTL));
    }

    // ── validate_record ───────────────────────────────────────────────────────

    #[test]
    fn valid_a_record() {
        let r = record("www", RecordType::A, 300, &["1.2.3.4"]);
        assert!(adapter().validate_record(&r).valid);
    }

    #[test]
    fn rejects_ttl_below_minimum() {
        let r = record("www", RecordType::A, 30, &["1.2.3.4"]);
        let vr = adapter().validate_record(&r);
        assert!(!vr.valid);
        assert!(vr.errors.iter().any(|e| e.contains("minimum")));
    }

    #[test]
    fn accepts_ttl_at_minimum() {
        let r = record("www", RecordType::A, MIN_TTL, &["1.2.3.4"]);
        assert!(adapter().validate_record(&r).valid);
    }

    #[test]
    fn rejects_cname_at_apex() {
        let r = record("@", RecordType::Cname, 300, &["target.example.com."]);
        let vr = adapter().validate_record(&r);
        assert!(!vr.valid);
        assert!(vr.errors.iter().any(|e| e.contains("apex")));
    }

    #[test]
    fn accepts_cname_on_subdomain() {
        let r = record("www", RecordType::Cname, 300, &["target.example.com."]);
        assert!(adapter().validate_record(&r).valid);
    }

    #[test]
    fn rejects_empty_values() {
        let r = record("www", RecordType::A, 300, &[]);
        let vr = adapter().validate_record(&r);
        assert!(!vr.valid);
        assert!(vr.errors.iter().any(|e| e.contains("at least one value")));
    }

    #[test]
    fn rejects_invalid_mx() {
        let r = record("@", RecordType::Mx, 300, &["not-valid"]);
        assert!(!adapter().validate_record(&r).valid);
    }

    #[test]
    fn accepts_valid_mx() {
        let r = record("@", RecordType::Mx, 300, &["10 mail.example.com."]);
        assert!(adapter().validate_record(&r).valid);
    }

    #[test]
    fn rejects_invalid_caa() {
        let r = record("@", RecordType::Caa, 300, &["notvalid"]);
        assert!(!adapter().validate_record(&r).valid);
    }

    #[test]
    fn accepts_valid_caa() {
        let r = record("@", RecordType::Caa, 300, &["0 issue \"letsencrypt.org\""]);
        assert!(adapter().validate_record(&r).valid);
    }

    #[test]
    fn fqdns_equal_normalisation() {
        assert!(fqdns_equal("example.com.", "example.com"));
        assert!(fqdns_equal("EXAMPLE.COM.", "example.com."));
        assert!(!fqdns_equal("www.example.com.", "example.com."));
    }
}
