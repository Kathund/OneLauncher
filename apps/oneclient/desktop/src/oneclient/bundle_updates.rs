use chrono::{DateTime, Utc};
use onelauncher_core::api::cluster::dao::ClusterId;
use onelauncher_core::api::packages::bundle_dao;
use onelauncher_core::api::packages::modpack::data::{
	ModpackArchive, ModpackFile, ModpackFileKind,
};
use onelauncher_core::entity::cluster_packages;
use onelauncher_core::error::LauncherResult;
use onelauncher_core::{api, send_error};

use crate::oneclient::bundles::BundlesManager;

#[taurpc::ipc_type]
pub struct BundlePackageUpdate {
	pub cluster_id: ClusterId,
	pub installed_package_hash: String,
	pub installed_version_id: String,
	pub bundle_name: String,
	pub new_version_id: String,
	pub new_file: ModpackFile,
	pub installed_at: DateTime<Utc>,
}

#[taurpc::ipc_type]
pub struct BundleUpdateCheckResult {
	pub cluster_id: ClusterId,
	pub updates_available: Vec<BundlePackageUpdate>,
	pub checked_at: DateTime<Utc>,
}

pub async fn check_bundle_updates(
	cluster_id: ClusterId,
) -> LauncherResult<BundleUpdateCheckResult> {
	tracing::debug!(cluster_id = %cluster_id, "Starting bundle update check");

	let cluster = onelauncher_core::api::cluster::dao::get_cluster_by_id(cluster_id)
		.await?
		.ok_or_else(|| {
			onelauncher_core::error::LauncherError::from(anyhow::anyhow!(
				"cluster with id {} not found",
				cluster_id
			))
		})?;

	tracing::debug!(
		cluster_id = %cluster_id,
		cluster_name = %cluster.name,
		mc_version = %cluster.mc_version,
		mc_loader = ?cluster.mc_loader,
		"Found cluster for update check"
	);

	let bundle_packages = bundle_dao::get_bundle_packages_for_cluster(cluster_id).await?;

	tracing::debug!(
		cluster_id = %cluster_id,
		package_count = %bundle_packages.len(),
		"Retrieved bundle packages from database"
	);

	if bundle_packages.is_empty() {
		tracing::debug!(cluster_id = %cluster_id, "No bundle packages found, skipping update check");
		return Ok(BundleUpdateCheckResult {
			cluster_id,
			updates_available: vec![],
			checked_at: Utc::now(),
		});
	}

	let bundles = BundlesManager::get()
		.await
		.get_bundles_for(&cluster.mc_version, cluster.mc_loader)
		.await?;

	tracing::debug!(
		cluster_id = %cluster_id,
		bundle_count = %bundles.len(),
		bundle_names = ?bundles.iter().map(|b| &b.manifest.name).collect::<Vec<_>>(),
		"Retrieved available bundles"
	);

	let mut bundle_versions: std::collections::HashMap<String, (String, String, ModpackFile)> =
		std::collections::HashMap::new();

	for bundle in &bundles {
		let mut enabled_count = 0;
		let mut disabled_count = 0;
		for file in &bundle.manifest.files {
			if let ModpackFileKind::Managed((pkg, version)) = &file.kind {
				if file.enabled {
					enabled_count += 1;
					tracing::trace!(
						bundle_name = %bundle.manifest.name,
						package_id = %pkg.id,
						version_id = %version.version_id,
						"Indexed bundle package version"
					);
					bundle_versions.insert(
						pkg.id.clone(),
						(
							bundle.manifest.name.clone(),
							version.version_id.clone(),
							file.clone(),
						),
					);
				} else {
					disabled_count += 1;
				}
			}
		}
		tracing::debug!(
			bundle_name = %bundle.manifest.name,
			enabled_packages = %enabled_count,
			disabled_packages = %disabled_count,
			total_files = %bundle.manifest.files.len(),
			"Indexed bundle"
		);
	}

	tracing::debug!(
		total_indexed_packages = %bundle_versions.len(),
		"Finished indexing all bundle versions"
	);

	let mut updates_available = Vec::new();
	let mut skipped_no_package_id = 0;
	let mut skipped_no_version_id = 0;
	let mut not_in_bundle = 0;

	for bundle_pkg in &bundle_packages {
		let Some(ref pkg_id) = bundle_pkg.package_id else {
			skipped_no_package_id += 1;
			tracing::debug!(
				package_hash = %bundle_pkg.package_hash,
				"Skipping package: missing package_id"
			);
			continue;
		};
		let Some(ref installed_version_id) = bundle_pkg.bundle_version_id else {
			skipped_no_version_id += 1;
			tracing::debug!(
				package_hash = %bundle_pkg.package_hash,
				package_id = %pkg_id,
				"Skipping package: missing bundle_version_id"
			);
			continue;
		};

		if let Some((bundle_name, new_version_id, new_file)) = bundle_versions.get(pkg_id) {
			tracing::debug!(
				package_id = %pkg_id,
				installed_version = %installed_version_id,
				bundle_version = %new_version_id,
				bundle_name = %bundle_name,
				"Checking bundle package for updates"
			);

			if installed_version_id != new_version_id {
				tracing::info!(
					package_id = %pkg_id,
					installed_version = %installed_version_id,
					bundle_version = %new_version_id,
					bundle_name = %bundle_name,
					"Update available for bundle package"
				);
				updates_available.push(BundlePackageUpdate {
					cluster_id,
					installed_package_hash: bundle_pkg.package_hash.clone(),
					installed_version_id: installed_version_id.clone(),
					bundle_name: bundle_name.clone(),
					new_version_id: new_version_id.clone(),
					new_file: new_file.clone(),
					installed_at: bundle_pkg.installed_at.unwrap_or_else(Utc::now),
				});
			} else {
				tracing::debug!(
					package_id = %pkg_id,
					version = %installed_version_id,
					"Bundle package is up to date"
				);
			}
		} else {
			not_in_bundle += 1;
			tracing::debug!(
				package_id = %pkg_id,
				installed_version = %installed_version_id,
				bundle_name = ?bundle_pkg.bundle_name,
				"Package not found in any current bundle (may have been removed from bundle)"
			);
		}
	}

	tracing::info!(
		cluster_id = %cluster_id,
		total_packages_checked = %bundle_packages.len(),
		updates_found = %updates_available.len(),
		skipped_no_package_id = %skipped_no_package_id,
		skipped_no_version_id = %skipped_no_version_id,
		not_in_bundle = %not_in_bundle,
		"Bundle update check completed"
	);

	Ok(BundleUpdateCheckResult {
		cluster_id,
		updates_available,
		checked_at: Utc::now(),
	})
}

pub async fn get_bundles_with_update_status(
	cluster_id: ClusterId,
) -> LauncherResult<Vec<BundleWithUpdateStatus>> {
	let cluster = onelauncher_core::api::cluster::dao::get_cluster_by_id(cluster_id)
		.await?
		.ok_or_else(|| {
			onelauncher_core::error::LauncherError::from(anyhow::anyhow!(
				"cluster with id {} not found",
				cluster_id
			))
		})?;

	let bundle_packages = bundle_dao::get_bundle_packages_for_cluster(cluster_id).await?;

	let installed_map: std::collections::HashMap<String, &cluster_packages::Model> =
		bundle_packages
			.iter()
			.filter_map(|bp| bp.package_id.as_ref().map(|pid| (pid.clone(), bp)))
			.collect();

	let bundles = BundlesManager::get()
		.await
		.get_bundles_for(&cluster.mc_version, cluster.mc_loader)
		.await?;

	let mut results = Vec::new();

	for bundle in bundles {
		let mut files_with_status = Vec::new();
		let mut has_updates = false;

		for file in &bundle.manifest.files {
			let update_status = match &file.kind {
				ModpackFileKind::Managed((pkg, version)) => {
					if let Some(installed) = installed_map.get(&pkg.id) {
						let installed_version =
							installed.bundle_version_id.as_deref().unwrap_or("");
						if installed_version != version.version_id {
							has_updates = true;
							FileUpdateStatus::UpdateAvailable {
								installed_version_id: installed_version.to_string(),
								new_version_id: version.version_id.clone(),
							}
						} else {
							FileUpdateStatus::UpToDate
						}
					} else {
						FileUpdateStatus::NotInstalled
					}
				}
				ModpackFileKind::External(ext) => {
					if bundle_packages.iter().any(|bp| bp.package_hash == ext.sha1) {
						FileUpdateStatus::UpToDate
					} else {
						FileUpdateStatus::NotInstalled
					}
				}
			};

			files_with_status.push(FileWithUpdateStatus {
				file: file.clone(),
				status: update_status,
			});
		}

		results.push(BundleWithUpdateStatus {
			bundle,
			files: files_with_status,
			has_updates,
		});
	}

	Ok(results)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub enum FileUpdateStatus {
	NotInstalled,
	UpToDate,
	UpdateAvailable {
		installed_version_id: String,
		new_version_id: String,
	},
}

#[taurpc::ipc_type]
pub struct FileWithUpdateStatus {
	pub file: ModpackFile,
	pub status: FileUpdateStatus,
}

#[taurpc::ipc_type]
pub struct BundleWithUpdateStatus {
	pub bundle: ModpackArchive,
	pub files: Vec<FileWithUpdateStatus>,
	pub has_updates: bool,
}

async fn apply_single_update(
	update: &BundlePackageUpdate,
) -> LauncherResult<onelauncher_core::entity::packages::Model> {
	tracing::info!(
		cluster_id = %update.cluster_id,
		package_hash = %update.installed_package_hash,
		bundle_name = %update.bundle_name,
		old_version = %update.installed_version_id,
		new_version = %update.new_version_id,
		"Applying bundle package update"
	);

	let cluster = api::cluster::dao::get_cluster_by_id(update.cluster_id)
		.await?
		.ok_or_else(|| anyhow::anyhow!("cluster with id {} not found", update.cluster_id))?;

	tracing::debug!(
		package_hash = %update.installed_package_hash,
		"Removing old package"
	);
	api::packages::remove_package(update.cluster_id, update.installed_package_hash.clone()).await?;

	tracing::debug!(
		package_hash = %update.installed_package_hash,
		"Removing old bundle tracking"
	);
	let _ = api::packages::bundle_dao::remove_bundle_package_tracking(
		update.cluster_id,
		&update.installed_package_hash,
	)
	.await;

	match &update.new_file.kind {
		ModpackFileKind::Managed((pkg, version)) => {
			tracing::debug!(
				package_id = %pkg.id,
				version_id = %version.version_id,
				"Downloading new managed package version"
			);
			let model = api::packages::download_package(pkg, version, None, None).await?;

			tracing::debug!(
				package_hash = %model.hash,
				"Linking new package to cluster"
			);
			api::packages::link_package(&model, &cluster, Some(true)).await?;

			tracing::debug!(
				package_hash = %model.hash,
				bundle_name = %update.bundle_name,
				"Tracking new package as bundle package"
			);
			api::packages::bundle_dao::track_bundle_package(
				&cluster,
				&model,
				&update.bundle_name,
				&version.version_id,
			)
			.await?;

			tracing::info!(
				package_id = %pkg.id,
				new_hash = %model.hash,
				"Successfully updated managed package"
			);
			Ok(model)
		}
		ModpackFileKind::External(ext_package) => {
			tracing::debug!(
				url = %ext_package.url,
				sha1 = %ext_package.sha1,
				"Downloading new external package version"
			);
			let model = api::packages::download_external_package(
				ext_package,
				&cluster,
				None,
				Some(true),
				None,
			)
			.await?
			.ok_or_else(|| anyhow::anyhow!("Failed to download external package"))?;

			tracing::debug!(
				package_hash = %model.hash,
				"Linking new external package to cluster"
			);
			api::packages::link_package(&model, &cluster, Some(true)).await?;

			tracing::debug!(
				package_hash = %model.hash,
				bundle_name = %update.bundle_name,
				"Tracking new external package as bundle package"
			);
			api::packages::bundle_dao::track_bundle_package(
				&cluster,
				&model,
				&update.bundle_name,
				&ext_package.sha1,
			)
			.await?;

			tracing::info!(
				url = %ext_package.url,
				new_hash = %model.hash,
				"Successfully updated external package"
			);
			Ok(model)
		}
	}
}

pub async fn apply_bundle_updates(
	cluster_id: ClusterId,
) -> LauncherResult<Vec<BundlePackageUpdate>> {
	let check_result = check_bundle_updates(cluster_id).await?;

	let mut applied_updates = Vec::new();

	for update in check_result.updates_available {
		match apply_single_update(&update).await {
			Ok(_) => {
				applied_updates.push(update);
			}
			Err(e) => {
				send_error!("Failed to update bundle package: {}", e);
			}
		}
	}

	Ok(applied_updates)
}
