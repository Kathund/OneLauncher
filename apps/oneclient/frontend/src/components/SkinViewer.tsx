import type { ModelType } from 'skinview-utils';
import { getSkinUrl } from '@/utils/minecraft';
import { useEffect, useRef } from 'react';
import * as skinviewer from 'skinview3d';
import { twMerge } from 'tailwind-merge';

export interface Position {
	x: number;
	y: number;
	z: number;
}

export const DefaultRoation: Position = {
	x: -21.100906085965875,
	y: 24.365227617754844,
	z: 36.54784142663224,
};

export interface SkinViewerProps {
	skinUrl?: string | undefined | null;
	capeUrl?: string | undefined | null;
	width?: number;
	height?: number;
	className?: string | undefined;
	autoRotate?: boolean;
	autoRotateSpeed?: number;
	showText?: boolean;
	playerRotation?: Position;
	zoom?: number;
	animate?: boolean;
	animation?: skinviewer.PlayerAnimation;
	enableDamping?: boolean;
	enableZoom?: boolean;
	enableRotate?: boolean;
	enablePan?: boolean;
	elytra?: boolean;
	isSlim?: boolean;
}

const defaultIdleAnimation = new skinviewer.IdleAnimation();

export function SkinViewer({
	skinUrl,
	capeUrl,
	width = 260,
	height = 300,
	className,
	autoRotate = true,
	autoRotateSpeed = 0.25,
	showText = true,
	playerRotation,
	zoom = 0.9,
	animate = false,
	animation = defaultIdleAnimation,
	enableDamping = true,
	enableZoom = true,
	enableRotate = true,
	enablePan = true,
	elytra = false,
	isSlim,
}: SkinViewerProps) {
	if (playerRotation === undefined)
		playerRotation = DefaultRoation;

	const canvasRef = useRef<HTMLCanvasElement>(null);
	const viewerRef = useRef<skinviewer.SkinViewer | null>(null);

	useEffect(() => {
		if (!canvasRef.current)
			return;

		const viewer = new skinviewer.SkinViewer({
			canvas: canvasRef.current,
		});

		viewer.controls.enableDamping = enableDamping;
		viewer.controls.enableZoom = enableZoom;
		viewer.controls.enableRotate = enableRotate;
		viewer.controls.enablePan = enablePan;

		viewer.zoom = zoom;

		viewer.controls.object.position.set(playerRotation.x, playerRotation.y, playerRotation.z);

		viewer.animation = animation;

		viewer.autoRotate = autoRotate;
		viewer.autoRotateSpeed = autoRotateSpeed;

		viewerRef.current = viewer;

		return () => {
			viewer.dispose();
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps -- intentional mount-only initialization
	}, []);

	useEffect(() => {
		if (!viewerRef.current)
			return;

		let model: ModelType | 'auto-detect' = 'auto-detect';
		if (isSlim !== undefined)
			model = isSlim ? 'slim' : 'default';

		viewerRef.current.loadSkin(getSkinUrl(skinUrl), { model });
	}, [skinUrl, isSlim]);

	useEffect(() => {
		if (!viewerRef.current)
			return;

		if (capeUrl)
			viewerRef.current.loadCape(capeUrl, { backEquipment: elytra ? 'elytra' : 'cape' });
		else
			viewerRef.current.resetCape();
	}, [capeUrl, elytra]);

	useEffect(() => {
		if (!viewerRef.current)
			return;

		viewerRef.current.setSize(width, height);
	}, [width, height]);

	useEffect(() => {
		if (!viewerRef.current)
			return;

		const playbackState = viewerRef.current.animation?.paused ?? false;
		viewerRef.current.animation = animation;
		viewerRef.current.animation.paused = playbackState;
	}, [animation]);

	useEffect(() => {
		if (!viewerRef.current || !viewerRef.current.animation)
			return;

		viewerRef.current.animation.paused = !animate;
	}, [animate]);

	return (
		<div className={twMerge('flex flex-col justify-center items-center', className)} style={{ minWidth: `${width}px`, minHeight: `${height}px` }}>
			<canvas
				height={height}
				ref={canvasRef}
				width={width}
			/>

			{showText ? <span className="text-fg-secondary text-xs">Hold to drag. Scroll to zoom in/out.</span> : <></>}
		</div>
	);
}
