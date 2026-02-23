pub mod bundle_updates;
pub mod bundles;
pub mod clusters;

use onelauncher_core::api::cluster::dao::get_all_clusters;
use onelauncher_core::send_info;

pub async fn initialize_oneclient() {
	if let Err(err) = clusters::init_clusters().await {
		tracing::error!("failed to initialize clusters: {err}");
	}

	if let Err(err) = onelauncher_core::api::cluster::sync_clusters().await {
		tracing::error!("failed to sync clusters: {err}");
	}

	bundles::BundlesManager::get().await;
	tokio::spawn(async {
		check_and_apply_all_bundle_updates().await;
	});
}

async fn check_and_apply_all_bundle_updates() {
	tracing::info!("checking for bundle updates...");

	let clusters = match get_all_clusters().await {
		Ok(clusters) => clusters,
		Err(err) => {
			tracing::error!("failed to get clusters for bundle update check: {err}");
			return;
		}
	};

	let mut total_updates_applied = 0;
	let mut total_updates_failed = 0;

	for cluster in clusters {
		tracing::debug!(
			cluster_id = %cluster.id,
			cluster_name = %cluster.name,
			"Checking and applying bundle updates for cluster"
		);

		match bundle_updates::apply_bundle_updates(cluster.id).await {
			Ok(applied_updates) => {
				if !applied_updates.is_empty() {
					let update_count = applied_updates.len();
					total_updates_applied += update_count;

					tracing::info!(
						"applied {} bundle update(s) for cluster '{}' (id: {})",
						update_count,
						cluster.name,
						cluster.id
					);

					for update in &applied_updates {
						tracing::info!(
							"  - updated package from bundle '{}': {} -> {}",
							update.bundle_name,
							update.installed_version_id,
							update.new_version_id
						);
					}
				} else {
					tracing::debug!("no bundle updates needed for cluster '{}'", cluster.name);
				}
			}
			Err(err) => {
				total_updates_failed += 1;
				tracing::warn!(
					"failed to apply bundle updates for cluster '{}': {err}",
					cluster.name
				);
			}
		}
	}

	if total_updates_applied > 0 {
		send_info!(
			"Bundle updates applied: {} mod(s) updated from bundles",
			total_updates_applied
		);
		tracing::info!("total bundle updates applied: {total_updates_applied}");
	} else if total_updates_failed == 0 {
		tracing::info!("all bundle packages are up to date");
	}

	if total_updates_failed > 0 {
		tracing::warn!("failed to apply updates for {total_updates_failed} cluster(s)");
	}
}
