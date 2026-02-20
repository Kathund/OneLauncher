import type { ExternalPackage, ManagedVersionDependency, ModpackArchive, ModpackFile, Provider } from '@/bindings.gen';
import { getModMetaDataName, Overlay } from '@/components';
import { useSettings } from '@/hooks/useSettings';
import { bindings } from '@/main';
import { useCommandMut } from '@onelauncher/common';
import { Button } from '@onelauncher/common/components';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useImperativeHandle, useState } from 'react';
import { DialogTrigger } from 'react-aria-components';

export interface DownloadModsRef {
	openDownloadDialog: (nextPath?: string) => void;
}

export interface BaseModData {
	name: string;
	clusterId: number;
	managed: boolean;
}

export interface ManagedModData extends BaseModData {
	provider: Provider;
	id: string;
	versionId: string;
	dependencies: Array<ManagedVersionDependency>;
}

export interface ExternalModData extends BaseModData {
	package: ExternalPackage;
}

export type ModData = ManagedModData | ExternalModData;
export type ModDataArray = Array<ModData>;

export function isManagedMod(mod: ModData): mod is ManagedModData {
	return mod.managed === true;
}

export function DownloadMods({ modsPerCluster, bundlesPerCluster, ref }: { modsPerCluster: Record<string, Array<ModpackFile>>; bundlesPerCluster: Record<string, Array<ModpackArchive>>; ref: React.Ref<DownloadModsRef> }) {
	const navigate = useNavigate();
	const [isOpen, setOpen] = useState<boolean>(false);
	const [mods, setMods] = useState<ModDataArray>([]);
	const [nextPath, setNextPath] = useState<string>('/app');

	useEffect(() => {
		const modsList: ModDataArray = [];
		for (const [clusterId, mods] of Object.entries(modsPerCluster))
			for (const mod of mods) {
				if ('External' in mod.kind)
					modsList.push({
						name: getModMetaDataName(mod),
						clusterId: Number(clusterId),
						managed: false,
						package: mod.kind.External,
					});

				if ('Managed' in mod.kind) {
					const [pkg, version] = mod.kind.Managed;
					modsList.push({
						name: getModMetaDataName(mod),
						clusterId: Number(clusterId),
						managed: true,
						provider: pkg.provider,
						id: pkg.id,
						versionId: version.version_id,
						dependencies: version.dependencies,
					});
				}
			}
		setMods(modsList);
	}, [modsPerCluster]);

	useImperativeHandle(ref, () => {
		return {
			openDownloadDialog(nextPath?: string) {
				if (mods.length !== 0) {
					setOpen(true);
					setNextPath(nextPath ?? '/app');
				}
				else {
					navigate({ to: nextPath ?? '/app' });
				}
			},
		};
	}, [mods.length, navigate]);

	return (
		<DialogTrigger>
			<Button className="mb-4" isDisabled={mods.length === 0} onPress={() => setOpen(prev => !prev)}>Download Mods</Button>

			<Overlay isDismissable={false} isOpen={isOpen}>
				<DownloadingMods
					bundlesPerCluster={bundlesPerCluster}
					mods={mods}
					nextPath={nextPath}
					setOpen={setOpen}
				/>
			</Overlay>
		</DialogTrigger>
	);
}

function downloadModsParallel(items: ModDataArray, limit: number, fn: (mod: ModData, index: number) => Promise<void>) {
	let index = 0;
	const workers = Array.from({ length: limit }).fill(null).map(async () => {
		while (index < items.length) {
			const i = index++;
			await fn(items[i], i);
		}
	});
	return Promise.all(workers);
}

function DownloadingMods({ mods, bundlesPerCluster, setOpen, nextPath }: { mods: ModDataArray; bundlesPerCluster: Record<string, Array<ModpackArchive>>; setOpen: React.Dispatch<React.SetStateAction<boolean>>; nextPath: string }) {
	const navigate = useNavigate();
	const [downloadedMods, setDownloadedMods] = useState(0);
	const [totalItems, setTotalItems] = useState(0);
	const [modName, setModName] = useState<string | null>(null);
	const download = useCommandMut(async (mod: ModData) => {
		if (isManagedMod(mod)) {
			if (mod.dependencies.length > 0)
				for (const dependency of mod.dependencies) {
					const cluster = await bindings.core.getClusterById(mod.clusterId);
					if (!cluster)
						continue;
					if (dependency.dependency_type === 'required') {
						const slug = dependency.project_id ?? '';
						const versions = await bindings.core.getPackageVersions(mod.provider, slug, cluster.mc_version, cluster.mc_loader, 0, 1);
						if (versions.items.length !== 0)
							await bindings.core.downloadPackage(mod.provider, slug, versions.items[0].version_id, cluster.id, null);
					}
				}
			await bindings.core.downloadPackage(mod.provider, mod.id, mod.versionId, mod.clusterId, true);
		}
		else { await bindings.core.downloadExternalPackage(mod.package, mod.clusterId, null, null); }
	});

	const { setting } = useSettings();
	let useParallelModDownloading = setting('parallel_mod_downloading');

	useEffect(() => {
		const downloadAll = async () => {
			let remainingMods = [...mods];
			const bundlesToInstall: Array<{ bundle: ModpackArchive; clusterId: number }> = [];

			function modMatchesFile(mod: ModData, file: ModpackFile, clusterId: number): boolean {
				if (mod.clusterId !== clusterId)
					return false;
				if (mod.managed) {
					if ('Managed' in file.kind) {
						const [pkg, version] = file.kind.Managed;
						return (
							mod.provider === pkg.provider
							&& mod.id === pkg.id
							&& mod.versionId === version.version_id
						);
					}
					return false;
				}
				else {
					if ('External' in file.kind)
						return mod.package.sha1 === file.kind.External.sha1;

					return false;
				}
			}

			for (const [clusterIdStr, bundles] of Object.entries(bundlesPerCluster)) {
				const clusterId = Number(clusterIdStr);
				for (const bundle of bundles) {
					const enabledFiles = bundle.manifest.files.filter(f => f.enabled);
					const anyIncluded = enabledFiles.some(f =>
						remainingMods.some(m => modMatchesFile(m, f, clusterId)));
					if (anyIncluded) {
						bundlesToInstall.push({ bundle, clusterId });
						for (const f of enabledFiles) {
							const idx = remainingMods.findIndex(m => modMatchesFile(m, f, clusterId));
							if (idx !== -1)
								remainingMods.splice(idx, 1);
						}
					}
				}
			}

			const items = bundlesToInstall.length + remainingMods.length;
			setTotalItems(items);
			setDownloadedMods(0);

			for (const { bundle, clusterId } of bundlesToInstall) {
				setModName(bundle.manifest.name);
				try {
					await bindings.oneclient.installBundle(bundle, clusterId);
				}
				finally {
					setDownloadedMods(prev => prev + 1);
				}
			}

			if (useParallelModDownloading)
				await downloadModsParallel(remainingMods, 10, async (mod) => {
					setModName(mod.name);
					try {
						await download.mutateAsync(mod);
					}
					finally {
						setDownloadedMods(prev => prev + 1);
					}
				});
			else
				for (const mod of remainingMods) {
					setModName(mod.name);
					try {
						await download.mutateAsync(mod);
					}
					finally {
						setDownloadedMods(prev => prev + 1);
					}
				}
		};

		downloadAll();
	}, [mods, bundlesPerCluster]);

	useEffect(() => {
		if (totalItems > 0 && downloadedMods >= totalItems) {
			setOpen(false);
			navigate({ to: nextPath });
		}
	}, [downloadedMods, totalItems, navigate, nextPath, setOpen]);

	return (
		<Overlay.Dialog isDismissable={false}>
			<Overlay.Title>Downloading Mods</Overlay.Title>

			<div className="w-full flex flex-col items-center gap-2">
				<p>Downloaded {downloadedMods} / {totalItems}</p>
				<div className="w-1/2 h-4 bg-component-bg-disabled rounded-full outline-2 outline-ghost-overlay">
					<div
						className="h-full bg-brand rounded-full transition-all duration-300"
						style={{ width: totalItems > 0 ? `${(downloadedMods / totalItems) * 100}%` : '0%' }}
					>
					</div>
				</div>
				{modName !== null ? <p>Downloading {modName}</p> : <></>}
			</div>

		</Overlay.Dialog>
	);
}
