use std::collections::HashMap;
use std::path::PathBuf;

use onelauncher_core::api::cluster::dao::ClusterId;
use onelauncher_core::api::packages::modpack::data::ModpackArchive;
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
/// the version that was installed. This allows us to detect when a bundle
/// has been updated in the DataStorage repo and seamlessly re-apply it.
#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct InstalledBundlesState {
	/// Map of "cluster_id:bundle_name" -> installed version string
	pub installed: HashMap<String, String>,
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

	/// Records that a bundle was installed for a given cluster
	pub async fn record_installed_bundle(
		&self,
		cluster_id: ClusterId,
		bundle: &ModpackArchive,
	) -> LauncherResult<()> {
		let key = InstalledBundlesState::key(cluster_id, &bundle.manifest.name);
		let mut state = self.installed_state.write().await;
		state
			.installed
			.insert(key, bundle.manifest.version.clone());
		drop(state);
		self.save_installed_state().await
	}

	/// Checks all clusters for bundle updates and re-installs any that have changed.
	/// This is called on startup to seamlessly keep bundles up to date.
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

				let installed_version = {
					let state = self.installed_state.read().await;
					state.installed.get(&key).cloned()
				};

				let Some(installed_version) = installed_version else {
					continue;
				};

				if installed_version == bundle.manifest.version {
					tracing::debug!(
						"bundle '{}' for cluster '{}' is up to date (version {})",
						bundle.manifest.name,
						cluster.folder_name,
						installed_version
					);
					continue;
				}

				tracing::info!(
					"updating bundle '{}' for cluster '{}': {} -> {}",
					bundle.manifest.name,
					cluster.folder_name,
					installed_version,
					bundle.manifest.version
				);

				if let Err(e) = bundle
					.format
					.install_modpack_archive(bundle, cluster, Some(true), None)
					.await
				{
					tracing::error!(
						"failed to update bundle '{}' for cluster '{}': {e}",
						bundle.manifest.name,
						cluster.folder_name
					);
					continue;
				}

				// Update the tracked version
				{
					let mut state = self.installed_state.write().await;
					state
						.installed
						.insert(key, bundle.manifest.version.clone());
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
		}

		Ok(())
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
