use aws_sdk_route53::error::SdkError;
use dns_manager_core::ProviderError;

/// Map any Route53 `SdkError<E>` to a `ProviderError`.
///
/// Throttling and rate-limit signals map to `RateLimitExceeded`.
/// All other service errors map to `ApiError` with the full message.
pub fn map_sdk_error<E>(err: SdkError<E>) -> ProviderError
where
    E: std::fmt::Debug + std::fmt::Display,
{
    match &err {
        SdkError::ServiceError(se) => {
            let msg = se.err().to_string();
            if is_throttle(&msg) {
                ProviderError::RateLimitExceeded
            } else {
                ProviderError::ApiError(msg)
            }
        }
        SdkError::TimeoutError(_) => ProviderError::ApiError("request to Route53 timed out".into()),
        SdkError::DispatchFailure(de) => {
            ProviderError::ApiError(format!("network dispatch failure: {de:?}"))
        }
        _ => ProviderError::ApiError(err.to_string()),
    }
}

fn is_throttle(msg: &str) -> bool {
    msg.contains("Throttling")
        || msg.contains("ThrottlingException")
        || msg.contains("RequestLimitExceeded")
        || msg.contains("Rate exceeded")
}

/// Return `true` when a `change_resource_record_sets` error indicates the
/// target record was not found – used to implement idempotent deletes (§5.8).
pub fn is_record_not_found(msg: &str) -> bool {
    // Route53 error text for DELETE of non-existent record:
    // "Tried to delete resource record set [...] but it was not found"
    msg.contains("was not found") || msg.contains("does not exist") || msg.contains("not found")
}
