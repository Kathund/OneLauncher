use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use onelauncher_core::api::cluster::dao::ClusterId;
use onelauncher_core::api::packages::modpack::data::{ModpackArchive, ModpackFileKind};
use onelauncher_core::api::packages::modpack::{InstallableModpackFormatExt, ModpackFormat};
use onelauncher_core::entity::loader::GameLoader;
use onelauncher_core::error::LauncherResult;
use onelauncher_core::send_error;
use onelauncher_core::store::Dirs;
use onelauncher_core::utils::{http, io};
use reqwest::{Method, header};
use tokio::sync::{OnceCell, RwLock};

/// e.g.
/// ```json
/// {
/// 	"versions": {
/// 		"1.21.5": {
/// 			"fabric": ["/generated/hud-fabric-1.21.5.mrpack"],
/// 			"forge": ["/generated/hud-forge-1.21.5.mrpack"]
/// 		}
/// 	},
/// }
/// ```
#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
struct BundleManifest {
	pub versions: HashMap<String, HashMap<String, Vec<String>>>,
}

/// Tracks which bundles have been installed to which clusters, along with
/// the version that was installed and which package hashes belong to the bundle.
/// This allows us to detect updates and apply them without disturbing
/// user configs, user-toggled mod states, or custom-installed mods.
#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct InstalledBundlesState {
	/// Map of "cluster_id:bundle_name" -> bundle install info
	pub installed: HashMap<String, InstalledBundleInfo>,
}

/// Info about a specific bundle installation in a cluster.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct InstalledBundleInfo {
	/// The version of the bundle that was installed
	pub version: String,
	/// The sha1 hashes of all packages that were installed as part of this bundle.
	/// This lets us distinguish bundle packages from user-installed custom mods.
	pub package_hashes: HashSet<String>,
}

impl InstalledBundlesState {
	fn key(cluster_id: ClusterId, bundle_name: &str) -> String {
		format!("{cluster_id}:{bundle_name}")
	}
}

static BUNDLES_STATE: OnceCell<BundlesManager> = OnceCell::const_new();

#[derive(Debug)]
pub struct BundlesManager {
	manifest: RwLock<BundleManifest>,
	bundles: RwLock<HashMap<String, HashMap<GameLoader, Vec<ModpackArchive>>>>,
	installed_state: RwLock<InstalledBundlesState>,
}

impl BundlesManager {
	pub async fn get() -> &'static Self {
		BUNDLES_STATE
			.get_or_init(|| async {
				let manifest = Self::fetch_cached().await;
				let installed_state = Self::load_installed_state().await;

				Self {
					manifest: RwLock::new(manifest),
					bundles: RwLock::new(HashMap::new()),
					installed_state: RwLock::new(installed_state),
				}
			})
			.await
	}

	#[tracing::instrument]
	pub async fn get_bundles_for(
		&self,
		mc_version: &str,
		loader: onelauncher_core::entity::loader::GameLoader,
	) -> LauncherResult<Vec<ModpackArchive>> {
		let manifest = self.manifest.read().await;
		let bundles_lock = self.bundles.read().await;

		if let Some(entry) = bundles_lock.get(mc_version) {
			if let Some(bundles) = entry.get(&loader) {
				return Ok(bundles.clone());
			}
		}

		// drop read lock as we're gonna acquire a write lock this time
		drop(bundles_lock);

		let mut bundles_lock = self.bundles.write().await;

		let mut found = Vec::new();

		for (version, loaders) in &manifest.versions {
			if version != mc_version {
				continue;
			}

			let Some(paths) = loaders.get(&loader.get_format_name()) else {
				continue;
			};

			// we will be first checking the disk cache, if that fails we fetch from remote
			for path in paths {
				let Some(file_name) = path.split('/').last() else {
					tracing::error!("no bundle name was found in path: {path}");
					continue;
				};

				let disk_path = BundlesManager::dir().await.join("bundles").join(file_name);

				let modpack = match download_and_load_bundle(path, &disk_path).await {
					Ok(modpack) => modpack,
					Err(e) => {
						tracing::error!("failed to load bundle from {path}: {e}");
						continue;
					}
				};

				let manifest = match modpack.manifest().await {
					Ok(manifest) => manifest,
					Err(e) => {
						tracing::error!("failed to load modpack manifest from {path}: {e}");
						continue;
					}
				}
				.clone();

				found.push(ModpackArchive {
					manifest,
					path: disk_path,
					format: modpack.kind(),
				});
			}
		}

		bundles_lock
			.entry(mc_version.to_string())
			.or_default()
			.insert(loader, found.clone());

		Ok(found.clone())
	}

	/// Fetches the bundles manifest from remote, falling back to a saved copy on disk if available
	#[tracing::instrument]
	pub async fn fetch_cached() -> BundleManifest {
		let url = format!("{}/bundles.json", crate::constants::META_URL_BASE);
		let manifest_path = Self::dir().await.join("bundles.json");

		match http::fetch_json::<BundleManifest>(Method::GET, &url, None, None).await {
			Ok(manifest) => {
				io::create_dir_all(manifest_path.parent().unwrap())
					.await
					.unwrap_or_else(|e| {
						tracing::error!("failed to create bundles dir: {e}");
					});

				if let Err(e) = io::write_json(&manifest_path, &manifest).await {
					send_error!("failed to cache bundles manifest to disk: {e}");
				}

				manifest
			}
			Err(e) if manifest_path.exists() => {
				tracing::debug!(
					"falling back to cached bundles manifest, due to error fetching remote: {e}"
				);

				match io::read_json::<BundleManifest>(&manifest_path).await {
					Ok(manifest) => manifest,
					Err(e) => {
						tracing::error!("failed to read cached bundles manifest: {e}");

						BundleManifest::default()
					}
				}
			}
			Err(e) => {
				tracing::error!("failed to fetch bundles manifest from remote: {e}");

				BundleManifest::default()
			}
		}
	}

	/// returns the directory for everything bundle related
	pub async fn dir() -> std::path::PathBuf {
		Dirs::get_caches_dir()
			.await
			.expect("failed to get caches dir")
			.join("oneclient")
			.join("bundles")
	}

	/// Path to the file tracking which bundles have been installed
	async fn installed_state_path() -> std::path::PathBuf {
		Self::dir().await.join("installed_bundles.json")
	}

	/// Load the installed bundles state from disk
	async fn load_installed_state() -> InstalledBundlesState {
		let path = Self::installed_state_path().await;
		if !path.exists() {
			return InstalledBundlesState::default();
		}
		match io::read_json::<InstalledBundlesState>(&path).await {
			Ok(state) => state,
			Err(e) => {
				tracing::error!("failed to read installed bundles state: {e}");
				InstalledBundlesState::default()
			}
		}
	}

	/// Save the installed bundles state to disk
	async fn save_installed_state(&self) -> LauncherResult<()> {
		let path = Self::installed_state_path().await;
		if let Some(parent) = path.parent() {
			io::create_dir_all(parent).await?;
		}
		let state = self.installed_state.read().await;
		io::write_json(&path, &*state).await?;
		Ok(())
	}

	/// Records that a bundle was installed for a given cluster, tracking
	/// its version and the hashes of all packages that were installed.
	pub async fn record_installed_bundle(
		&self,
		cluster_id: ClusterId,
		bundle: &ModpackArchive,
	) -> LauncherResult<()> {
		let key = InstalledBundlesState::key(cluster_id, &bundle.manifest.name);

		let package_hashes = collect_bundle_package_hashes(bundle);

		let mut state = self.installed_state.write().await;
		state.installed.insert(
			key,
			InstalledBundleInfo {
				version: bundle.manifest.version.clone(),
				package_hashes,
			},
		);
		drop(state);
		self.save_installed_state().await
	}

	/// Checks all clusters for bundle updates and applies them differentially.
	/// Only new or updated packages are installed. User configs (overrides),
	/// user-toggled mod states, and custom-installed mods are preserved.
	#[tracing::instrument(skip(self))]
	pub async fn check_and_apply_bundle_updates(&self) -> LauncherResult<()> {
		let clusters = onelauncher_core::api::cluster::dao::get_all_clusters().await?;

		let state = self.installed_state.read().await;
		if state.installed.is_empty() {
			tracing::debug!("no installed bundles to check for updates");
			return Ok(());
		}
		drop(state);

		for cluster in &clusters {
			let bundles = match self
				.get_bundles_for(&cluster.mc_version, cluster.mc_loader)
				.await
			{
				Ok(b) => b,
				Err(e) => {
					tracing::error!(
						"failed to get bundles for cluster {}: {e}",
						cluster.folder_name
					);
					continue;
				}
			};

			for bundle in &bundles {
				let key = InstalledBundlesState::key(cluster.id, &bundle.manifest.name);

				let installed_info = {
					let state = self.installed_state.read().await;
					state.installed.get(&key).cloned()
				};

				let Some(installed_info) = installed_info else {
					continue;
				};

				if installed_info.version == bundle.manifest.version {
					tracing::debug!(
						"bundle '{}' for cluster '{}' is up to date (version {})",
						bundle.manifest.name,
						cluster.folder_name,
						installed_info.version
					);
					continue;
				}

				tracing::info!(
					"updating bundle '{}' for cluster '{}': {} -> {}",
					bundle.manifest.name,
					cluster.folder_name,
					installed_info.version,
					bundle.manifest.version
				);

				match apply_bundle_update(cluster, bundle, &installed_info).await {
					Ok(new_hashes) => {
						{
							let mut state = self.installed_state.write().await;
							state.installed.insert(
								key,
								InstalledBundleInfo {
									version: bundle.manifest.version.clone(),
									package_hashes: new_hashes,
								},
							);
						}

						if let Err(e) = self.save_installed_state().await {
							tracing::error!("failed to save installed bundles state: {e}");
						}

						tracing::info!(
							"successfully updated bundle '{}' for cluster '{}'",
							bundle.manifest.name,
							cluster.folder_name
						);
					}
					Err(e) => {
						tracing::error!(
							"failed to update bundle '{}' for cluster '{}': {e}",
							bundle.manifest.name,
							cluster.folder_name
						);
					}
				}
			}
		}

		Ok(())
	}
}

/// Collects the sha1 hashes of all enabled packages in a bundle manifest.
fn collect_bundle_package_hashes(bundle: &ModpackArchive) -> HashSet<String> {
	let mut hashes = HashSet::new();
	for file in &bundle.manifest.files {
		if !file.enabled {
			continue;
		}
		match &file.kind {
			ModpackFileKind::Managed((_, version)) => {
				if let Some(primary) = version.files.iter().find(|f| f.primary) {
					hashes.insert(primary.sha1.clone());
				}
			}
			ModpackFileKind::External(ext) => {
				hashes.insert(ext.sha1.clone());
			}
		}
	}
	hashes
}

/// Applies a bundle update differentially:
/// - Only installs packages that are NEW in the updated bundle
/// - Updates packages that changed version, but only if the user still has the
///   old version linked (respects user removals)
/// - Does NOT copy overrides (preserves user configs)
/// - Does NOT touch packages the user installed separately
/// Returns the set of package hashes in the new bundle.
async fn apply_bundle_update(
	cluster: &onelauncher_core::entity::clusters::Model,
	bundle: &ModpackArchive,
	old_info: &InstalledBundleInfo,
) -> LauncherResult<HashSet<String>> {
	let linked_packages =
		onelauncher_core::api::packages::dao::get_linked_packages(cluster).await?;
	let linked_hashes: HashSet<String> = linked_packages.iter().map(|p| p.hash.clone()).collect();

	let new_hashes = collect_bundle_package_hashes(bundle);

	// Old bundle hashes that no longer appear in the new bundle (replaced/removed packages)
	let removed_old_hashes: HashSet<&String> = old_info
		.package_hashes
		.iter()
		.filter(|h| !new_hashes.contains(*h))
		.collect();

	let mut errors = Vec::new();
	let mut packages_to_link = Vec::new();

	for file in &bundle.manifest.files {
		if !file.enabled {
			continue;
		}

		let file_hash = match &file.kind {
			ModpackFileKind::Managed((_, version)) => {
				version.files.iter().find(|f| f.primary).map(|f| &f.sha1)
			}
			ModpackFileKind::External(ext) => Some(&ext.sha1),
		};

		let Some(file_hash) = file_hash else {
			continue;
		};

		// Already linked to the cluster, no action needed
		if linked_hashes.contains(file_hash) {
			continue;
		}

		if old_info.package_hashes.contains(file_hash) {
			// Hash is already tracked but not linked — user removed this mod, skip
			tracing::debug!(
				"skipping bundle package with hash {file_hash} (user previously removed it)"
			);
			continue;
		}

		// This is a new hash not in the old bundle. It's either:
		// 1. A brand new package added to the bundle
		// 2. An updated version replacing an old package
		//
		// For brand new packages: always install.
		// For updated packages: only install if the user still has the old
		// version linked (respects user removals).
		let is_replacing_old = !removed_old_hashes.is_empty();

		if is_replacing_old {
			// Check if the user still has ANY of the removed old bundle packages
			// linked. If they removed all old bundle packages, they likely don't
			// want the bundle mods at all.
			let user_kept_old_mods = removed_old_hashes
				.iter()
				.any(|old_hash| linked_hashes.contains(*old_hash));

			if !user_kept_old_mods {
				tracing::debug!(
					"skipping updated bundle package with hash {file_hash} \
					(user removed old bundle packages)"
				);
				continue;
			}
		}

		// Install this new/updated package
		match download_and_link_file(file, cluster).await {
			Ok(Some(model)) => packages_to_link.push(model),
			Ok(None) => {}
			Err(e) => errors.push(e),
		}
	}

	if !packages_to_link.is_empty() {
		let linked = onelauncher_core::api::packages::link_many_packages_to_cluster(
			&packages_to_link,
			cluster,
			Some(true),
		)
		.await?;
		if linked < packages_to_link.len() as u64 {
			tracing::warn!(
				"only {linked}/{} updated bundle packages could be linked to cluster '{}'",
				packages_to_link.len(),
				cluster.folder_name
			);
		}
	}

	if !errors.is_empty() {
		tracing::warn!(
			"{} errors occurred while applying bundle update for cluster '{}'",
			errors.len(),
			cluster.folder_name
		);
	}

	Ok(new_hashes)
}

/// Downloads a single package from a bundle manifest file entry.
async fn download_and_link_file(
	file: &onelauncher_core::api::packages::modpack::data::ModpackFile,
	cluster: &onelauncher_core::entity::clusters::Model,
) -> LauncherResult<Option<onelauncher_core::entity::packages::Model>> {
	match &file.kind {
		ModpackFileKind::Managed((package, version)) => {
			let model =
				onelauncher_core::api::packages::download_package(package, version, None, None)
					.await?;
			Ok(Some(model))
		}
		ModpackFileKind::External(package) => {
			onelauncher_core::api::packages::download_external_package(
				package,
				cluster,
				None,
				Some(true),
				None,
			)
			.await
		}
	}
}

#[tracing::instrument]
async fn download_and_load_bundle(
	url_path: &str,
	disk_path: &PathBuf,
) -> LauncherResult<Box<dyn InstallableModpackFormatExt>> {
	let url = format!("{}{}", crate::constants::META_URL_BASE, url_path);

	if disk_path.exists() {
		// we check if the remote file is different to the local file
		let res = http::request(Method::HEAD, &url).await?;

		if !res.status().is_success() {
			return Err(anyhow::anyhow!("failed to download bundle from remote: {}", url).into());
		}

		if res.headers().get(reqwest::header::CONTENT_LENGTH).is_none() {
			return Err(anyhow::anyhow!(
				"bundle at {url} missing content-length header, skipping..."
			)
			.into());
		}

		// TODO: check hash if provided in future, for now we check file size :(
		let content_length = res
			.headers()
			.get(header::CONTENT_LENGTH)
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse::<u64>().ok())
			.unwrap_or(0);

		let file_size = io::stat(disk_path).await.map(|m| m.len()).unwrap_or(0);

		tracing::debug!("bundle content length: {content_length}, local file size: {file_size}");
		if content_length == file_size {
			// file is up to date, load from disk
			return Ok(ModpackFormat::from_file(disk_path).await?);
		}
	}

	tracing::debug!("downloading bundle from remote: {url}");
	// if we are at this point, it means we either need to update or download
	http::download(Method::GET, &url, disk_path, None, None).await?;

	Ok(ModpackFormat::from_file(disk_path).await?)
}
