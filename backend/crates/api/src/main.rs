mod error;
mod routes;

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
};

use axum::{
    extract::FromRef,
    routing::{get, patch, post},
    Router,
};
use dns_manager_adapter_route53::Route53Adapter;
use dns_manager_core::{EnvKeyProvider, KeyProvider, ProviderAdapter};
use dns_manager_db::DbPool;
use dns_manager_worker::ReconcileWorker;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use routes::{changesets, health, providers, zones};

// ── application state ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DbPool>,
    pub key_provider: Arc<dyn KeyProvider>,
}

impl FromRef<AppState> for Arc<DbPool> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.db)
    }
}

impl FromRef<AppState> for Arc<dyn KeyProvider> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.key_provider)
    }
}

fn build_adapter_registry() -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let route53: Arc<dyn ProviderAdapter> = Arc::new(Route53Adapter::new());
    HashMap::from([(route53.provider_id().to_string(), route53)])
}

// ── router ────────────────────────────────────────────────────────────────────

fn build_router(state: AppState, cors: CorsLayer) -> Router {
    let zones_routes = Router::new()
        .route("/", get(zones::list_zones).post(zones::create_zone))
        .route(
            "/{zone_id}",
            get(zones::get_zone)
                .patch(zones::update_zone)
                .delete(zones::delete_zone),
        )
        .route(
            "/{zone_id}/records",
            get(zones::list_records).post(zones::create_record),
        )
        .route(
            "/{zone_id}/records/{record_id}",
            patch(zones::update_record).delete(zones::delete_record),
        )
        .route(
            "/{zone_id}/bindings",
            get(zones::list_bindings).post(zones::create_binding),
        )
        .route(
            "/{zone_id}/bindings/{binding_id}",
            axum::routing::delete(zones::delete_binding),
        )
        .route("/{zone_id}/sync-states", get(zones::list_zone_sync_states))
        .route("/{zone_id}/push", post(zones::push_pending_records));

    let changesets_routes = Router::new()
        .route(
            "/",
            get(changesets::list_changesets).post(changesets::create_changeset),
        )
        .route("/{changeset_id}", get(changesets::get_changeset))
        .route(
            "/{changeset_id}/validate",
            post(changesets::validate_changeset),
        )
        .route("/{changeset_id}/apply", post(changesets::apply_changeset))
        .route(
            "/{changeset_id}/rollback",
            post(changesets::rollback_changeset),
        );

    let providers_routes = Router::new()
        .route(
            "/",
            get(providers::list_providers).post(providers::create_provider),
        )
        .route(
            "/{provider_id}",
            get(providers::get_provider)
                .patch(providers::update_provider)
                .delete(providers::delete_provider),
        )
        .route(
            "/{provider_id}/bindings",
            get(providers::list_provider_bindings),
        )
        .route("/{provider_id}/sync-state", get(providers::get_sync_state));

    Router::new()
        .route("/health", get(health::get_health))
        .nest("/api/v1/zones", zones_routes)
        .nest("/api/v1/changesets", changesets_routes)
        .nest("/api/v1/providers", providers_routes)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ── CORS ──────────────────────────────────────────────────────────────────────

fn build_cors() -> anyhow::Result<CorsLayer> {
    // In development (no APP_ENV or APP_ENV != "production"), allow all origins.
    // In production, tighten this to the actual frontend origin.
    let env = std::env::var("APP_ENV").unwrap_or_default();
    if env == "production" {
        // Placeholder: callers must set CORS_ALLOWED_ORIGIN in production.
        let origin = std::env::var("CORS_ALLOWED_ORIGIN").map_err(|_| {
            anyhow::anyhow!("CORS_ALLOWED_ORIGIN must be set when APP_ENV=production")
        })?;
        let origin = origin
            .parse::<axum::http::HeaderValue>()
            .map_err(|_| anyhow::anyhow!("CORS_ALLOWED_ORIGIN is not a valid header value"))?;
        Ok(CorsLayer::new()
            .allow_origin(origin)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any))
    } else {
        Ok(CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any))
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Return only the scheme and host of a database URL, stripping any embedded
/// credentials so the string is safe to write to logs.
///
/// Examples:
/// - `postgres://user:secret@host:5432/db` → `postgres://host:5432`
/// - `sqlite:dns_manager.db`               → `sqlite:dns_manager.db`
fn redact_database_url(url: &str) -> String {
    if let Some(scheme_sep) = url.find("://") {
        let after_scheme = &url[scheme_sep + 3..];
        if let Some(at_pos) = after_scheme.find('@') {
            let host_and_rest = &after_scheme[at_pos + 1..];
            let host = host_and_rest.split('/').next().unwrap_or(host_and_rest);
            let scheme = &url[..scheme_sep];
            return format!("{}://{}", scheme, host);
        }
    }
    url.to_string()
}

// ── startup ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise tracing so tower-http's TraceLayer has somewhere to write.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dns_manager_api=debug,tower_http=debug".into()),
        )
        .init();

    // ── database ──────────────────────────────────────────────────────────────
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:dns_manager.db".to_string());

    tracing::info!(database_url = %redact_database_url(&database_url), "connecting to database");
    let pool = dns_manager_db::connect(&database_url).await?;

    tracing::info!("running migrations");
    dns_manager_db::migrate(&pool).await?;

    // Validate MASTER_KEY before accepting any requests.
    // Returns an error (not a panic) so the process exits with a non-zero code
    // and a descriptive message.
    EnvKeyProvider
        .get_kek()
        .map_err(|e| anyhow::anyhow!("MASTER_KEY configuration error: {e}"))?;
    let key_provider: Arc<dyn KeyProvider> = Arc::new(EnvKeyProvider);

    let adapters = build_adapter_registry();

    let state = AppState {
        db: Arc::new(pool),
        key_provider,
    };

    // ── reconcile worker ─────────────────────────────────────────────────────
    let worker = std::sync::Arc::new(ReconcileWorker::new(
        Arc::clone(&state.db),
        adapters,
        Arc::clone(&state.key_provider),
    ));
    tokio::spawn(async move { worker.run().await });

    // ── server ────────────────────────────────────────────────────────────────
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let app = build_router(state, build_cors()?);

    tracing::info!(%addr, "listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
