mod audit;
mod bundle;
mod config;
mod server;
mod tools;

use tracing_subscriber::EnvFilter;

use crate::config::{BundleBackend, ResolvedBundleConfig, ServerConfig};
use crate::server::OkfServer;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load config from environment or default path
    let config_path = std::env::var("OKF_CONFIG").unwrap_or_else(|_| "okf-config.toml".to_string());
    let config = match std::fs::read_to_string(&config_path) {
        Ok(content) => match ServerConfig::from_toml(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to load config from {config_path}: {e}. Using default single-bundle config.");
                ServerConfig {
                    audit_dir: Some(".okf-audit".to_string()),
                    search: None,
                    bundles: std::collections::HashMap::new(),
                }
            }
        },
        Err(_) => {
            let bundle_path =
                std::env::var("OKF_BUNDLE_PATH").unwrap_or_else(|_| "bundles".to_string());
            let bundle_name =
                std::env::var("OKF_BUNDLE_NAME").unwrap_or_else(|_| "default".to_string());
            tracing::info!(
                "No config file found. Using single bundle at {bundle_path} with name {bundle_name}"
            );

            let mut bundles = std::collections::HashMap::new();
            bundles.insert(
                bundle_name,
                config::BundleConfig {
                    backend: "fs".to_string(),
                    path: bundle_path,
                    remote: None,
                    default_branch: None,
                    branch_policy: None,
                    auth: None,
                    write_allowlist: None,
                },
            );

            ServerConfig {
                audit_dir: Some(".okf-audit".to_string()),
                search: None,
                bundles,
            }
        }
    };

    let resolved_bundles: Vec<ResolvedBundleConfig> = config
        .bundles
        .into_iter()
        .map(|(name, bc)| ResolvedBundleConfig {
            name: name.clone(),
            backend: match bc.backend.as_str() {
                "git" => BundleBackend::Git,
                _ => BundleBackend::Fs,
            },
            path: std::path::PathBuf::from(bc.path),
            remote: bc.remote,
            default_branch: bc.default_branch,
            branch_policy: bc.branch_policy,
            auth: bc.auth.map(|a| crate::config::AuthConfig {
                ssh_key: a.ssh_key,
                token_env: a.token_env,
            }),
        })
        .collect();

    if resolved_bundles.is_empty() {
        tracing::error!(
            "No bundles configured. Create an okf-config.toml or set OKF_BUNDLE_PATH / OKF_BUNDLE_NAME."
        );
        std::process::exit(1);
    }

    let server = OkfServer::new(resolved_bundles, config.audit_dir.as_deref())
        .unwrap_or_else(|e| {
            tracing::error!("Failed to create server: {e}");
            std::process::exit(1);
        });

    tracing::info!("OKF MCP Server started");

    if let Err(e) = server.start().await {
        tracing::error!("Server error: {e}");
        std::process::exit(1);
    }
}
