use serde::{Deserialize, Serialize};
use specta::Type;

use reqwest::Method;

use crate::error::LauncherResult;
use crate::utils::http;

#[onelauncher_macro::specta]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PolyPlusActiveCape {
	pub active: Option<u32>,
}

#[onelauncher_macro::specta]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PolyPlusActive {
	pub cape: Option<PolyPlusActiveCape>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub enum PolyPlusCosmeticType {
	#[serde(rename = "cape")]
	Cape,
	#[serde(rename = "emote")]
	Emote,
}

#[onelauncher_macro::specta]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PolyPlusCosmetic {
	#[serde(rename = "type")]
	pub kind: PolyPlusCosmeticType,
	pub hash: String,
	pub id: u32,
	pub url: Option<String>,
}

#[onelauncher_macro::specta]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PolyPlusPlayer {
	pub active: PolyPlusActive,
	pub cosmetics: Vec<PolyPlusCosmetic>,
}

pub async fn get_player_cosmetics(uuid: &str) -> LauncherResult<PolyPlusPlayer> {
	http::fetch_json_advanced::<PolyPlusPlayer>(
		Method::GET,
		&format!(
			"{}/cosmetics/player?player={}",
			crate::constants::POLY_PLUS_BASE_API_URL,
			uuid
		),
		None,
		None,
		None,
		None,
	)
	.await
}
