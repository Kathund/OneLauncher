import type { PlayerAnimation } from 'skinview3d';
import { DefaultRoation, Overlay, SettingDropdown, SettingSwitch, SheetPage, SkinViewer } from '@/components';
import { bindings } from '@/main';
import { getSkinUrl } from '@/utils/minecraft';
import { useToast } from '@/utils/toast';
import { Button, TextField, Tooltip, WingsFilledIcon, WingsIcon } from '@onelauncher/common/components';
import { createFileRoute } from '@tanstack/react-router';
import { dataDir, downloadDir, join } from '@tauri-apps/api/path';
import { save } from '@tauri-apps/plugin-dialog';
import { exists, mkdir, readTextFile, writeFile, writeTextFile } from '@tauri-apps/plugin-fs';
import { Download01Icon, PauseSquareIcon, PlaySquareIcon, PlusIcon, Trash01Icon } from '@untitled-theme/icons-react';
import { OverlayScrollbarsComponent } from 'overlayscrollbars-react';
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { CrouchAnimation, FlyingAnimation, HitAnimation, IdleAnimation, WalkingAnimation } from 'skinview3d';

interface Skin {
	slim: boolean | null;
	url: string;
}

interface Cape {
	url: string;
	id: string;
}

export const Route = createFileRoute('/app/account/skins')({
	component: RouteComponent,
});

interface Animation {
	name: string;
	animation: PlayerAnimation;
	speed: number;
}

const animations: Array<Animation> = [
	{ name: 'Idle', animation: new IdleAnimation(), speed: 0.1 },
	{ name: 'Walking', animation: new WalkingAnimation(), speed: 0.2 },
	{ name: 'Flying', animation: new FlyingAnimation(), speed: 0.2 },
	{ name: 'Crouch', animation: new CrouchAnimation(), speed: 0.025 },
	{ name: 'Hit', animation: new HitAnimation(), speed: 0.2 },
];

interface SkinHistoryApi {
	skins: Array<Skin>;
	addSkin: (newSkin: Skin) => void;
	removeSkin: (newSkin: Skin) => void;
}

interface SkinRenderApi extends SkinHistoryApi {
	skin: Skin;
	slim: boolean;
	setSlim: React.Dispatch<React.SetStateAction<boolean>>;
	setSkin: React.Dispatch<React.SetStateAction<Skin>>;
	capes: Array<Cape>;
	cape: Cape;
	setCape: React.Dispatch<React.SetStateAction<Cape>>;
	animation?: PlayerAnimation;
	animationPlaying?: boolean;
	shouldShowElytra?: boolean;
}

const SkinRenderContext = createContext<SkinRenderApi | undefined>(undefined);
function useSkinRenderContext() {
	const ctx = useContext(SkinRenderContext);

	if (!ctx)
		throw new Error('useSkinRenderContext must be used within a SkinRenderContext.Provider');

	return ctx;
}

export function MissingAccountData({ validSearch }: { validSearch: boolean }) {
	return (
		<SheetPage headerLarge={<></>} headerSmall={<></>}>
			<h1>
				{validSearch
					? 'Please select an account before going to the skins page'
					: 'Missing Profile auth. Please log out and log back in'}
			</h1>
		</SheetPage>
	);
}

async function getSkinHistory(): Promise<Array<Skin>> {
	const parentDir = await join(await dataDir(), 'OneClient', 'metadata', 'history');
	const skinsPath = await join(parentDir, 'skins.json');
	try {
		const dirExists = await exists(parentDir);
		if (!dirExists)
			await mkdir(parentDir, { recursive: true });
		const fileExists = await exists(skinsPath);
		if (!fileExists) {
			await writeTextFile(skinsPath, JSON.stringify([]));
			return [];
		}
		const contents = await readTextFile(skinsPath);
		return JSON.parse(contents) as Array<Skin>;
	}
	catch (error) {
		console.error(error);
		await writeTextFile(skinsPath, JSON.stringify([]));
		return [];
	}
}

async function saveSkinHistory(skins: Array<Skin>): Promise<void> {
	const parentDir = await join(await dataDir(), 'OneClient', 'metadata', 'history');
	const skinsPath = await join(parentDir, 'skins.json');
	try {
		const dirExists = await exists(parentDir);
		if (!dirExists)
			await mkdir(parentDir, { recursive: true });
		await writeTextFile(skinsPath, JSON.stringify(skins));
	}
	catch (error) {
		console.error(error);
	}
}

function useSkinHistory(): SkinHistoryApi {
	const [skins, setSkins] = useState<Array<Skin>>([]);
	const [loaded, setLoaded] = useState(false);
	const pendingSkinsRef = useRef<Array<Skin>>([]);

	useEffect(() => {
		async function loadHistory() {
			const history = await getSkinHistory();
			const merged = [...pendingSkinsRef.current, ...history].filter(
				(skin, index, array) => array.findIndex(x => x.url === skin.url) === index,
			);
			setSkins(merged);
			await saveSkinHistory(merged);

			pendingSkinsRef.current = [];
			setLoaded(true);
		}
		loadHistory();
	}, []);

	const addSkin = useCallback(
		(newSkin: Skin) => {
			if (!loaded) {
				pendingSkinsRef.current.push(newSkin);
				return;
			}

			setSkins((prevSkins) => {
				const exists = prevSkins.some(s => s.url === newSkin.url);
				if (exists)
					return prevSkins;

				const updatedSkins = [newSkin, ...prevSkins];
				saveSkinHistory(updatedSkins).catch(console.error);
				return updatedSkins;
			});
		},
		[loaded],
	);

	const removeSkin = useCallback((skin: Skin) => {
		setSkins((prevSkins) => {
			const updatedSkins = prevSkins.filter(s => s.url !== skin.url);
			saveSkinHistory(updatedSkins).catch(console.error);
			return updatedSkins;
		});
	}, []);

	return {
		skins,
		addSkin,
		removeSkin,
	};
}

function RouteComponent() {
	const { profileData, validSearch, playerData } = Route.useRouteContext();

	const [shouldShowElytra, setShouldShowElytra] = useState<boolean>(false);
	const [animationPlaying, setAnimationPlaying] = useState<boolean>(true);
	const [animation, setAnimation] = useState<Animation>(() => animations[0]);
	const selectAnimationDropDown = (value: string) => {
		const foundAnimation = animations.find(animation => animation.name === value) ?? animations[0];
		if (foundAnimation.name === 'Walking')
			(foundAnimation.animation as WalkingAnimation).headBobbing = false;
		foundAnimation.animation.speed = foundAnimation.speed;
		setAnimation(foundAnimation);
	};

	const [cape, setCape] = useState<Cape>({ url: '', id: '' });
	const capes = useMemo<Array<Cape>>(() => {
		if (!profileData)
			return [{ url: '', id: '' }];

		return [{ url: '', id: '' }, ...profileData.capes.map(cape => ({ url: cape.url, id: cape.id }))];
	}, [profileData]);

	const { skins, addSkin, removeSkin } = useSkinHistory();

	const [skin, setSkin] = useState<Skin>(() => ({
		slim: profileData?.skins[0].variant === 'slim',
		url: getSkinUrl(profileData?.skins[0].url),
	}));

	useEffect(() => {
		if (profileData?.skins[0]) {
			const newSkin = {
				slim: profileData.skins[0].variant === 'slim',
				url: getSkinUrl(profileData.skins[0].url),
			};
			setSkin(newSkin);
			addSkin(newSkin);
		}

		if (playerData?.cape_url) {
			const foundCape = capes.find(cape => cape.url === playerData.cape_url);
			if (foundCape)
				setCape(foundCape);
		}
	}, [profileData?.skins, addSkin, playerData?.cape_url, capes]);
	const [slim, setSlim] = useState<boolean>(skin.slim ?? false);

	const context = useMemo<SkinRenderApi>(
		() => ({
			skins,
			slim,
			setSlim,
			addSkin,
			removeSkin,
			skin,
			setSkin,
			capes,
			cape,
			setCape,
			shouldShowElytra,
			animation: animation.animation,
			animationPlaying,
		}),
		[skins, slim, setSlim, addSkin, removeSkin, skin, capes, cape, shouldShowElytra, animation, animationPlaying],
	);

	if (profileData === null)
		return <MissingAccountData validSearch={validSearch} />;

	return (
		<SkinRenderContext.Provider value={context}>
			<SheetPage headerLarge={<HeaderLarge />} headerSmall={<HeaderLarge />}>
				<SheetPage.Content>
					<div className="flex-1 flex flex-row gap-8">
						<div className="flex flex-col justify-center items-center">
							<div className="flex flex-col justify-center items-center gap-2">
								<p>Current Skin</p>
								<SettingDropdown
									options={animations.map(animation => ({
										key: animation.name,
										label: `${animation.name} Animation`,
									}))}
									setting={[animation.name, selectAnimationDropDown]}
								/>
							</div>

							<div className="relative">
								<Viewer cape={cape} enableControls skin={skin} />

								<div className="absolute bottom-4 left-0 w-full">
									<div className="flex flex-row justify-between items-center">
										<Tooltip text={`${shouldShowElytra ? 'Hide' : 'Show'} Elytra`}>
											<Button
												className="w-12 h-12"
												color="ghost"
												onPress={() => setShouldShowElytra(prev => !prev)}
												size="icon"
											>
												{shouldShowElytra ? <WingsFilledIcon className="w-8 h-8" /> : <WingsIcon className="w-8 h-8" />}
											</Button>
										</Tooltip>

										<div className="flex flex-row items-center gap-2">
											<p>Slim</p>
											<SettingSwitch setting={[slim, (value: boolean) => setSlim(value)]} />
										</div>

										<Tooltip text={`${animationPlaying ? 'Pause' : 'Play'} Animation`}>
											<Button
												className="w-12 h-12"
												color="ghost"
												onPress={() => setAnimationPlaying(prev => !prev)}
												size="icon"
											>
												{animationPlaying
													? <PauseSquareIcon className="w-8 h-8" />
													: <PlaySquareIcon className="w-8 h-8" />}
											</Button>
										</Tooltip>
									</div>
								</div>
							</div>
						</div>

						<div className="min-h-full w-px bg-component-border"></div>

						<div className="w-full flex flex-col min-h-full justify-between overflow-hidden">
							<Skins />
							<div className="min-w-full h-px bg-component-border"></div>
							<div className="min-w-full h-px bg-component-border"></div>
							<Capes />
						</div>
					</div>
				</SheetPage.Content>
			</SheetPage>
		</SkinRenderContext.Provider>
	);
}

function ImportSkinModal() {
	const { setSkin, skins, addSkin, setSlim } = useSkinRenderContext();
	const toast = useToast();
	const [input, setInput] = useState<string>('');

	const importFromUsername = async () => {
		toast({ type: 'info', title: 'Import Skin', message: `Importing skin from ${input}` });
		const { id } = await bindings.core.convertUsernameUUID(input);
		if (id === '')
			return toast({ type: 'error', title: 'Import Skin', message: `${input} doesn't exist` });
		const playerProfile = await bindings.core.fetchMinecraftProfile(id);
		if (playerProfile.skin_url) {
			const skin: Skin = { slim: playerProfile.is_slim, url: getSkinUrl(playerProfile.skin_url) };
			if (skins.includes(skin))
				return toast({ type: 'error', title: 'Import Skin', message: 'Skin already exists' });

			addSkin(skin);
			setSkin(skin);
			setSlim(skin.slim ?? false);
			toast({ type: 'success', title: 'Import Skin', message: `Imported skin from ${input}` });
		}
	};

	const importFromURL = () => {
		const skin: Skin = { slim: null, url: getSkinUrl(input) };
		if (skins.includes(skin))
			return toast({ type: 'error', title: 'Import Skin', message: 'Skin already exists' });

		addSkin(skin);
		setSkin(skin);
		toast({ type: 'success', title: 'Import Skin', message: 'Imported skin' });
	};

	return (
		<Overlay.Dialog>
			<Overlay.Title>Import</Overlay.Title>
			<TextField className="w-full" onChange={e => setInput(e.target.value)} />

			<div className="flex flex-row gap-4 h-1/2 w-full">
				<Button
					className="w-1/2"
					color="primary"
					onPress={importFromUsername}
					size="normal"
					slot="close"
				>
					From Username
				</Button>
				<Button
					className="w-1/2"
					color="primary"
					onPress={importFromURL}
					size="normal"
					slot="close"
				>
					From URL
				</Button>
			</div>
		</Overlay.Dialog>
	);
}

function Skins() {
	const { skin: currentSkin, skins, setSkin, setSlim } = useSkinRenderContext();
	return (
		<div className="flex flex-col h-full justify-around w-10/12">
			<div className="flex flex-col justify-center items-center">
				<p>Skin History</p>
			</div>

			<OverlayScrollbarsComponent>
				<div className="flex flex-row h-fit gap-2">
					<Tooltip text="Add Skin">
						<Overlay.Trigger>
							<Button
								className="w-[75px] h-[120px] border rounded-xl bg-component-border hover:border-brand border-component-border"
								color="ghost"
							>
								<div className="flex flex-col justify-center items-center content-center h-full">
									<PlusIcon className="scale-200" />
								</div>
							</Button>
							<Overlay>
								<ImportSkinModal />
							</Overlay>
						</Overlay.Trigger>
					</Tooltip>
					{skins.map(skin => (
						<TinySkin
							allowRemoveal={currentSkin.url !== skin.url}
							key={skin.url}
							onPress={({ skin }: TinySkinOnPressProps) => {
								if (skin) {
									setSkin(skin);
									setSlim(skin.slim ?? false);
								}
							}}
							selected={currentSkin.url === skin.url}
							skin={skin}
						/>
					))}
					<div className="w-4 shrink-0" />
				</div>
			</OverlayScrollbarsComponent>
		</div>
	);
}

function Capes() {
	const { cape: currentCape, capes, setCape } = useSkinRenderContext();
	return (
		<div className="flex flex-col h-full justify-around w-10/12">
			<OverlayScrollbarsComponent>
				<div className="flex flex-row h-fit gap-2">
					{capes.map(cape => (
						<TinySkin
							cape={cape}
							flip
							key={cape.id}
							onPress={({ cape }: TinySkinOnPressProps) => {
								if (cape)
									setCape(cape);
							}}
							selected={currentCape.id === cape.id}
						/>
					))}
					<div className="w-4 shrink-0" />
				</div>
			</OverlayScrollbarsComponent>

			<div className="flex flex-col justify-center items-center">
				<p>Capes</p>
			</div>
		</div>
	);
}

function RemoveSkinCapeModal({ skin }: { skin: Skin }) {
	const { removeSkin } = useSkinRenderContext();
	return (
		<Overlay.Dialog>
			<Overlay.Title>Are you sure?</Overlay.Title>

			<p>This cannot be undone</p>

			<Button
				className="w-full"
				color="danger"
				onPress={() => removeSkin(skin)}
				size="large"
				slot="close"
			>
				Remove
			</Button>
		</Overlay.Dialog>
	);
}

interface TinySkinOnPressProps {
	skin?: Skin;
	cape?: Cape;
}

interface TinySkinProps extends TinySkinOnPressProps {
	selected?: boolean;
	flip?: boolean;
	onPress?: ({ skin, cape }: TinySkinOnPressProps) => void;
	allowRemoveal?: boolean;
}
function TinySkin({ skin, cape, flip, selected = false, onPress, allowRemoveal = false }: TinySkinProps) {
	const onAction = () => {
		if (onPress !== undefined)
			onPress({ skin, cape });
	};

	const exportSkin = async () => {
		try {
			if (skin === undefined)
				return;

			const filePath = await save({
				title: 'Skin Export Location',
				filters: [{ name: 'Images', extensions: ['png'] }],
				defaultPath: await join(await downloadDir(), `${skin.url.split('/').reverse()[0]}.png`),
			});

			if (!filePath)
				return;

			const response = await fetch(skin.url);
			const buffer = await response.arrayBuffer();
			await writeFile(filePath, new Uint8Array(buffer));
		}
		catch (error) {
			console.error(error);
		}
	};

	return (
		<Button
			className={`w-[75px] h-[120px] relative border rounded-xl bg-component-border ${selected ? 'border-brand' : 'hover:border-brand border-component-border'}`}
			color="ghost"
			onPress={onAction}
		>
			<Viewer
				cape={cape}
				flip={flip}
				height={120}
				showText={false}
				skin={skin}
				slim={skin?.slim}
				width={75}
			/>
			{skin && (
				<Tooltip text="Export Skin">
					<Button
						className="group w-8 h-8 absolute bottom-0 left-0"
						color="ghost"
						onPress={exportSkin}
						size="icon"
					>
						<Download01Icon className="group-hover:stroke-brand" />
					</Button>
				</Tooltip>
			)}
			{skin && allowRemoveal && (
				<Tooltip text="Remove Skin">
					<Overlay.Trigger>
						<Button className="group w-8 h-8 absolute bottom-0 right-0" color="ghost" size="icon">
							<Trash01Icon className="group-hover:stroke-danger" />
						</Button>

						<Overlay>
							<RemoveSkinCapeModal skin={skin} />
						</Overlay>
					</Overlay.Trigger>
				</Tooltip>
			)}
		</Button>
	);
}

function HeaderLarge() {
	const toast = useToast();
	const { profileData, profile, queryClient } = Route.useRouteContext();
	const { skin, cape } = useSkinRenderContext();

	const save = async () => {
		try {
			if (!profile)
				return;
			await bindings.core.changeSkin(profile.access_token, skin.url, skin.slim ? 'slim' : 'classic');
			if (cape.id === '')
				await bindings.core.removeCape(profile.access_token);
			else await bindings.core.changeCape(profile.access_token, cape.id);

			queryClient.invalidateQueries({ queryKey: ['getDefaultUser'] });
			queryClient.invalidateQueries({ queryKey: ['fetchLoggedInProfile'] });
			queryClient.invalidateQueries({ queryKey: ['fetchMinecraftProfile'] });
			toast({ type: 'success', title: 'Changed Skin' });
		}
		catch (error) {
			console.error(error);
		}
	};
	return (
		<div className="flex flex-row justify-between items-end gap-16">
			<h1 className="text-3xl font-semibold">{`${profileData?.username ?? 'UNKNOWN'}'s Skins`}</h1>
			<Tooltip text="Save skin to the account">
				<Button color="primary" onClick={save} size="normal">
					<p>Save</p>
				</Button>
			</Tooltip>
		</div>
	);
}

interface ViewerProps {
	skin?: Skin;
	cape?: Cape | null;
	height?: number;
	width?: number;
	showText?: boolean;
	enableControls?: boolean;
	flip?: boolean;
	slim?: boolean | null;
}

function Viewer({
	skin,
	cape,
	height = 400,
	width = 250,
	showText = true,
	enableControls = false,
	flip = false,
	slim,
}: ViewerProps) {
	const {
		animation,
		animationPlaying,
		shouldShowElytra,
		skin: currentSkin,
		cape: currentCape,
		capes,
		slim: isSlim,
	} = useSkinRenderContext();
	const rotation = flip ? { ...DefaultRoation, x: -DefaultRoation.x, z: -DefaultRoation.z } : DefaultRoation;
	if (skin === undefined)
		skin = currentSkin;
	if (cape === undefined)
		cape = currentCape;
	if (cape === null)
		cape = capes[0];
	if (slim === undefined || slim === null)
		slim = isSlim;
	return (
		<SkinViewer
			animate={animationPlaying}
			animation={animation}
			autoRotate={false}
			capeUrl={cape.url !== '' ? cape.url : null}
			className="h-full w-full max-w-1/4"
			elytra={shouldShowElytra}
			enableDamping={enableControls}
			enablePan={enableControls}
			enableRotate={enableControls}
			enableZoom={enableControls}
			height={height}
			isSlim={slim}
			playerRotation={rotation}
			showText={showText}
			skinUrl={skin.url}
			width={width}
			zoom={0.8}
		/>
	);
}
