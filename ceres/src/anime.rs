use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;

pub const ANIME_RSS: &str = "https://raw.githubusercontent.com/ArjixGamer/gogoanime-rss/main/gogoanime/gogoanime-rss-sub.xml";

#[derive(Debug, Serialize, Deserialize)]
pub struct Follows {
    //#[serde(borrow = "'a")]
    // key is the md5 of the slugified original Japanese name
    pub following: HashMap<String, AniInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Updates {
    pub updates: HashMap<String, AniMinInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq)]
pub struct AniInfo {
    pub info: AniMinInfo,
    pub extra: AniExtraInfo,
}

impl Ord for AniInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.extra.cmp(&other.extra)
    }
}

impl PartialOrd for AniInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AniInfo {
    fn eq(&self, other: &Self) -> bool {
        self.extra == other.extra && self.info == other.info
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq)]
pub struct AniMinInfo {
    pub name: String,
    pub last_episode: i16,
}

impl Ord for AniMinInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for AniMinInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AniMinInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.last_episode == other.last_episode
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq)]
pub struct AniExtraInfo {
    pub en_name: String,
    pub season: AnimeSeason,
}

impl Default for AniExtraInfo {
    fn default() -> Self {
        AniExtraInfo {
            en_name: String::new(),
            season: AnimeSeason::Unknown,
        }
    }
}

impl Ord for AniExtraInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.en_name.cmp(&other.en_name)
    }
}

impl PartialOrd for AniExtraInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AniExtraInfo {
    fn eq(&self, other: &Self) -> bool {
        self.en_name == other.en_name
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum AnimeSeason {
    Winter(u16),
    Spring(u16),
    Summer(u16),
    Autumn(u16),
    Unknown,
}

impl Display for AnimeSeason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnimeSeason::Winter(y) => write!(f, "Winter {y}"),
            AnimeSeason::Spring(y) => write!(f, "Spring {y}"),
            AnimeSeason::Summer(y) => write!(f, "Summer {y}"),
            AnimeSeason::Autumn(y) => write!(f, "Autumn {y}"),
            AnimeSeason::Unknown => write!(f, "Unknown"),
        }
    }
}
