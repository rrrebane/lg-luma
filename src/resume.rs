//! "Continue watching" and kids rows on Home, below the inputs.
//!
//! The ERR Jupiter and Lasteekraan apps (built next to this fork) tell this launcher which videos
//! are under way, on a loopback-only port (see `resume-service.rs`). The first row lists the
//! newest of them from both apps; the second is for kids: the unfinished Lasteekraan videos, then
//! the front-page recommendations that Lasteekraan itself shows, fetched here. A card opens its
//! app straight on the video, through the launch parameter `{"open": {...}}`.
//!
//! Everything that arrives is data from another process, so every field is validated and
//! bounded before it is kept or drawn.

use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

/// Loopback port the apps post to.
pub const PORT: u16 = 47615;
/// The page Lasteekraan itself builds its front page from.
pub const PICKS_URL: &str =
    "https://services.err.ee/api/v2/category/getByUrl?url=vaata-ja-kuula&domain=lasteekraan.err.ee";
const PICKS_SHELF: &str = "Lasteekraan soovitab";
const PICKS_REFRESH: i64 = 30 * 60;
const PICKS_RETRY: i64 = 2 * 60;
const IMAGE_RETRY: i64 = 5 * 60;
const MAX_ITEMS_PER_APP: usize = 30;
/// Cards per row; the rows do not scroll sideways.
pub const MAX_CARDS: usize = 5;
/// The card artwork is cut to this 16:9 size when it is downloaded.
pub const IMAGE_WIDTH: u32 = 480;
pub const IMAGE_HEIGHT: u32 = 270;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Jupiter,
    Lasteekraan,
}

impl Source {
    pub const ALL: [Self; 2] = [Self::Jupiter, Self::Lasteekraan];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.key() == name)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Jupiter => "jupiter",
            Self::Lasteekraan => "lasteekraan",
        }
    }

    /// The webOS application that plays the video.
    pub fn app_id(self) -> &'static str {
        match self {
            Self::Jupiter => "com.rebane.jupiter",
            Self::Lasteekraan => "com.rebane.lasteekraan",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Jupiter => "ERR Jupiter",
            Self::Lasteekraan => "Lasteekraan",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub source: Source,
    pub id: String,
    pub title: String,
    /// "movie", "episode" or "series".
    pub kind: String,
    pub domain: String,
    pub url: String,
    /// Artwork address; empty when there is none.
    pub image: String,
    /// Seconds watched and total, both zero for a recommendation.
    pub position: f64,
    pub duration: f64,
    /// Milliseconds since the epoch of the last time it was watched.
    pub updated: i64,
}

impl Item {
    pub fn progress(&self) -> f32 {
        if self.duration > 0. {
            (self.position / self.duration).clamp(0., 1.) as f32
        } else {
            0.
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id, "title": self.title, "type": self.kind, "domain": self.domain,
            "url": self.url, "img": self.image, "t": self.position, "d": self.duration,
            "ts": self.updated,
        })
    }
}

fn safe_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// An ERR host name (`jupiter.err.ee`, `s.err.ee`), nothing with a port or credentials.
fn err_host(host: &str) -> bool {
    host.len() <= 64
        && host.ends_with(".err.ee")
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        && !host.starts_with('.')
}

/// An https address on an ERR host, or None.
fn err_url(value: &str) -> Option<String> {
    let rest = value.strip_prefix("https://")?;
    let (host, _) = rest.split_once('/').unwrap_or((rest, ""));
    (value.len() <= 512 && err_host(host) && !value.chars().any(|c| c.is_control() || c == ' '))
        .then(|| value.to_owned())
}

/// One entry as the apps send it: `{id, title, type, domain, url, img, t, d, ts}`.
fn parse_item(source: Source, value: &Value) -> Option<Item> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    let id = text("id");
    let title = text("title");
    let kind = text("type");
    let domain = text("domain");
    if !safe_id(id)
        || !safe_text(title, 200)
        || !matches!(kind, "movie" | "episode" | "series")
        || !err_host(domain)
    {
        return None;
    }
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    };
    let position = number("t").unwrap_or(0.).max(0.);
    let duration = number("d").unwrap_or(0.).max(0.);
    Some(Item {
        source,
        id: id.into(),
        title: title.into(),
        kind: kind.into(),
        domain: domain.into(),
        url: err_url(text("url")).unwrap_or_default(),
        image: err_url(text("img")).unwrap_or_default(),
        position,
        duration,
        updated: number("ts").map_or(0, |n| n as i64),
    })
}

/// The list one app posts. Entries that fail validation are dropped; the rest is cut to the
/// newest `MAX_ITEMS_PER_APP`.
pub fn parse_items(source: Source, value: &Value) -> Vec<Item> {
    let mut items: Vec<Item> = value
        .as_array()
        .map(|list| {
            list.iter()
                .take(200)
                .filter_map(|entry| parse_item(source, entry))
                .collect()
        })
        .unwrap_or_default();
    items.sort_by(|a, b| b.updated.cmp(&a.updated));
    items.truncate(MAX_ITEMS_PER_APP);
    items
}

/// The photo of an ERR item: the smallest 16:9 rendition that is wide enough, else the widest.
fn photo(item: &Value) -> Option<String> {
    for key in ["photos", "squarePhotos", "verticalPhotos"] {
        let Some(types) = item
            .get(key)
            .and_then(|list| list.get(0))
            .and_then(|photo| photo.get("photoTypes"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let mut found: Vec<(i64, i64, &str)> = types
            .values()
            .filter_map(|t| {
                Some((
                    t.get("w")?.as_i64()?,
                    t.get("h")?.as_i64()?,
                    t.get("url")?.as_str()?,
                ))
            })
            .collect();
        found.sort_by_key(|(w, ..)| *w);
        let wide: Vec<&(i64, i64, &str)> = found.iter().filter(|(w, h, _)| w > h).collect();
        let pick = wide
            .iter()
            .copied()
            .find(|(w, ..)| *w >= IMAGE_WIDTH as i64)
            .or(wide.last().copied())
            .or(found.last())
            .map(|(.., url)| *url);
        if let Some(url) = pick.and_then(err_url) {
            return Some(url);
        }
    }
    None
}

/// The recommendations on Lasteekraan's front page (its "Lasteekraan soovitab" row).
pub fn parse_picks(value: &Value) -> Result<Vec<Item>, String> {
    let shelves = value
        .pointer("/data/category/frontPage")
        .and_then(Value::as_array)
        .ok_or("The front page has no rows")?;
    let shelf = shelves
        .iter()
        .find(|shelf| shelf.get("header").and_then(Value::as_str) == Some(PICKS_SHELF))
        .ok_or("The recommendations row is missing")?;
    let items: Vec<Item> = shelf
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let kind = entry.get("type")?.as_str()?;
                    let id = entry.get("id").map(|id| match id {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })?;
                    let title = entry.get("heading")?.as_str()?;
                    if !matches!(kind, "movie" | "episode" | "series")
                        || !safe_id(&id)
                        || !safe_text(title, 200)
                    {
                        return None;
                    }
                    Some(Item {
                        source: Source::Lasteekraan,
                        id,
                        title: title.into(),
                        kind: kind.into(),
                        domain: "lasteekraan.err.ee".into(),
                        url: String::new(),
                        image: photo(entry).unwrap_or_default(),
                        position: 0.,
                        duration: 0.,
                        updated: 0,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() {
        return Err("The recommendations row is empty".into());
    }
    Ok(items)
}

/// What the view draws for one tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// Unique among the visible cards ("watch:jupiter:123").
    pub key: String,
    pub source: Source,
    pub title: String,
    pub caption: String,
    /// The decoded artwork, once it has been downloaded.
    pub image: Option<PathBuf>,
    /// 0 to 1, or None for something not started.
    pub progress: Option<f32>,
    /// Launch parameters that make the app open this video.
    pub params: Value,
}

pub enum Command {
    Picks,
    Image { url: String, path: PathBuf },
}

#[derive(Default)]
pub struct Resume {
    items: BTreeMap<Source, Vec<Item>>,
    picks: Vec<Item>,
    picks_pending: bool,
    next_picks: i64,
    /// Image address -> when it may be requested again (after a failure).
    requested: HashMap<String, i64>,
    file: PathBuf,
    cache: PathBuf,
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

impl Resume {
    /// Reads what was kept next to the preferences; a missing or damaged file means nothing.
    pub fn new(preferences: &Path) -> Self {
        let file = preferences.with_file_name("resume.json");
        let mut items = BTreeMap::new();
        if let Some(Value::Object(saved)) = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            for source in Source::ALL {
                if let Some(list) = saved.get(source.key()) {
                    items.insert(source, parse_items(source, list));
                }
            }
        }
        Self {
            items,
            file,
            cache: preferences.with_file_name("cards"),
            ..Self::default()
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let mut saved = Map::new();
        for (source, items) in &self.items {
            saved.insert(
                source.key().into(),
                Value::Array(items.iter().map(Item::to_json).collect()),
            );
        }
        let bytes = serde_json::to_vec(&Value::Object(saved)).map_err(|e| e.to_string())?;
        // Written beside the target and renamed over it, so a power cut keeps the old file.
        let temporary = self.file.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .and_then(|()| std::fs::rename(&temporary, &self.file))
            .map_err(|e| format!("Resume list not saved: {e}"))
    }

    /// Replaces one app's list. True when something changed.
    pub fn accept_items(&mut self, source: Source, items: Vec<Item>) -> bool {
        if self.items.get(&source).is_some_and(|old| *old == items) {
            return false;
        }
        self.items.insert(source, items);
        true
    }

    pub fn accept_picks(&mut self, result: Result<Vec<Item>, String>, now: i64) {
        self.picks_pending = false;
        match result {
            Ok(picks) => {
                self.picks = picks;
                self.next_picks = now + PICKS_REFRESH;
            }
            Err(_) => self.next_picks = now + PICKS_RETRY,
        }
    }

    /// A downloaded image is picked up by the next scene; a failed one is tried again later.
    pub fn accept_image(&mut self, url: &str, ok: bool, now: i64) {
        if ok {
            self.requested.remove(url);
        } else {
            self.requested.insert(url.to_owned(), now + IMAGE_RETRY);
        }
    }

    fn image_path(&self, url: &str) -> PathBuf {
        self.cache.join(format!("{:016x}.jpg", fnv(url)))
    }

    fn card(&self, row: &str, item: &Item, caption: &str) -> Card {
        let image = (!item.image.is_empty())
            .then(|| self.image_path(&item.image))
            .filter(|path| path.is_file());
        Card {
            key: format!("{row}:{}:{}", item.source.key(), item.id),
            source: item.source,
            title: item.title.clone(),
            caption: caption.into(),
            image,
            progress: (item.duration > 0.).then(|| item.progress()),
            params: json!({"open": {
                "id": item.id, "title": item.title, "type": item.kind,
                "domain": item.domain, "url": item.url, "img": item.image,
            }}),
        }
    }

    fn resumed(&self) -> Vec<&Item> {
        let mut all: Vec<&Item> = self.items.values().flatten().collect();
        all.sort_by(|a, b| b.updated.cmp(&a.updated));
        all
    }

    /// The newest unfinished videos of every app.
    pub fn watch_cards(&self) -> Vec<Card> {
        self.resumed()
            .into_iter()
            .take(MAX_CARDS)
            .map(|item| self.card("watch", item, item.source.title()))
            .collect()
    }

    /// Unfinished Lasteekraan videos first, then its recommendations.
    pub fn kids_cards(&self) -> Vec<Card> {
        let started: Vec<&Item> = self
            .resumed()
            .into_iter()
            .filter(|item| item.source == Source::Lasteekraan)
            .collect();
        let mut cards: Vec<Card> = started
            .iter()
            .map(|item| self.card("kids", item, Source::Lasteekraan.title()))
            .collect();
        for pick in &self.picks {
            if !started.iter().any(|item| item.id == pick.id) {
                cards.push(self.card("kids", pick, Source::Lasteekraan.title()));
            }
        }
        cards.truncate(MAX_CARDS);
        cards
    }

    /// The card a control key stands for, as it is now.
    pub fn find(&self, key: &str) -> Option<Card> {
        self.watch_cards()
            .into_iter()
            .chain(self.kids_cards())
            .find(|card| card.key == key)
    }

    /// The downloads that are due. Nothing is fetched while Home is not showing.
    pub fn sync(&mut self, home_visible: bool, now: i64) -> Vec<Command> {
        let mut commands = Vec::new();
        if !home_visible {
            return commands;
        }
        if !self.picks_pending && now >= self.next_picks {
            self.picks_pending = true;
            commands.push(Command::Picks);
        }
        let wanted: Vec<String> = self
            .resumed()
            .into_iter()
            .take(MAX_CARDS)
            .chain(self.kids_items())
            .map(|item| item.image.clone())
            .filter(|url| !url.is_empty())
            .collect();
        for url in wanted {
            let path = self.image_path(&url);
            if path.is_file() || self.requested.get(&url).is_some_and(|until| now < *until) {
                continue;
            }
            // Far in the future until the download reports back, so it is not asked for twice.
            self.requested.insert(url.clone(), i64::MAX);
            commands.push(Command::Image { url, path });
        }
        commands
    }

    fn kids_items(&self) -> Vec<&Item> {
        let mut items: Vec<&Item> = self
            .resumed()
            .into_iter()
            .filter(|item| item.source == Source::Lasteekraan)
            .collect();
        items.extend(self.picks.iter());
        items.truncate(MAX_CARDS);
        items
    }
}

#[cfg(test)]
#[path = "resume-tests.rs"]
mod tests;
