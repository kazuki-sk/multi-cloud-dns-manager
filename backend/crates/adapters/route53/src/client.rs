use aws_credential_types::Credentials as AwsCredentials;
use aws_sdk_route53::{config::Region, Client, Config};
use dns_manager_core::{Credentials, ProviderError, ProviderResult};

/// Build an authenticated Route53 client from decrypted provider credentials.
///
/// Expected keys in `creds`:
/// - `"access_key_id"`    (required)
/// - `"secret_access_key"` (required)
/// - `"session_token"`    (optional – for STS-issued temporary credentials)
///
/// Route53 is a global service; the SDK always routes to us-east-1.
pub fn build_client(creds: &Credentials) -> ProviderResult<Client> {
    let access_key_id = creds
        .0
        .get("access_key_id")
        .ok_or_else(|| ProviderError::AuthError("missing access_key_id in credentials".into()))?;
    let secret_access_key = creds.0.get("secret_access_key").ok_or_else(|| {
        ProviderError::AuthError("missing secret_access_key in credentials".into())
    })?;
    let session_token = creds.0.get("session_token").cloned();

    let aws_creds = AwsCredentials::new(
        access_key_id.as_str(),
        secret_access_key.as_str(),
        session_token,
        None, // credentials do not expire (callers rotate via re-registration §8.6)
        "dns-manager-static",
    );

    let config = Config::builder()
        .credentials_provider(aws_creds)
        .region(Region::new("us-east-1"))
        .build();

    Ok(Client::from_conf(config))
}
