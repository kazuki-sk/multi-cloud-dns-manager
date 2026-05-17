use aws_sdk_route53::types::RrType;
use dns_manager_core::RecordType;

// ── Zone ID ───────────────────────────────────────────────────────────────────

/// Strip the `/hostedzone/` prefix that Route53 returns in API responses.
///
/// Our system stores only the bare ID (e.g. "Z1234ABCDE").
/// Route53 accepts both forms when making requests, so normalising on read
/// is sufficient.
pub fn normalize_zone_id(raw: &str) -> String {
    raw.strip_prefix("/hostedzone/").unwrap_or(raw).to_string()
}

// ── Record type mapping ───────────────────────────────────────────────────────

pub fn record_type_to_rr_type(rt: RecordType) -> RrType {
    match rt {
        RecordType::A => RrType::A,
        RecordType::Aaaa => RrType::Aaaa,
        RecordType::Cname => RrType::Cname,
        RecordType::Mx => RrType::Mx,
        RecordType::Txt => RrType::Txt,
        RecordType::Ns => RrType::Ns,
        RecordType::Srv => RrType::Srv,
        RecordType::Caa => RrType::Caa,
    }
}

/// Returns `None` for Route53-only types (SOA, PTR, NAPTR, SPF …) that are
/// outside our supported set.
pub fn rr_type_to_record_type(rrt: &RrType) -> Option<RecordType> {
    match rrt {
        RrType::A => Some(RecordType::A),
        RrType::Aaaa => Some(RecordType::Aaaa),
        RrType::Cname => Some(RecordType::Cname),
        RrType::Mx => Some(RecordType::Mx),
        RrType::Txt => Some(RecordType::Txt),
        RrType::Ns => Some(RecordType::Ns),
        RrType::Srv => Some(RecordType::Srv),
        RrType::Caa => Some(RecordType::Caa),
        _ => None,
    }
}

// ── DNS name normalisation ────────────────────────────────────────────────────

/// Convert an absolute FQDN returned by Route53 to a zone-relative name.
///
/// Route53 returns `"www.example.com."` for a zone `"example.com."`.
/// We store zone-relative names: `"www"`, `"@"` (apex), `"*"` (wildcard).
pub fn to_relative_name(fqdn: &str, zone_fqdn: &str) -> String {
    let name = fqdn.strip_suffix('.').unwrap_or(fqdn);
    let zone = zone_fqdn.strip_suffix('.').unwrap_or(zone_fqdn);

    if name.eq_ignore_ascii_case(zone) {
        "@".to_string()
    } else if let Some(relative) = name
        .to_lowercase()
        .strip_suffix(&format!(".{}", zone.to_lowercase()))
    {
        relative.to_string()
    } else {
        // Already relative or from a different zone (should not happen in practice)
        name.to_string()
    }
}

/// Convert a zone-relative name back to an absolute FQDN for Route53.
pub fn to_fqdn(relative: &str, zone_fqdn: &str) -> String {
    let zone = zone_fqdn.strip_suffix('.').unwrap_or(zone_fqdn);
    if relative == "@" {
        format!("{zone}.")
    } else {
        format!("{relative}.{zone}.")
    }
}

// ── TXT record quoting ────────────────────────────────────────────────────────

/// Route53 stores TXT RDATA with surrounding double-quotes (`"value"`).
/// Strip them so our internal representation is the raw string.
pub fn strip_txt_quotes(s: &str) -> String {
    match s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner.to_string(),
        None => s.to_string(),
    }
}

/// Re-wrap a raw TXT value with double-quotes before sending to Route53.
pub fn add_txt_quotes(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s.to_string() // Already quoted
    } else {
        format!("\"{s}\"")
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_id_prefix_stripped() {
        assert_eq!(normalize_zone_id("/hostedzone/Z1234"), "Z1234");
        assert_eq!(normalize_zone_id("Z1234"), "Z1234");
    }

    #[test]
    fn relative_name_apex() {
        assert_eq!(to_relative_name("example.com.", "example.com."), "@");
    }

    #[test]
    fn relative_name_subdomain() {
        assert_eq!(to_relative_name("www.example.com.", "example.com."), "www");
    }

    #[test]
    fn relative_name_wildcard() {
        assert_eq!(to_relative_name("*.example.com.", "example.com."), "*");
    }

    #[test]
    fn fqdn_apex() {
        assert_eq!(to_fqdn("@", "example.com."), "example.com.");
    }

    #[test]
    fn fqdn_subdomain() {
        assert_eq!(to_fqdn("www", "example.com."), "www.example.com.");
    }

    #[test]
    fn txt_quote_strip() {
        assert_eq!(strip_txt_quotes("\"v=spf1 ~all\""), "v=spf1 ~all");
        assert_eq!(strip_txt_quotes("already-raw"), "already-raw");
    }

    #[test]
    fn txt_quote_add() {
        assert_eq!(add_txt_quotes("v=spf1 ~all"), "\"v=spf1 ~all\"");
        assert_eq!(add_txt_quotes("\"already\""), "\"already\"");
    }
}
